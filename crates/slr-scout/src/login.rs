//! `--login`: sign in to SLR in a visible Chrome window (password login
//! needs reCAPTCHA v3, which a headless Chrome fails) driven over the
//! DevTools protocol, and save the `refresh_jwt` cookie to
//! `slr-session.json`. Credentials and tokens are never printed.

use crate::cdp::{self, Cdp};
use crate::quota::{COOKIE_NAME, Session, epoch_iso};
use crate::slr::SITE;
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub const CREDENTIALS_ENV: &str = "FUNSYNC_SLR_CREDENTIALS";
pub const CHROME_ENV: &str = "FUNSYNC_CHROME_PATH";
pub const CHROME_PATH: &str = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
/// Chrome's profile for the login, in userData.
pub const PROFILE_DIR: &str = "slr-chrome";

#[derive(Clone, PartialEq)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

/// `username: x` and `password: y` lines.
pub fn parse_credentials(raw: &str) -> Result<Credentials, String> {
    let (mut username, mut password) = (None, None);
    for line in raw.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = v.trim().to_string();
        match k.trim().to_ascii_lowercase().as_str() {
            "username" | "user" | "login" | "email" => username = Some(v),
            "password" => password = Some(v),
            _ => {}
        }
    }
    match (username, password) {
        (Some(u), Some(p)) if !u.is_empty() && !p.is_empty() => Ok(Credentials {
            username: u,
            password: p,
        }),
        _ => Err("needs a \"username: …\" and a \"password: …\" line".into()),
    }
}

/// `--credentials`, else `$FUNSYNC_SLR_CREDENTIALS`, else `slr_creds.txt` at
/// the repo root.
pub fn credentials_path(flag: Option<&Path>, env: Option<String>) -> PathBuf {
    if let Some(p) = flag {
        return p.to_path_buf();
    }
    if let Some(p) = env.filter(|v| !v.trim().is_empty()) {
        return PathBuf::from(p);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../slr_creds.txt")
}

pub fn read_credentials(path: &Path) -> Result<Credentials, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_credentials(&raw).map_err(|e| format!("{}: {e}", path.display()))
}

/// The session in a `Network.getAllCookies` result: SLR's `refresh_jwt`.
pub fn session_from_cookies(result: &Value, now_iso: &str) -> Option<Session> {
    result["cookies"]
        .as_array()?
        .iter()
        .find(|c| {
            c["name"] == COOKIE_NAME
                && c["domain"]
                    .as_str()
                    .is_some_and(|d| d.trim_start_matches('.').ends_with("sexlikereal.com"))
                && c["value"].as_str().is_some_and(|v| !v.is_empty())
        })
        .map(|c| Session {
            refresh_jwt: c["value"].as_str().unwrap_or_default().to_string(),
            expires: c["expires"]
                .as_f64()
                .filter(|&e| e > 0.0)
                .map(|e| epoch_iso(e as i64)),
            saved_at: Some(now_iso.to_string()),
        })
}

/// `DevToolsActivePort` (port line, browser path line) → WebSocket URL.
pub fn devtools_url(port_file: &str) -> Option<String> {
    let mut lines = port_file.lines();
    let port = lines.next()?.trim();
    let path = lines.next()?.trim();
    (!port.is_empty() && path.starts_with('/')).then(|| format!("ws://127.0.0.1:{port}{path}"))
}

/// Kills Chrome when dropped.
struct Chrome(Child);

