//! Member pages through the installed Google Chrome (port of
//! scripts/vrbangers-browser.mjs): the sites sign download links per member
//! inside the scene page, so `--fetch` signs in and reads each scene page's
//! `#__NUXT_DATA__` right before its download. Chrome runs headless
//! (`--headful` shows it) with its own profile, `<userData>/vrb-scout-chrome`,
//! so the session survives runs, and is driven over the DevTools protocol
//! (`slr_scout::cdp`). The account comes from `vb_hack.txt` (`Username: …` /
//! `password: …`); the password, cookies and link tokens are never printed.
//!
//! With the canned network (tests) [`Canned`] reads the same pages through
//! the fake HTTP client instead.

use crate::nuxt;
use crate::sites::Site;
use serde_json::{Value, json};
use slr_scout::cdp::{self, Cdp};
use slr_scout::http::{Http, Pacer, Request};
use slr_scout::login::{self, Credentials};
use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Chrome's profile, in userData. Not the EroScripts scout's (`vrbangers-chrome`): both
/// download from VRBangers in their own hourly jobs, and one Chrome profile can't be open
/// twice (since 2026-11-23); this one signs in on its first use.
pub const PROFILE_DIR: &str = "vrb-scout-chrome";
pub const CREDENTIALS_ENV: &str = "FUNSYNC_VRB_CREDENTIALS";
/// Where a kept session lands after `/login/`.
const DASHBOARD_PATH: &str = "/main-dashboard/";
const LOAD_TIMEOUT: Duration = Duration::from_secs(60);
const FORM_TIMEOUT: Duration = Duration::from_secs(15);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(30);
/// Between page loads.
pub const PAGE_GAP: Duration = Duration::from_secs(1);
/// Before loading a scene page again that came without links.
const RELOAD_WAIT: Duration = Duration::from_secs(3);

/// Why a sign-in didn't work.
#[derive(Debug, Clone, PartialEq)]
pub enum LoginError {
    /// No credentials, or the site refused them.
    Refused(String),
    /// Chrome or the page broke.
    Failed(String),
}

impl LoginError {
    pub fn message(&self) -> &str {
        match self {
            LoginError::Refused(m) | LoginError::Failed(m) => m,
        }
    }
}

/// A member's view of the sites.
pub trait Members {
    /// Sign in to `site` (a kept session counts).
    fn login(&mut self, site: &Site) -> Result<(), LoginError>;
    /// The scene page's `#__NUXT_DATA__` array; None when the page has none.
    fn nuxt_data(&mut self, site: &Site, slug: &str) -> Result<Option<Value>, String>;
}

