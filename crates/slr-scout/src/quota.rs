//! The account's download quota. Needs a session: `<userData>/slr-session.json`
//! holds the `refresh_jwt` cookie (saved by `--login`, mode 0600); it buys
//! an access token, which reads the user overview's download limits. A
//! rotated `refresh_jwt` in the token response is saved back. Tokens are
//! never printed.

use crate::http::{Http, Request, Response};
use crate::slr::{API, SITE};
use crate::store::{Limit, Quota, ScoutFile};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use video_tags::timefmt;

pub const SESSION_FILE: &str = "slr-session.json";
pub const COOKIE_NAME: &str = "refresh_jwt";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub refresh_jwt: String,
    /// When the cookie expires (ISO), when known.
    #[serde(default)]
    pub expires: Option<String>,
    #[serde(default)]
    pub saved_at: Option<String>,
}

pub fn session_path(user_data: &Path) -> PathBuf {
    user_data.join(SESSION_FILE)
}

/// The saved session; `Ok(None)` when there is none.
pub fn read_session(user_data: &Path) -> Result<Option<Session>, String> {
    let path = session_path(user_data);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let s: Session = serde_json::from_str(&raw)
        .map_err(|_| format!("{} is not a session file", path.display()))?;
    Ok((!s.refresh_jwt.is_empty()).then_some(s))
}

/// Save the session atomically, readable by this user only (0600).
pub fn write_session(user_data: &Path, s: &Session) -> Result<(), String> {
    let path = session_path(user_data);
    let tmp = user_data.join(format!("{SESSION_FILE}.{}.tmp", std::process::id()));
    let result = (|| -> io::Result<()> {
        let json = serde_json::to_string_pretty(s).map_err(io::Error::other)?;
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|e| format!("writing {}: {e}", path.display()))
}

/// Why the quota wasn't read.
#[derive(Debug, Clone, PartialEq)]
pub enum QuotaError {
    NoSession,
    Expired,
    RateLimited(String),
    Failed(String),
}

impl QuotaError {
    /// The file's `quotaError`.
    pub fn message(&self) -> String {
        match self {
            QuotaError::NoSession => "no session".into(),
            QuotaError::Expired => "session expired".into(),
            QuotaError::RateLimited(m) => format!("rate limited: {m}"),
            QuotaError::Failed(m) => m.clone(),
        }
    }

    /// Not logged in (exit code 3).
    pub fn needs_login(&self) -> bool {
        matches!(self, QuotaError::NoSession | QuotaError::Expired)
    }
}

/// A cookie set by a response: its value and expiry (epoch seconds).
#[derive(Debug, Clone, PartialEq)]
pub struct SetCookie {
    pub value: String,
    pub expires: Option<i64>,
}

/// The cookie `name` from one `Set-Cookie` header; `None` for another
/// cookie or a deletion (empty value, `Max-Age=0`). `Max-Age` wins over
/// `Expires`.
pub fn parse_set_cookie(header: &str, name: &str, now: i64) -> Option<SetCookie> {
    let mut parts = header.split(';');
    let (n, value) = parts.next()?.split_once('=')?;
    if n.trim() != name {
        return None;
    }
    let value = value.trim().trim_matches('"').to_string();
    let (mut max_age, mut expires) = (None, None);
    for attr in parts {
        let Some((k, v)) = attr.split_once('=') else {
            continue;
        };
        match k.trim().to_ascii_lowercase().as_str() {
            "max-age" => max_age = v.trim().parse::<i64>().ok(),
            "expires" => expires = parse_http_date(v.trim()),
            _ => {}
        }
    }
    if value.is_empty() || max_age.is_some_and(|a| a <= 0) {
        return None;
    }
    Some(SetCookie {
        value,
        expires: max_age.map(|a| now + a).or(expires),
    })
}

/// `Sun, 13 Dec 2026 10:20:30 GMT` (or `13-Dec-2026`) → epoch seconds.
pub fn parse_http_date(s: &str) -> Option<i64> {
    let s = s.split_once(',').map_or(s, |(_, rest)| rest).trim();
    let mut it = s.split([' ', '-']).filter(|p| !p.is_empty());
    let day: i64 = it.next()?.parse().ok()?;
    let month = it.next()?.to_ascii_lowercase();
    let month = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|m| month.starts_with(m))? as i64
        + 1;
    let mut year: i64 = it.next()?.parse().ok()?;
    if year < 100 {
        year += 2000;
    }
    let mut hms = it.next()?.split(':').map(|p| p.parse::<i64>().ok());
    let (h, m, sec) = (hms.next()??, hms.next()??, hms.next()??);
    Some(days_from_civil(year, month, day) * 86_400 + h * 3600 + m * 60 + sec)
}

