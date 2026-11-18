//! Instagram: the session (`--login`: a visible Chrome signs in with the
//! credentials file, or the user does; its cookies go to
//! `instagram-session.json`, 0600; credentials and cookies are never
//! printed) and the pictures step (`--pictures`): with a session, the
//! newest timeline posts from `web_profile_info`; without, the profile
//! photo from the public page.

use crate::state::{State, Swipes};
use crate::store::{self, CreatorsFile, Instagram, Picture};
use crate::verify;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use slr_scout::http::{Http, Pacer, Request, Response};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const SESSION_FILE: &str = "instagram-session.json";
pub const PROFILE_DIR: &str = "instagram-chrome";
pub const LOGIN_URL: &str = "https://www.instagram.com/accounts/login/";
pub const APP_ID: &str = "936619743392459";
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
pub const CREDENTIALS_ENV: &str = "FUNSYNC_INSTAGRAM_CREDENTIALS";
pub const CREDENTIALS_FILE: &str = "instagram_creds.txt";
/// Timeline posts looked at, pictures kept.
pub const TIMELINE_POSTS: usize = 12;
pub const KEEP_PICTURES: usize = 9;
pub const MAX_WIDTH: u64 = 1080;
/// A creator's pictures are fetched at most this often.
pub const REFRESH_DAYS: u64 = 3;
const COOKIES: &[&str] = &["csrftoken", "sessionid", "ds_user_id"];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub cookies: Vec<Value>,
    #[serde(default)]
    pub saved_at: Option<String>,
}

fn instagram_cookie(c: &Value) -> bool {
    c["domain"]
        .as_str()
        .is_some_and(|d| d.trim_start_matches('.').ends_with("instagram.com"))
}