/// `--credentials`, else `$FUNSYNC_VRB_CREDENTIALS`, else `vb_hack.txt` at
/// the repo root.
pub fn credentials_path(flag: Option<&Path>, env: Option<String>) -> PathBuf {
    if let Some(p) = flag {
        return p.to_path_buf();
    }
    if let Some(p) = env.filter(|v| !v.trim().is_empty()) {
        return PathBuf::from(p);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vb_hack.txt")
}

pub fn read_credentials(path: &Path) -> Result<Credentials, LoginError> {
    let raw = fs::read_to_string(path).map_err(|e| {
        LoginError::Refused(format!(
            "no VRBangers credentials ({}: {e})",
            path.display()
        ))
    })?;
    login::parse_credentials(&raw)
        .map_err(|e| LoginError::Refused(format!("{}: {e}", path.display())))
}

fn js(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// The page script that says whether the browser is signed in (on the
/// dashboard, or `/login/` shows no form).
fn signed_in_check() -> String {
    format!(
        "location.pathname.startsWith({}) || !document.querySelector('input[name=\"mail\"]')",
        js(DASHBOARD_PATH)
    )
}

/// Kills Chrome when dropped.
struct Process(Child);

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A headless (or visible) Chrome on one page.
pub struct Chrome {
    cdp: Cdp<TcpStream>,
    session: String,
    pacer: Pacer,
    credentials: PathBuf,
    // Last: Chrome is killed after the socket closes.
    _process: Process,
}

impl Chrome {
    pub fn open(user_data: &Path, credentials: PathBuf, headful: bool) -> Result<Chrome, String> {
        let profile = user_data.join(PROFILE_DIR);
        fs::create_dir_all(&profile).map_err(|e| format!("{}: {e}", profile.display()))?;
        let port_file = profile.join("DevToolsActivePort");
        let _ = fs::remove_file(&port_file);
        let mut cmd = Command::new(login::CHROME_PATH);
        cmd.arg(format!("--user-data-dir={}", profile.display()))
            .args([
                "--remote-debugging-port=0",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-background-networking",
                "--disable-sync",
                "--mute-audio",
                "--window-size=1366,900",
            ]);
        if !headful {
            cmd.arg("--headless=new");
        }
        let mut child = cmd
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("can't start Chrome ({}): {e}", login::CHROME_PATH))?;
        let started = Instant::now();
        let url = loop {
            if let Some(url) = fs::read_to_string(&port_file)
                .ok()
                .and_then(|s| login::devtools_url(&s))
            {
                break url;
            }
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!(
                    "Chrome exited ({status}); is another run using {}?",
                    profile.display()
                ));
            }
            if started.elapsed() > Duration::from_secs(30) {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Chrome didn't open its DevTools port".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let process = Process(child);
        let mut cdp = Cdp::new(cdp::connect(&url, LOAD_TIMEOUT)?);
        let version = cdp.call("Browser.getVersion", json!({}), None)?;
        let user_agent = version["userAgent"]
            .as_str()
            .unwrap_or_default()
            .replace("HeadlessChrome", "Chrome");
        let target = cdp.call("Target.createTarget", json!({ "url": "about:blank" }), None)?;
        let target_id = target["targetId"].as_str().ok_or("no target id")?;
        let attached = cdp.call(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
            None,
        )?;
        let session = attached["sessionId"]
            .as_str()
            .ok_or("no session id")?
            .to_string();
        if !user_agent.is_empty() {
            cdp.call(
                "Network.setUserAgentOverride",
                json!({ "userAgent": user_agent }),
                Some(&session),
            )?;
        }
        Ok(Chrome {
            cdp,
            session,
            pacer: Pacer::new(PAGE_GAP),
            credentials,
            _process: process,
        })
    }

    fn eval(&mut self, expression: &str) -> Result<Value, String> {
        self.cdp.eval(&self.session, expression)
    }

    /// Poll `expression` until it is true or `timeout` passes.
    fn wait_for(&mut self, expression: &str, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if self.eval(expression).is_ok_and(|v| v == true) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        false
    }

    /// Open `url` and wait for its document to finish loading: the old
    /// document gets a marker first, so its `complete` doesn't count.
    fn navigate(&mut self, url: &str) -> Result<(), String> {
        self.pacer.wait();
        let _ = self.eval("window.__vrbScoutOld = true");
        let r = self.cdp.call(
            "Page.navigate",
            json!({ "url": url }),
            Some(&self.session.clone()),
        )?;
        if let Some(e) = r["errorText"].as_str().filter(|e| !e.is_empty()) {
            return Err(format!("can't open {url}: {e}"));
        }
        if self.wait_for(
            "!window.__vrbScoutOld && document.readyState === 'complete'",
            LOAD_TIMEOUT,
        ) {
            Ok(())
        } else {
            Err(format!("{url} didn't finish loading"))
        }
    }
}

impl Members for Chrome {
    fn login(&mut self, site: &Site) -> Result<(), LoginError> {
        let failed = LoginError::Failed;
        self.navigate(&site.login_url()).map_err(failed)?;
        let form = "!!document.querySelector('input[name=\"mail\"]')";
        let dashboard = format!("location.pathname.startsWith({})", js(DASHBOARD_PATH));
        self.wait_for(&format!("{dashboard} || {form}"), FORM_TIMEOUT);
        if self.eval(&signed_in_check()).map_err(failed)? == true {
            return Ok(());
        }
        let creds = read_credentials(&self.credentials)?;
        // Consent banners and the age splash cover the form.
        self.eval(
            r#"(() => {
                for (const b of document.querySelectorAll('button, a')) {
                    if (/^\s*(I Agree|Accept)\s*$/i.test(b.textContent || '')) b.click();
                }
                document.querySelectorAll('.splash-screen, .splash-screen__text-container').forEach((el) => el.remove());
            })()"#,
        )
        .map_err(failed)?;
        for (name, value) in [("mail", &creds.username), ("password", &creds.password)] {
            let found = self
                .eval(&format!(
                    "(() => {{ const el = document.querySelector('input[name=\"{name}\"]'); if (!el) return false; el.focus(); el.select(); return true }})()"
                ))
                .map_err(failed)?;
            if found != true {
                return Err(LoginError::Failed(format!(
                    "{} login form: no {name} field",
                    site.name
                )));
            }
            self.cdp
                .call(
                    "Input.insertText",
                    json!({ "text": value }),
                    Some(&self.session.clone()),
                )
                .map_err(failed)?;
        }
        let filled = self
            .eval("[...document.querySelectorAll('input[name=\"mail\"], input[name=\"password\"]')].every((el) => el.value.length > 0)")
            .map_err(failed)?;
        if filled != true {
            return Err(LoginError::Failed(format!(
                "{} login form didn't take the credentials",
                site.name
            )));
        }
        self.eval(
            r#"(() => {
                document.querySelectorAll('.splash-screen, .splash-screen__text-container').forEach((el) => el.remove());
                document.querySelector('button[type="submit"]')?.click();
            })()"#,
        )
        .map_err(failed)?;
        if self.wait_for(&dashboard, LOGIN_TIMEOUT) {
            return Ok(());
        }
        let shown = self
            .eval("(document.querySelector('.error, .form-error, [class*=\"error\"]')?.textContent || '').trim().slice(0, 200)")
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        let at = self
            .eval("location.pathname")
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "?".into());
        Err(LoginError::Refused(format!(
            "{} login failed (still on {at}){}",
            site.name,
            if shown.is_empty() {
                String::new()
            } else {
                format!(": {shown}")
            }
        )))
    }

    /// Right after a sign-in (or with a session the page refreshes on load)
    /// the server can still render the scene signed out, without links; such
    /// a page is loaded once more after [`RELOAD_WAIT`].
    fn nuxt_data(&mut self, site: &Site, slug: &str) -> Result<Option<Value>, String> {
        let mut text = String::new();
        for attempt in 0..2 {
            if attempt > 0 {
                std::thread::sleep(RELOAD_WAIT);
            }
            self.navigate(&site.scene_url(slug))?;
            let v = self.eval("document.getElementById('__NUXT_DATA__')?.textContent || ''")?;
            text = v.as_str().unwrap_or_default().to_string();
            if text.contains(".mp4?secure=") {
                break;
            }
        }
        Ok(Some(text)
            .filter(|t| !t.is_empty())
            .and_then(|t| serde_json::from_str(&t).ok()))
    }
}