/// (year, month, day) → days since 1970-01-01 (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn epoch_iso(secs: i64) -> String {
    timefmt::iso_utc(UNIX_EPOCH + Duration::from_secs(u64::try_from(secs).unwrap_or(0)))
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// An access token for the session, and the rotated session when the
/// response set a new `refresh_jwt`.
pub fn access_token(
    http: &dyn Http,
    session: &Session,
) -> Result<(String, Option<Session>), QuotaError> {
    let req = Request::post(format!("{SITE}/api/get-access-token"), "{}")
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("Cookie", format!("{COOKIE_NAME}={}", session.refresh_jwt));
    let res = http
        .send(&req)
        .map_err(|e| QuotaError::Failed(format!("get-access-token: {e}")))?;
    check(&res, "get-access-token")?;
    let token = serde_json::from_str::<Value>(&res.body)
        .ok()
        .and_then(|v| v["accessToken"].as_str().map(str::to_string))
        .filter(|t| !t.is_empty())
        .ok_or(QuotaError::Expired)?;
    let now = now_epoch();
    let rotated = res
        .header_values("set-cookie")
        .filter_map(|h| parse_set_cookie(h, COOKIE_NAME, now))
        .last()
        .filter(|c| c.value != session.refresh_jwt)
        .map(|c| Session {
            refresh_jwt: c.value,
            expires: c.expires.map(epoch_iso).or_else(|| session.expires.clone()),
            saved_at: Some(epoch_iso(now)),
        });
    Ok((token, rotated))
}

fn check(res: &Response, what: &str) -> Result<(), QuotaError> {
    match res.status {
        200..=299 => Ok(()),
        401 => Err(QuotaError::Expired),
        429 => Err(QuotaError::RateLimited(format!("{what}: HTTP 429"))),
        s => Err(QuotaError::Failed(format!("{what}: HTTP {s}"))),
    }
}

fn limit(v: &Value, key: &str) -> Result<Limit, String> {
    let l = &v[key];
    let num = |f: &str| {
        l[f].as_i64()
            .or_else(|| l[f].as_f64().map(|x| x as i64))
            .or_else(|| l[f].as_str().and_then(|s| s.parse().ok()))
    };
    Ok(Limit {
        remaining: num("remaining").ok_or_else(|| format!("overview: no {key}.remaining"))?,
        total: num("total").ok_or_else(|| format!("overview: no {key}.total"))?,
        usage_expires_at: num("usageExpiresAt"),
    })
}

/// The download limits of a user overview response.
pub fn parse_overview(v: &Value, checked_at: &str) -> Result<Quota, String> {
    let limits = &v["data"]["limits"];
    Ok(Quota {
        checked_at: checked_at.to_string(),
        scripts: limit(limits, "downloadScript")?,
        ai_scripts: limit(limits, "downloadAiScript")?,
        videos: limit(limits, "downloadVideo")?,
    })
}

/// An access token for the saved session, saving a rotated cookie.
pub fn session_token(http: &dyn Http, user_data: &Path) -> Result<String, QuotaError> {
    let session = read_session(user_data)
        .map_err(QuotaError::Failed)?
        .ok_or(QuotaError::NoSession)?;
    let (token, rotated) = access_token(http, &session)?;
    if let Some(next) = rotated {
        write_session(user_data, &next).map_err(QuotaError::Failed)?;
    }
    Ok(token)
}

