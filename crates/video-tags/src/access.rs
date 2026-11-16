//! The access PIN every FunSync site on the home network asks for (the Rust
//! side; the backend has its own copy of the same contract).
//!
//! `<userData>/access-pin.json` (mode 0600, written by `npm run access:pin`):
//! `{"version":1,"pin":"<4-12 digits>","secret":"<64 hex>","changedAt":"<ISO>"}`.
//! No file: no PIN, every site is open. A device that logged in holds the
//! token `v1.<expiry>.<hex HMAC-SHA256(secret, "v1.<expiry>")>` (30 days) in
//! the `funsync_access` cookie, or passes it as the `k` query parameter.
//! Changing the PIN makes a new secret, which logs every device out.
//! Loopback clients (this Mac) never need the PIN.

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fmt;
use std::fs;
use std::io;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The PIN file in userData.
pub const FILE_NAME: &str = "access-pin.json";
pub const VERSION: u32 = 1;
/// The cookie holding the token.
pub const COOKIE_NAME: &str = "funsync_access";
/// The query parameter holding the token, for media fetches that can't send
/// cookies (headsets).
pub const QUERY_KEY: &str = "k";
/// How long a login lasts: 30 days.
pub const TOKEN_LIFETIME_SECS: u64 = 30 * 24 * 60 * 60;
pub const PIN_MIN_LEN: usize = 4;
pub const PIN_MAX_LEN: usize = 12;
/// The secret's length in bytes (64 hex digits in the file).
pub const SECRET_LEN: usize = 32;
/// The cookie that logs a device out.
pub const CLEAR_COOKIE: &str = "funsync_access=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax";

type HmacSha256 = Hmac<Sha256>;

/// The PIN file's contents.
#[derive(Clone, PartialEq, Eq)]
pub struct AccessPin {
    pub pin: String,
    secret: [u8; SECRET_LEN],
    pub changed_at: String,
}

/// Neither the PIN nor the secret.
impl fmt::Debug for AccessPin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessPin")
            .field("changed_at", &self.changed_at)
            .finish_non_exhaustive()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileJson {
    version: u32,
    pin: String,
    secret: String,
    changed_at: String,
}

/// `<userData>/access-pin.json`.
pub fn file_path(user_data: &Path) -> PathBuf {
    user_data.join(FILE_NAME)
}

/// 4 to 12 ASCII digits.
pub fn valid_pin(pin: &str) -> bool {
    (PIN_MIN_LEN..=PIN_MAX_LEN).contains(&pin.len()) && pin.bytes().all(|b| b.is_ascii_digit())
}

impl AccessPin {
    pub fn new(pin: &str, secret: [u8; SECRET_LEN], changed_at: &str) -> Result<Self, String> {
        if !valid_pin(pin) {
            return Err(format!(
                "the PIN must be {PIN_MIN_LEN} to {PIN_MAX_LEN} digits"
            ));
        }
        Ok(AccessPin {
            pin: pin.to_string(),
            secret,
            changed_at: changed_at.to_string(),
        })
    }

    /// The PIN file in `user_data`: `Ok(None)` when there is none (no PIN),
    /// `Err` when it can't be read or isn't a valid PIN file (callers deny
    /// every non-loopback request then).
    pub fn load(user_data: &Path) -> Result<Option<Self>, String> {
        let path = file_path(user_data);
        match fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text)
                .map(Some)
                .map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let file: FileJson = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if file.version != VERSION {
            return Err(format!("unknown version {}", file.version));
        }
        let secret =
            unhex::<SECRET_LEN>(&file.secret).ok_or("secret must be 64 lowercase hex digits")?;
        Self::new(&file.pin, secret, &file.changed_at)
    }

    /// The file's contents (pretty JSON, trailing newline).
    pub fn to_json(&self) -> String {
        let file = FileJson {
            version: VERSION,
            pin: self.pin.clone(),
            secret: hex(&self.secret),
            changed_at: self.changed_at.clone(),
        };
        let mut text = serde_json::to_string_pretty(&file).expect("plain strings serialize");
        text.push('\n');
        text
    }

    /// Whether `pin` is the PIN, in constant time for PINs of its length.
    pub fn check_pin(&self, pin: &str) -> bool {
        ct_eq(self.pin.as_bytes(), pin.as_bytes())
    }

    /// A token for a login at `now` (Unix seconds), valid for
    /// TOKEN_LIFETIME_SECS.
    pub fn token(&self, now: u64) -> String {
        let payload = format!("v1.{}", now.saturating_add(TOKEN_LIFETIME_SECS));
        let sig = self.mac(&payload).finalize().into_bytes();
        format!("{payload}.{}", hex(&sig))
    }

    /// Whether `token` was made with this secret and expires after `now`.
    pub fn verify(&self, token: &str, now: u64) -> bool {
        let Some((payload, sig)) = token.rsplit_once('.') else {
            return false;
        };
        let Some(expiry) = payload.strip_prefix("v1.") else {
            return false;
        };
        if expiry.is_empty() || !expiry.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        if !expiry.parse::<u64>().is_ok_and(|e| e > now) {
            return false;
        }
        let Some(sig) = unhex::<32>(sig) else {
            return false;
        };
        self.mac(payload).verify_slice(&sig).is_ok()
    }

    fn mac(&self, payload: &str) -> HmacSha256 {
        let mut mac = HmacSha256::new_from_slice(&self.secret).expect("HMAC takes any key");
        mac.update(payload.as_bytes());
        mac
    }
}