impl Drop for Chrome {
    fn drop(&mut self) {
        let _ = self.cdp.call("Browser.close", json!({}), None);
    }
}

/// The member pages through the canned network: `GET <site>/login/` with a
/// non-2xx status is a refused login (anything else signs in), scene pages
/// come from `GET <scene url>` as HTML.
pub struct Canned<'a> {
    pub http: &'a dyn Http,
}

impl Members for Canned<'_> {
    fn login(&mut self, site: &Site) -> Result<(), LoginError> {
        let res = self
            .http
            .send(&Request::get(site.login_url()))
            .map_err(LoginError::Failed)?;
        if res.status == 404 || (200..300).contains(&res.status) {
            Ok(())
        } else {
            Err(LoginError::Refused(format!(
                "{} login failed (HTTP {})",
                site.name, res.status
            )))
        }
    }

    fn nuxt_data(&mut self, site: &Site, slug: &str) -> Result<Option<Value>, String> {
        let res = self.http.send(&Request::get(site.scene_url(slug)))?;
        if !(200..300).contains(&res.status) {
            return Err(format!("{}: HTTP {}", site.scene_url(slug), res.status));
        }
        Ok(nuxt::nuxt_data_of(&res.body))
    }
}

/// Chrome, started at the first sign-in (a fetch run with nothing to do
/// never starts it).
pub struct LazyChrome {
    pub user_data: PathBuf,
    pub credentials: PathBuf,
    pub headful: bool,
    chrome: Option<Chrome>,
}

impl LazyChrome {
    pub fn new(user_data: PathBuf, credentials: PathBuf, headful: bool) -> Self {
        LazyChrome {
            user_data,
            credentials,
            headful,
            chrome: None,
        }
    }

    fn get(&mut self) -> Result<&mut Chrome, String> {
        if self.chrome.is_none() {
            self.chrome = Some(Chrome::open(
                &self.user_data,
                self.credentials.clone(),
                self.headful,
            )?);
        }
        self.chrome
            .as_mut()
            .ok_or_else(|| "Chrome isn't open".into())
    }
}

impl Members for LazyChrome {
    fn login(&mut self, site: &Site) -> Result<(), LoginError> {
        self.get().map_err(LoginError::Failed)?.login(site)
    }

    fn nuxt_data(&mut self, site: &Site, slug: &str) -> Result<Option<Value>, String> {
        self.get()?.nuxt_data(site, slug)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sites::by_key;
    use slr_scout::http::Fake;

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
        assert!(credentials_path(None, None).ends_with("../../vb_hack.txt"));
        let missing = read_credentials(Path::new("/nonexistent/vb_hack.txt"));
        assert!(matches!(missing, Err(LoginError::Refused(_))));
    }

    #[test]
    fn canned_pages() {
        let dir = std::env::temp_dir().join(format!("vrb-scout-canned-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let fixtures = dir.join("f.json");
        fs::write(
            &fixtures,
            serde_json::json!({
                "GET https://vrconk.com/login/": {"status": 403, "body": ""},
                "GET https://vrbangers.com/video/a/": {"status": 200,
                    "body": "<script id=\"__NUXT_DATA__\" type=\"application/json\">[[\"Reactive\",1],{}]</script>"}
            })
            .to_string(),
        )
        .unwrap();
        let fake = Fake::load(&fixtures).unwrap();
        let mut m = Canned { http: &fake };
        let vrb = by_key("vrbangers").unwrap();
        let conk = by_key("vrconk").unwrap();
        assert!(m.login(vrb).is_ok());
        assert!(matches!(m.login(conk), Err(LoginError::Refused(_))));
        assert_eq!(
            m.nuxt_data(vrb, "a").unwrap(),
            Some(serde_json::json!([["Reactive", 1], {}]))
        );
        assert!(m.nuxt_data(vrb, "missing").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