impl Drop for Chrome {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn js(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// Sign in with a visible Chrome and return the new session.
pub fn login(
    user_data: &Path,
    creds: &Credentials,
    now_iso: &str,
    log: &mut dyn Write,
) -> Result<Session, String> {
    let profile = user_data.join(PROFILE_DIR);
    fs::create_dir_all(&profile).map_err(|e| format!("{}: {e}", profile.display()))?;
    let port_file = profile.join("DevToolsActivePort");
    let _ = fs::remove_file(&port_file);
    let chrome_path = std::env::var(CHROME_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| CHROME_PATH.to_string());
    let _ = writeln!(log, "Opening Chrome to sign in to SLR...");
    let _chrome = Chrome(
        Command::new(&chrome_path)
            .arg(format!("--user-data-dir={}", profile.display()))
            .args([
                "--remote-debugging-port=0",
                "--no-first-run",
                "--no-default-browser-check",
                "--window-size=1366,900",
                "about:blank",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("can't start Chrome ({chrome_path}): {e}"))?,
    );
    let started = Instant::now();
    let url = loop {
        if let Some(url) = fs::read_to_string(&port_file)
            .ok()
            .and_then(|s| devtools_url(&s))
        {
            break url;
        }
        if started.elapsed() > Duration::from_secs(30) {
            return Err("Chrome didn't open its DevTools port".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut cdp = Cdp::new(cdp::connect(&url, Duration::from_secs(30))?);
    let target = cdp.call("Target.createTarget", json!({ "url": "about:blank" }), None)?;
    let target_id = target["targetId"]
        .as_str()
        .ok_or("no target id")?
        .to_string();
    let attached = cdp.call(
        "Target.attachToTarget",
        json!({ "targetId": target_id, "flatten": true }),
        None,
    )?;
    let session = attached["sessionId"]
        .as_str()
        .ok_or("no session id")?
        .to_string();
    let s = session.as_str();
    let cookies = |cdp: &mut Cdp<_>| cdp.call("Network.getAllCookies", json!({}), Some(s));

    cdp.call(
        "Page.navigate",
        json!({ "url": format!("{SITE}/sign-in") }),
        Some(s),
    )?;
    std::thread::sleep(Duration::from_secs(6));
    let gate = cdp.eval(
        s,
        "(()=>{const b=[...document.querySelectorAll('button')].find(x=>/I am over 18/i.test(x.innerText)); if(!b) return false; b.click(); return true})()",
    )?;
    if gate == true {
        std::thread::sleep(Duration::from_millis(1500));
    }
    let before = session_from_cookies(&cookies(&mut cdp)?, now_iso);
    let user_sel = r#"input[autocomplete="username"]"#;
    let has_form = cdp.eval(s, &format!("!!document.querySelector({})", js(user_sel)))?;
    if has_form != true {
        // Already signed in in this profile: the page has no form.
        return before.ok_or_else(|| "the sign-in form didn't show up".to_string());
    }
    for (sel, value) in [
        (user_sel, &creds.username),
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
            return Err(format!("sign-in form: no {sel}"));
        }
        cdp.call("Input.insertText", json!({ "text": value }), Some(s))?;
        std::thread::sleep(Duration::from_millis(300));
    }
    let clicked = cdp.eval(
        s,
        "(()=>{const b=[...document.querySelectorAll('button')].find(x=>x.innerText.trim()==='Sign In'); if(!b) return false; b.click(); return true})()",
    )?;
    if clicked != true {
        return Err("sign-in form: no \"Sign In\" button".into());
    }
    let _ = writeln!(log, "Signing in...");
    let waited = Instant::now();
    while waited.elapsed() < Duration::from_secs(15) {
        std::thread::sleep(Duration::from_secs(1));
        if let Some(new) = session_from_cookies(&cookies(&mut cdp)?, now_iso)
            && before
                .as_ref()
                .is_none_or(|b| b.refresh_jwt != new.refresh_jwt)
        {
            return Ok(new);
        }
    }
    Err(
        "no SLR session cookie after signing in (wrong credentials, or a captcha in the window)"
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_credentials() {
        let c = parse_credentials("username: lexa_fan\npassword:  p@ss: word \n").unwrap();
        assert_eq!(c.username, "lexa_fan");
        assert_eq!(c.password, "p@ss: word");
        assert!(parse_credentials("username: a\n").is_err());
        assert!(parse_credentials("password: b\nusername:\n").is_err());
    }

    #[test]
    fn credentials_flag_wins_over_env_over_repo_file() {
        assert_eq!(
            credentials_path(Some(Path::new("/f")), Some("/e".into())),
            PathBuf::from("/f")
        );
        assert_eq!(
            credentials_path(None, Some("/e".into())),
            PathBuf::from("/e")
        );
        assert!(credentials_path(None, Some(" ".into())).ends_with("slr_creds.txt"));
        assert!(credentials_path(None, None).ends_with("../../slr_creds.txt"));
    }

    #[test]
    fn finds_the_session_cookie() {
        let result = json!({"cookies": [
            {"name": "refresh_jwt", "value": "elsewhere", "domain": "example.com", "expires": 1.0},
            {"name": "other", "value": "x", "domain": ".sexlikereal.com"},
            {"name": "refresh_jwt", "value": "jwt", "domain": ".sexlikereal.com", "expires": 1797120000.5}
        ]});
        let s = session_from_cookies(&result, "NOW").unwrap();
        assert_eq!(s.refresh_jwt, "jwt");
        assert_eq!(s.expires.as_deref(), Some("2026-12-13T00:00:00.000Z"));
        assert_eq!(s.saved_at.as_deref(), Some("NOW"));
        let session_cookie = json!({"cookies": [
            {"name": "refresh_jwt", "value": "jwt", "domain": "www.sexlikereal.com", "expires": -1}
        ]});
        assert_eq!(
            session_from_cookies(&session_cookie, "N").unwrap().expires,
            None
        );
        assert!(session_from_cookies(&json!({"cookies": []}), "N").is_none());
        assert!(session_from_cookies(&json!({}), "N").is_none());
    }

    #[test]
    fn reads_the_devtools_port_file() {
        assert_eq!(
            devtools_url("53211\n/devtools/browser/0a1b\n").as_deref(),
            Some("ws://127.0.0.1:53211/devtools/browser/0a1b")
        );
        assert_eq!(devtools_url("53211\n"), None);
        assert_eq!(devtools_url(""), None);
    }
}