impl Session {
    /// `csrftoken=…; sessionid=…; ds_user_id=…`; `None` without a
    /// `sessionid`.
    pub fn cookie_header(&self) -> Option<String> {
        let value = |name: &str| {
            self.cookies
                .iter()
                .filter(|c| instagram_cookie(c) && c["name"] == name)
                .find_map(|c| c["value"].as_str().filter(|v| !v.is_empty()))
        };
        value("sessionid")?;
        Some(
            COOKIES
                .iter()
                .filter_map(|n| value(n).map(|v| format!("{n}={v}")))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }
}

/// The Instagram cookies of a `Network.getAllCookies` result, once a
/// `sessionid` is among them.
pub fn session_from_cookies(result: &Value, now_iso: &str) -> Option<Session> {
    let cookies: Vec<Value> = result["cookies"]
        .as_array()?
        .iter()
        .filter(|c| instagram_cookie(c))
        .cloned()
        .collect();
    let s = Session {
        cookies,
        saved_at: Some(now_iso.to_string()),
    };
    s.cookie_header().is_some().then_some(s)
}

pub fn read_session(user_data: &Path) -> Result<Option<Session>, String> {
    let path = user_data.join(SESSION_FILE);
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map(Some)
            .map_err(|_| format!("{} is not a session file; run --login", path.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Write the session readable by its owner only (temp + rename).
pub fn write_session(user_data: &Path, s: &Session) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let target = user_data.join(SESSION_FILE);
    let tmp = user_data.join(format!("{SESSION_FILE}.{}.tmp", std::process::id()));
    let result = (|| -> io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(
            serde_json::to_string_pretty(s)
                .map_err(io::Error::other)?
                .as_bytes(),
        )?;
        f.sync_all()?;
        fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|e| format!("writing {}: {e}", target.display()))
}

fn js(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// Fill Instagram's login form (username, password, submit). `Ok(false)`
/// when the page shows no form (already signed in in this profile, or a
/// layout this doesn't know: the user signs in by hand).
fn fill_form(
    cdp: &mut slr_scout::cdp::Cdp<std::net::TcpStream>,
    s: &str,
    creds: &slr_scout::login::Credentials,
) -> Result<bool, String> {
    // The EU cookie dialog covers the form until it is answered.
    let _ = cdp.eval(
        s,
        "(()=>{const b=[...document.querySelectorAll('button')].find(x=>/^(Decline optional cookies|Only allow essential cookies|Allow all cookies)$/i.test(x.innerText.trim())); if(b) b.click(); return !!b})()",
    )?;
    std::thread::sleep(Duration::from_millis(800));
    for (sel, value) in [
        // Instagram renamed the fields (2026-11: name="email" / "pass").
        (
            r#"input[name="username"], input[name="email"], input[autocomplete^="username"]"#,
            &creds.username,
        ),
        (r#"input[type="password"]"#, &creds.password),
    ] {
        let found = cdp.eval(
            s,
            &format!(
                "(()=>{{const i=document.querySelector({}); if(!i) return false; i.focus(); i.select?.(); return true}})()",
                js(sel)
            ),
        )?;
        if found != true {
            return Ok(false);
        }
        cdp.call("Input.insertText", json!({ "text": value }), Some(s))?;
        std::thread::sleep(Duration::from_millis(400));
    }
    std::thread::sleep(Duration::from_millis(600));
    let clicked = cdp.eval(
        s,
        "(()=>{const b=document.querySelector('button[type=\"submit\"], input[type=\"submit\"]') || [...document.querySelectorAll('button,[role=button]')].find(x=>x.innerText.trim()==='Log in'); if(!b) return false; b.click(); return true})()",
    )?;
    Ok(clicked == true)
}

/// `--login`: open Instagram's login page in a visible Chrome (its own
/// profile in userData), sign in with `creds` when given, and wait until
/// a session appears (the user finishes any check Instagram asks for).
pub fn login(
    user_data: &Path,
    creds: Option<&slr_scout::login::Credentials>,
    log: &mut dyn Write,
) -> Result<Session, String> {
    let mut b = slr_scout::login::open_chrome(&user_data.join(PROFILE_DIR))?;
    let s = b.session.clone();
    b.cdp
        .call("Page.navigate", json!({ "url": LOGIN_URL }), Some(&s))?;
    std::thread::sleep(Duration::from_secs(5));
    let typed = match creds {
        Some(c) => fill_form(&mut b.cdp, &s, c)?,
        None => false,
    };
    let _ = writeln!(
        log,
        "{}; waiting up to {} minutes...",
        if typed {
            "Signed in with the credentials file: finish any check Instagram asks for (a code, \"save login info\") in the Chrome window"
        } else {
            "Sign in to Instagram in the Chrome window"
        },
        LOGIN_TIMEOUT.as_secs() / 60
    );
    let started = Instant::now();
    let found = loop {
        std::thread::sleep(Duration::from_secs(2));
        let cookies = b.cdp.call("Network.getAllCookies", json!({}), Some(&s))?;
        if session_from_cookies(&cookies, "").is_some() {
            // The rest of the cookies (csrftoken, ds_user_id) come with the
            // same redirect; give them a moment.
            std::thread::sleep(Duration::from_secs(3));
            let cookies = b.cdp.call("Network.getAllCookies", json!({}), Some(&s))?;
            break session_from_cookies(&cookies, &video_tags::timefmt::now_iso());
        }
        if started.elapsed() > LOGIN_TIMEOUT {
            break None;
        }
    };
    let _ = b.cdp.call("Browser.close", json!({}), None);
    found.ok_or_else(|| "no Instagram session after 10 minutes; run --login again".to_string())
}

/// A timeline post.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: String,
    pub shortcode: String,
    /// The biggest rendition at most 1080 px wide.
    pub image: String,
    pub taken_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub followers: u64,
    pub posts: u64,
    pub full_name: String,
    pub bio: String,
    /// Newest first, at most 12.
    pub items: Vec<Item>,
}

fn image_of(node: &Value) -> Option<String> {
    let best = node["display_resources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| Some((r["config_width"].as_u64()?, r["src"].as_str()?)))
        .filter(|(w, _)| *w <= MAX_WIDTH)
        .max_by_key(|(w, _)| *w)
        .map(|(_, s)| s.to_string());
    best.or_else(|| node["display_url"].as_str().map(str::to_string))
        .or_else(|| node["thumbnail_src"].as_str().map(str::to_string))
        .filter(|u| u.starts_with("https://"))
}

/// `web_profile_info`'s answer; `None` when the user doesn't exist.
pub fn parse_profile(v: &Value) -> Option<Profile> {
    let u = &v["data"]["user"];
    if !u.is_object() {
        return None;
    }
    let media = &u["edge_owner_to_timeline_media"];
    let mut items: Vec<Item> = media["edges"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let n = &e["node"];
            Some(Item {
                id: match &n["id"] {
                    Value::String(s) => s.clone(),
                    Value::Number(x) => x.to_string(),
                    _ => return None,
                },
                shortcode: n["shortcode"].as_str()?.to_string(),
                image: image_of(n)?,
                taken_at: n["taken_at_timestamp"].as_i64()?,
            })
        })
        .collect();
    items.sort_by(|a, b| b.taken_at.cmp(&a.taken_at));
    items.truncate(TIMELINE_POSTS);
    Some(Profile {
        followers: u["edge_followed_by"]["count"].as_u64().unwrap_or(0),
        posts: media["count"].as_u64().unwrap_or(0),
        full_name: u["full_name"].as_str().unwrap_or_default().to_string(),
        bio: u["biography"].as_str().unwrap_or_default().to_string(),
        items,
    })
}

/// What an API answer means.
#[derive(Debug, PartialEq)]
pub enum Answer {
    Profile(Box<Profile>),
    Missing,
    NotLoggedIn,
    /// 429, a challenge or checkpoint: stop for today.
    Limited(String),
    Failed(String),
}

pub fn classify(res: &Response) -> Answer {
    let body = res.body.as_str();
    let challenge = body.contains("checkpoint_required") || body.contains("challenge_required");
    let location = res.header_values("location").next().unwrap_or("");
    match res.status {
        429 => Answer::Limited("HTTP 429".into()),
        300..=399 if location.contains("/challenge") => Answer::Limited("challenge".into()),
        300..=399 if location.contains("/accounts/login") => Answer::NotLoggedIn,
        401 | 403 if challenge => Answer::Limited("challenge".into()),
        401 | 403 => Answer::NotLoggedIn,
        404 => Answer::Missing,
        200..=299 => {
            let Ok(v) = serde_json::from_str::<Value>(body) else {
                // The login page instead of JSON.
                return Answer::NotLoggedIn;
            };
            if v["require_login"] == true || v["message"] == "login_required" {
                return Answer::NotLoggedIn;
            }
            if challenge {
                return Answer::Limited("challenge".into());
            }
            match parse_profile(&v) {
                Some(p) => Answer::Profile(Box::new(p)),
                None if v["data"].is_object() => Answer::Missing,
                None => Answer::Failed("unexpected answer".into()),
            }
        }
        s => Answer::Failed(format!("HTTP {s}")),
    }
}

fn profile_request(base: &str, handle: &str, cookie: &str) -> Request {
    Request::get(format!(
        "{base}/api/v1/users/web_profile_info/?username={handle}"
    ))
    .header("x-ig-app-id", APP_ID)
    .header("X-Requested-With", "XMLHttpRequest")
    .header("Accept", "*/*")
    .header("Referer", format!("{base}/{handle}/"))
    .header("Cookie", cookie)
}

fn iso_of(secs: i64) -> String {
    video_tags::timefmt::iso_utc(UNIX_EPOCH + Duration::from_secs(secs.max(0) as u64))
}

/// Seconds since the epoch of an ISO time (`2026-11-17T21:00:00[.mmm]Z`).
pub fn parse_iso(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Creators whose pictures are due (an Instagram handle, not swiped no,
/// none fetched for 3 days), stalest first.
pub fn due(file: &CreatorsFile, state: &State, swipes: &Swipes, now: i64) -> Vec<String> {
    let mut list: Vec<(Option<i64>, String)> = file
        .creators
        .iter()
        .filter(|(slug, c)| c.instagram.is_some() && !swipes.creator_is_no(slug))
        .map(|(slug, _)| {
            (
                state.pictures.get(slug).and_then(|t| parse_iso(t)),
                slug.clone(),
            )
        })
        .filter(|(at, _)| at.is_none_or(|t| now - t >= (REFRESH_DAYS * 86_400) as i64))
        .collect();
    list.sort();
    list.into_iter().map(|(_, s)| s).collect()
}

/// 8–15 s between creators.
fn pause() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    std::thread::sleep(Duration::from_millis(8_000 + u64::from(nanos) % 7_001));
}

/// Download `url` to `dir/name` unless it is there (through a `.part` file).
pub fn save(http: &dyn Http, url: &str, dir: &Path, name: &str) -> Result<bool, String> {
    let dest = dir.join(name);
    if dest.is_file() {
        return Ok(false);
    }
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = dir.join(format!(".{name}.part"));
    let _ = fs::remove_file(&part);
    let r = http
        .download(&Request::get(url), &part)
        .and_then(|_| fs::rename(&part, &dest).map_err(|e| e.to_string()));
    if r.is_err() {
        let _ = fs::remove_file(&part);
    }
    r.map(|_| true)
}

pub struct PicturesRun {
    pub code: i32,
    pub done: usize,
    pub saved: usize,
    pub left: usize,
}

/// `--pictures N`. Writes creators.json and the state after each creator.
#[allow(clippy::too_many_arguments)]
pub fn pictures(
    http: &dyn Http,
    base: &str,
    user_data: &Path,
    file: &mut CreatorsFile,
    state: &mut State,
    swipes: &Swipes,
    limit: usize,
    fake: bool,
    out: &mut dyn Write,
) -> Result<PicturesRun, String> {
    let session = read_session(user_data)?;
    let cookie = session.as_ref().and_then(Session::cookie_header);
    let due = due(file, state, swipes, now_secs());
    let mut run = PicturesRun {
        code: 0,
        done: 0,
        saved: 0,
        left: due.len().saturating_sub(limit),
    };
    let mut page_pacer = Pacer::new(if fake {
        Duration::ZERO
    } else {
        verify::INSTAGRAM_GAP
    });
    if cookie.is_none() {
        let _ = writeln!(
            out,
            "No Instagram session: profile photos only (run --login for the posts)."
        );
    }
    for (i, slug) in due.iter().take(limit).enumerate() {
        if i > 0 && !fake {
            pause();
        }
        let Some(c) = file.creators.get_mut(slug) else {
            continue;
        };
        let Some(ig) = c.instagram.clone() else {
            continue;
        };
        let now = video_tags::timefmt::now_iso();
        let dir = store::pictures_dir(user_data, slug);
        match &cookie {
            Some(cookie) => {
                let req = profile_request(base, &ig.handle, cookie);
                let res = http.send(&req).map_err(|e| format!("Instagram: {e}"))?;
                match classify(&res) {
                    Answer::Profile(p) => {
                        let mut ig = ig;
                        ig.followers = Some(verify::compact(p.followers));
                        ig.posts = Some(p.posts.to_string());
                        if !p.full_name.is_empty() {
                            ig.full_name = Some(p.full_name.clone());
                        }
                        ig.bio = Some(p.bio.clone()).filter(|b| !b.is_empty());
                        ig.last_post_at = p.items.first().map(|x| iso_of(x.taken_at));
                        ig.checked_at = Some(now.clone());
                        let mut pics = Vec::new();
                        for item in p.items.iter().take(KEEP_PICTURES) {
                            let Some(name) = store::instagram_picture(&item.id) else {
                                continue;
                            };
                            match save(http, &item.image, &dir, &name) {
                                Ok(new) => run.saved += usize::from(new),
                                Err(e) => {
                                    let _ = writeln!(out, "  {slug}: {name}: {e}");
                                    continue;
                                }
                            }
                            pics.push(Picture {
                                file: name,
                                source: "instagram".into(),
                                url: format!("{}/p/{}/", store::INSTAGRAM_SITE, item.shortcode),
                                taken_at: Some(iso_of(item.taken_at)),
                            });
                        }
                        set_instagram_pictures(c, ig, pics);
                        let _ = writeln!(
                            out,
                            "  {} (@{}): {} picture(s)",
                            c.name,
                            c.instagram.as_ref().map_or("", |i| i.handle.as_str()),
                            c.pictures
                                .iter()
                                .filter(|p| p.source == "instagram")
                                .count()
                        );
                    }
                    Answer::Missing => {
                        let _ = writeln!(out, "  {}: @{} not found", c.name, ig.handle);
                        let mut ig = ig;
                        ig.checked_at = Some(now.clone());
                        c.instagram = Some(ig);
                    }
                    Answer::NotLoggedIn => {
                        let _ = writeln!(
                            out,
                            "Instagram asks to log in: the session expired; run --login."
                        );
                        run.code = 3;
                        break;
                    }
                    Answer::Limited(why) => {
                        let _ = writeln!(out, "Instagram: {why}; stopping for today.");
                        run.code = 2;
                        break;
                    }
                    Answer::Failed(why) => {
                        let _ = writeln!(out, "  {}: @{}: {why}", c.name, ig.handle);
                        run.code = run.code.max(2);
                        continue;
                    }
                }
            }
            None => match verify::instagram(http, &mut page_pacer, base, &ig.handle) {
                Ok(Some(page)) => {
                    let mut ig = ig;
                    ig.followers = Some(page.followers.clone());
                    ig.posts = Some(page.posts.clone());
                    if !page.full_name.is_empty() {
                        ig.full_name = Some(page.full_name.clone());
                    }
                    if page.bio.is_some() {
                        ig.bio = page.bio.clone();
                    }
                    ig.checked_at = Some(now.clone());
                    let mut pics = Vec::new();
                    if let Some(url) = &page.picture {
                        // The photo may have changed: fetch it again.
                        let _ = fs::remove_file(dir.join(store::INSTAGRAM_PROFILE_PICTURE));
                        match save(http, url, &dir, store::INSTAGRAM_PROFILE_PICTURE) {
                            Ok(_) => {
                                run.saved += 1;
                                pics.push(Picture {
                                    file: store::INSTAGRAM_PROFILE_PICTURE.into(),
                                    source: "instagram".into(),
                                    url: format!("{}/{}/", store::INSTAGRAM_SITE, ig.handle),
                                    taken_at: None,
                                });
                            }
                            Err(e) => {
                                let _ = writeln!(out, "  {slug}: profile photo: {e}");
                            }
                        }
                    }
                    // Keep earlier timeline pictures; the photo replaces only itself.
                    let mut keep: Vec<Picture> = c
                        .pictures
                        .iter()
                        .filter(|p| {
                            p.source == "instagram" && p.file != store::INSTAGRAM_PROFILE_PICTURE
                        })
                        .cloned()
                        .collect();
                    keep.extend(pics);
                    set_instagram_pictures(c, ig, keep);
                    let _ = writeln!(out, "  {}: profile photo", c.name);
                }
                Ok(None) => {
                    let _ = writeln!(out, "  {}: @{} not found", c.name, ig.handle);
                }
                Err(slr_scout::http::NetError::RateLimited(m)) => {
                    let _ = writeln!(out, "Instagram: {m}; stopping for today.");
                    run.code = 2;
                    break;
                }
                Err(e) => {
                    let _ = writeln!(out, "  {}: {e}", c.name);
                    run.code = run.code.max(2);
                    continue;
                }
            },
        }
        let removed = store::prune(user_data, slug, &file.creators[slug]);
        if removed > 0 {
            let _ = writeln!(out, "  {slug}: {removed} old picture(s) removed");
        }
        state.pictures.insert(slug.clone(), now);
        run.done += 1;
        store::write(user_data, file)?;
        crate::state::write(user_data, state)?;
    }
    if run.code == 0 && cookie.is_none() {
        run.code = 3;
    }
    if run.code == 0 && run.left > 0 {
        run.code = 2;
    }
    Ok(run)
}

fn set_instagram_pictures(c: &mut store::Creator, ig: Instagram, pics: Vec<Picture>) {
    c.instagram = Some(ig);
    c.pictures.retain(|p| p.source != "instagram");
    c.pictures.extend(pics);
    store::rebuild_pictures(c);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_cookies() {
        let result = json!({"cookies": [
            {"name": "sessionid", "value": "S", "domain": ".instagram.com"},
            {"name": "csrftoken", "value": "C", "domain": ".instagram.com"},
            {"name": "ds_user_id", "value": "D", "domain": ".instagram.com"},
            {"name": "sessionid", "value": "X", "domain": ".example.com"},
            {"name": "mid", "value": "M", "domain": ".instagram.com"}
        ]});
        let s = session_from_cookies(&result, "NOW").unwrap();
        assert_eq!(s.cookies.len(), 4);
        assert_eq!(s.saved_at.as_deref(), Some("NOW"));
        assert_eq!(
            s.cookie_header().as_deref(),
            Some("csrftoken=C; sessionid=S; ds_user_id=D")
        );
        let no_session =
            json!({"cookies": [{"name": "csrftoken", "value": "C", "domain": ".instagram.com"}]});
        assert!(session_from_cookies(&no_session, "N").is_none());
    }

    fn profile_json() -> Value {
        let node = |id: u64, t: i64| {
            json!({"node": {"id": id.to_string(), "shortcode": format!("S{id}"), "taken_at_timestamp": t,
                "display_url": format!("https://cdn.ig/{id}_1440.jpg"),
                "display_resources": [
                    {"src": format!("https://cdn.ig/{id}_640.jpg"), "config_width": 640},
                    {"src": format!("https://cdn.ig/{id}_1080.jpg"), "config_width": 1080},
                    {"src": format!("https://cdn.ig/{id}_1440.jpg"), "config_width": 1440}]}})
        };
        let edges: Vec<Value> = (1..=14)
            .map(|i| node(i, 1_700_000_000 + i as i64))
            .collect();
        json!({"data": {"user": {"full_name": "Sweetie Fox", "biography": "hi",
            "edge_followed_by": {"count": 5_000_000},
            "edge_owner_to_timeline_media": {"count": 225, "edges": edges}}}})
    }

    #[test]
    fn parses_the_profile_api() {
        let p = parse_profile(&profile_json()).unwrap();
        assert_eq!((p.followers, p.posts), (5_000_000, 225));
        assert_eq!(p.items.len(), TIMELINE_POSTS);
        assert_eq!(p.items[0].id, "14");
        assert_eq!(p.items[0].image, "https://cdn.ig/14_1080.jpg");
        assert_eq!(parse_profile(&json!({"data": {"user": null}})), None);
    }

    fn res(status: u16, body: &str) -> Response {
        Response {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    #[test]
    fn classifies_answers() {
        assert!(matches!(
            classify(&res(200, &profile_json().to_string())),
            Answer::Profile(_)
        ));
        assert_eq!(
            classify(&res(200, r#"{"data":{"user":null}}"#)),
            Answer::Missing
        );
        assert_eq!(
            classify(&res(
                200,
                r#"{"message":"login_required","require_login":true}"#
            )),
            Answer::NotLoggedIn
        );
        assert_eq!(
            classify(&res(200, "<html>login</html>")),
            Answer::NotLoggedIn
        );
        assert_eq!(classify(&res(401, "{}")), Answer::NotLoggedIn);
        assert!(matches!(classify(&res(429, "")), Answer::Limited(_)));
        assert!(matches!(
            classify(&res(400, r#"{"message":"checkpoint_required"}"#)),
            Answer::Failed(_)
        ));
        assert!(matches!(
            classify(&res(403, r#"{"message":"challenge_required"}"#)),
            Answer::Limited(_)
        ));
        let redirect = Response {
            status: 302,
            headers: vec![(
                "location".into(),
                "https://www.instagram.com/accounts/login/".into(),
            )],
            body: String::new(),
        };
        assert_eq!(classify(&redirect), Answer::NotLoggedIn);
    }

    #[test]
    fn due_creators_stalest_first() {
        let mut file = CreatorsFile::empty();
        for s in ["a", "b", "c", "d", "e"] {
            file.creators.insert(
                s.into(),
                store::Creator {
                    name: s.into(),
                    instagram: (s != "e").then(|| Instagram {
                        handle: s.into(),
                        ..Instagram::default()
                    }),
                    ..store::Creator::default()
                },
            );
        }
        let mut state = State::default();
        let now = parse_iso("2026-11-17T00:00:00Z").unwrap();
        state
            .pictures
            .insert("a".into(), "2026-11-16T00:00:00.000Z".into());
        state
            .pictures
            .insert("b".into(), "2026-11-10T00:00:00.000Z".into());
        let mut swipes = Swipes::default();
        swipes.creators.insert("d".into(), "no".into());
        assert_eq!(due(&file, &state, &swipes, now), ["c", "b"]);
        assert_eq!(parse_iso("1970-01-02T00:00:01Z"), Some(86_401));
        assert_eq!(iso_of(1_793_793_600), "2026-11-04T12:00:00.000Z");
    }
}