/// `Set-Cookie` value logging a device in with `token` for 30 days.
pub fn set_cookie(token: &str) -> String {
    format!("{COOKIE_NAME}={token}; Path=/; Max-Age={TOKEN_LIFETIME_SECS}; HttpOnly; SameSite=Lax")
}

/// The values of cookie `name` in a `Cookie` header (`a=1; b=2`).
pub fn cookie_values<'a>(header: &'a str, name: &'a str) -> impl Iterator<Item = &'a str> {
    header.split(';').filter_map(move |pair| {
        let (k, v) = pair.split_once('=')?;
        (k.trim() == name).then(|| v.trim().trim_matches('"'))
    })
}

/// The values of `key` in a query string (`a=1&k=…`), undecoded (tokens are
/// plain ASCII).
pub fn query_values<'a>(query: &'a str, key: &'a str) -> impl Iterator<Item = &'a str> {
    query.split('&').filter_map(move |pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then_some(v)
    })
}

/// Whether a client at `ip` is this machine: 127.0.0.0/8, ::1 and their
/// IPv4-mapped forms. Loopback clients never need the PIN.
pub fn is_loopback(ip: IpAddr) -> bool {
    ip.to_canonical().is_loopback()
}

/// Unix seconds now.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Equal bytes, without stopping at the first difference.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[usize::from(b >> 4)] as char);
        s.push(DIGITS[usize::from(b & 0xf)] as char);
    }
    s
}