/// A GET to the API with the access token.
pub fn authed(url: String, token: &str) -> Request {
    Request::get(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Client-Type", "web")
        .header("project", "1")
        .header("Accept", "application/json")
}

/// The download limits, with an access token.
pub fn overview(http: &dyn Http, token: &str, now_iso: &str) -> Result<Quota, QuotaError> {
    let req = authed(format!("{API}/users/v1/user/overview"), token);
    let res = http
        .send(&req)
        .map_err(|e| QuotaError::Failed(format!("user overview: {e}")))?;
    check(&res, "user overview")?;
    let v: Value = serde_json::from_str(&res.body)
        .map_err(|_| QuotaError::Failed("user overview: not JSON".into()))?;
    parse_overview(&v, now_iso).map_err(QuotaError::Failed)
}

/// Read the quota with the saved session, saving a rotated cookie.
pub fn fetch(http: &dyn Http, user_data: &Path, now_iso: &str) -> Result<Quota, QuotaError> {
    let token = session_token(http, user_data)?;
    overview(http, &token, now_iso)
}

/// An authorised API call's JSON: 401 means the session expired.
pub fn authed_json(http: &dyn Http, req: &Request, what: &str) -> Result<Value, QuotaError> {
    let res = http
        .send(req)
        .map_err(|e| QuotaError::Failed(format!("{what}: {e}")))?;
    check(&res, what)?;
    serde_json::from_str(&res.body).map_err(|_| QuotaError::Failed(format!("{what}: not JSON")))
}

/// The file's `quota` and `quotaError`: a fresh quota, or the previous
/// file's quota with the error.
pub fn merge(
    previous: Option<&ScoutFile>,
    result: &Result<Quota, QuotaError>,
) -> (Option<Quota>, Option<String>) {
    match result {
        Ok(q) => (Some(q.clone()), None),
        Err(e) => (previous.and_then(|p| p.quota.clone()), Some(e.message())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::testing::Canned;
    use serde_json::json;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("slr-scout-quota-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn overview() -> String {
        json!({"data": {"limits": {
            "downloadScript": {"remaining": 2, "total": 3, "usageExpiresAt": 1795000000, "period": "month"},
            "downloadAiScript": {"remaining": 3, "total": 3, "usageExpiresAt": null},
            "downloadVideo": {"remaining": 35, "total": 35}
        }}})
        .to_string()
    }

    fn quota() -> Quota {
        parse_overview(&serde_json::from_str(&overview()).unwrap(), "T").unwrap()
    }

    #[test]
    fn parses_the_overview_limits() {
        let q = quota();
        assert_eq!(
            q.scripts,
            Limit {
                remaining: 2,
                total: 3,
                usage_expires_at: Some(1795000000)
            }
        );
        assert_eq!(q.ai_scripts.usage_expires_at, None);
        assert_eq!(q.videos.total, 35);
        assert_eq!(q.checked_at, "T");
        assert!(parse_overview(&json!({"data": {"limits": {}}}), "T").is_err());
    }

    #[test]
    fn parses_set_cookie_headers() {
        let c = parse_set_cookie(
            "refresh_jwt=abc; Path=/; Domain=.sexlikereal.com; Max-Age=2592000; HttpOnly",
            COOKIE_NAME,
            1000,
        )
        .unwrap();
        assert_eq!(c.value, "abc");
        assert_eq!(c.expires, Some(1000 + 2_592_000));
        let c = parse_set_cookie(
            "refresh_jwt=xyz; expires=Sun, 13 Dec 2026 00:00:00 GMT; path=/",
            COOKIE_NAME,
            0,
        )
        .unwrap();
        assert_eq!(epoch_iso(c.expires.unwrap()), "2026-12-13T00:00:00.000Z");
        assert_eq!(parse_set_cookie("other=1", COOKIE_NAME, 0), None);
        assert_eq!(
            parse_set_cookie("refresh_jwt=; Max-Age=0", COOKIE_NAME, 0),
            None
        );
        assert_eq!(
            parse_set_cookie("refresh_jwt=a; Max-Age=0", COOKIE_NAME, 0),
            None
        );
        assert_eq!(parse_http_date("Thu, 01-Jan-1970 00:00:10 GMT"), Some(10));
        assert_eq!(parse_http_date("nonsense"), None);
    }

    #[test]
    fn keeps_the_old_quota_on_failure() {
        let prev = ScoutFile {
            generated_at: "G".into(),
            actors: vec![],
            quota: Some(quota()),
            quota_error: None,
            scenes: vec![],
        };
        assert_eq!(
            merge(Some(&prev), &Err(QuotaError::Expired)),
            (Some(quota()), Some("session expired".into()))
        );
        assert_eq!(
            merge(None, &Err(QuotaError::NoSession)),
            (None, Some("no session".into()))
        );
        let mut fresh = quota();
        fresh.checked_at = "NEW".into();
        assert_eq!(merge(Some(&prev), &Ok(fresh.clone())), (Some(fresh), None));
        assert!(QuotaError::Expired.needs_login() && QuotaError::NoSession.needs_login());
        assert!(!QuotaError::Failed("x".into()).needs_login());
    }

    #[test]
    fn fetches_with_the_session_and_saves_a_rotated_cookie() {
        let dir = temp("fetch");
        assert_eq!(
            fetch(&Canned::default(), &dir, "T"),
            Err(QuotaError::NoSession)
        );

        let old = Session {
            refresh_jwt: "old".into(),
            expires: Some("2026-12-13T00:00:00.000Z".into()),
            saved_at: None,
        };
        write_session(&dir, &old).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(session_path(&dir))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let mut http = Canned::default();
        http.on("POST", &format!("{SITE}/api/get-access-token"), 401, "{}");
        assert_eq!(fetch(&http, &dir, "T"), Err(QuotaError::Expired));

        http.with_headers(
            "POST",
            &format!("{SITE}/api/get-access-token"),
            200,
            &[("set-cookie", "refresh_jwt=new; Max-Age=86400; Path=/")],
            r#"{"accessToken": "tok"}"#,
        );
        http.on(
            "GET",
            &format!("{API}/users/v1/user/overview"),
            200,
            &overview(),
        );
        let q = fetch(&http, &dir, "T").unwrap();
        assert_eq!(q, quota());
        let seen = http.seen.borrow();
        let post = seen.iter().rfind(|r| r.method == "POST").unwrap();
        assert!(
            post.headers
                .contains(&("Cookie".into(), "refresh_jwt=old".into()))
        );
        let get = seen.last().unwrap();
        assert!(
            get.headers
                .contains(&("Authorization".into(), "Bearer tok".into()))
        );
        drop(seen);
        let saved = read_session(&dir).unwrap().unwrap();
        assert_eq!(saved.refresh_jwt, "new");
        assert!(saved.saved_at.is_some());

        http.on("GET", &format!("{API}/users/v1/user/overview"), 429, "");
        assert!(matches!(
            fetch(&http, &dir, "T"),
            Err(QuotaError::RateLimited(_))
        ));
        let _ = fs::remove_dir_all(&dir);
    }
}