/// `N` bytes from exactly `2 * N` lowercase hex digits.
fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let s = s.as_bytes();
    if s.len() != 2 * N {
        return None;
    }
    let mut out = [0u8; N];
    for (byte, pair) in out.iter_mut().zip(s.chunks_exact(2)) {
        *byte = digit(pair[0])? << 4 | digit(pair[1])?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    const SECRET_HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    fn pin(secret: [u8; SECRET_LEN]) -> AccessPin {
        AccessPin::new("1234", secret, "2026-11-16T10:00:00.000Z").unwrap()
    }

    fn secret(first: u8) -> [u8; SECRET_LEN] {
        std::array::from_fn(|i| first.wrapping_add(i as u8))
    }

    #[test]
    fn tokens_match_the_contract() {
        // Python: hmac.new(bytes(range(32)), b"v1.1800000000", "sha256").hexdigest()
        let p = pin(secret(0));
        let now = 1_800_000_000 - TOKEN_LIFETIME_SECS;
        assert_eq!(
            p.token(now),
            "v1.1800000000.866fbcd4be953182d7f832e844bf390a5f79d112cccd131af311acbb7431e77a"
        );
    }

    #[test]
    fn token_round_trip_and_expiry() {
        let p = pin(secret(7));
        let now = 1_763_290_000;
        let t = p.token(now);
        assert!(p.verify(&t, now));
        assert!(p.verify(&t, now + TOKEN_LIFETIME_SECS - 1));
        assert!(!p.verify(&t, now + TOKEN_LIFETIME_SECS));
        assert!(!p.verify(&t, now + TOKEN_LIFETIME_SECS + 1));
    }

    #[test]
    fn tampered_tokens_fail() {
        let p = pin(secret(7));
        let now = 1_763_290_000;
        let t = p.token(now);
        let (payload, sig) = t.rsplit_once('.').unwrap();
        // A later expiry with the old signature.
        let later = format!("v1.{}.{sig}", now + 10 * TOKEN_LIFETIME_SECS);
        assert!(!p.verify(&later, now));
        // One signature digit changed.
        let mut bad = sig.as_bytes().to_vec();
        bad[0] = if bad[0] == b'0' { b'1' } else { b'0' };
        let bad = format!("{payload}.{}", String::from_utf8(bad).unwrap());
        assert!(!p.verify(&bad, now));
        for t in [
            "",
            "v1",
            "v1.",
            &format!("v2.{}", &t[3..]),
            &format!("{payload}.{}", sig.to_uppercase()),
            &format!("{payload}.{}", &sig[2..]),
            &format!("{t}0"),
            &format!("v1.+{}", &t[3..]),
            &format!("v1.{}", &t[3..].replacen('.', "..", 1)),
        ] {
            assert!(!p.verify(t, now), "{t}");
        }
    }

    #[test]
    fn a_new_secret_logs_everyone_out() {
        let now = 1_763_290_000;
        let t = pin(secret(7)).token(now);
        assert!(!pin(secret(8)).verify(&t, now));
        // Same secret, another PIN: still logged in (the CLI always makes a new secret).
        let same = AccessPin::new("98765", secret(7), "x").unwrap();
        assert!(same.verify(&t, now));
    }

    #[test]
    fn checks_pins() {
        assert!(valid_pin("1234") && valid_pin("123456789012"));
        for bad in [
            "123",
            "1234567890123",
            "12a4",
            "",
            " 1234",
            "１２３４",
            "-123",
        ] {
            assert!(!valid_pin(bad), "{bad}");
            assert!(AccessPin::new(bad, secret(0), "x").is_err());
        }
        let p = pin(secret(0));
        assert!(p.check_pin("1234"));
        assert!(!p.check_pin("1235") && !p.check_pin("12345") && !p.check_pin(""));
    }

    #[test]
    fn parses_and_writes_the_file() {
        let text = format!(
            r#"{{"version":1,"pin":"0042","secret":"{SECRET_HEX}","changedAt":"2026-11-16T10:00:00.000Z"}}"#
        );
        let p = AccessPin::parse(&text).unwrap();
        assert_eq!(
            p,
            AccessPin::new("0042", secret(0), "2026-11-16T10:00:00.000Z").unwrap()
        );
        assert_eq!(AccessPin::parse(&p.to_json()).unwrap(), p);
        let v: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
        assert_eq!(v["secret"], SECRET_HEX);
        assert_eq!(v["changedAt"], "2026-11-16T10:00:00.000Z");
        assert!(!format!("{p:?}").contains("0042"));

        for bad in [
            "",
            "{}",
            "[]",
            &text.replace(r#""version":1"#, r#""version":2"#),
            &text.replace("0042", "42"),
            &text.replace(SECRET_HEX, &SECRET_HEX.to_uppercase()),
            &text.replace(SECRET_HEX, &SECRET_HEX[2..]),
            &text.replace(r#","changedAt":"2026-11-16T10:00:00.000Z""#, ""),
            &text.replace(r#""pin""#, r#""extra":1,"pin""#),
        ] {
            assert!(AccessPin::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn loads_none_without_a_file_and_fails_on_a_bad_one() {
        let dir = std::env::temp_dir().join(format!("video-tags-access-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(AccessPin::load(&dir), Ok(None));
        let p = pin(secret(3));
        fs::write(file_path(&dir), p.to_json()).unwrap();
        assert_eq!(AccessPin::load(&dir), Ok(Some(p)));
        fs::write(file_path(&dir), "{").unwrap();
        assert!(AccessPin::load(&dir).unwrap_err().contains(FILE_NAME));
        fs::remove_file(file_path(&dir)).unwrap();
        fs::create_dir(file_path(&dir)).unwrap();
        assert!(AccessPin::load(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_loopback_clients() {
        for ip in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2)),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            IpAddr::V6(Ipv4Addr::LOCALHOST.to_ipv6_mapped()),
        ] {
            assert!(is_loopback(ip), "{ip}");
        }
        for ip in [
            IpAddr::V4(Ipv4Addr::new(192, 168, 2, 220)),
            IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            IpAddr::V6(Ipv4Addr::new(192, 168, 2, 30).to_ipv6_mapped()),
            // IPv4-compatible (deprecated), not mapped: not loopback.
            IpAddr::V6(Ipv4Addr::LOCALHOST.to_ipv6_compatible()),
            "fe80::1".parse().unwrap(),
        ] {
            assert!(!is_loopback(ip), "{ip}");
        }
    }

    #[test]
    fn reads_cookies_and_query_tokens() {
        let h =
            r#"theme=dark; funsync_access=v1.1.ab ;x_funsync_access=no; funsync_access="v1.2.cd""#;
        assert_eq!(
            cookie_values(h, COOKIE_NAME).collect::<Vec<_>>(),
            ["v1.1.ab", "v1.2.cd"]
        );
        assert_eq!(cookie_values("", COOKIE_NAME).count(), 0);
        assert_eq!(
            query_values("limit=3&k=v1.1.ab&kk=no&k", QUERY_KEY).collect::<Vec<_>>(),
            ["v1.1.ab"]
        );
        assert_eq!(
            set_cookie("v1.1.ab"),
            "funsync_access=v1.1.ab; Path=/; Max-Age=2592000; HttpOnly; SameSite=Lax"
        );
    }
}
