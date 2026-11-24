//! `--follow [N]`: our Instagram account (the `--login` session) follows the
//! creators the user wants: a confirmed Instagram account and source
//! `favourite` or `swiped` (yes on the Creators site; one swiped no is
//! left alone). Instagram's follow API answers HTML and does nothing, so a
//! Chrome does it like a person: headless on the `--login` profile with the
//! session's cookies (headed when headless meets a login wall), her profile
//! page, the "Follow" button, then "Following" / "Requested". One browser
//! for the step, 20–40 s between creators; a login wall stops the run
//! (exit 3), a "Try Again Later" / action-blocked dialog too (exit 2, next
//! day). Recorded in the state (`follows`) and on her Instagram entry
//! (`followed`: following / requested); never undone.

use crate::instagram;
use crate::state::{Follow, State, Swipes};
use crate::store::{self, CreatorsFile};
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Chrome's profile for `--follow`, in userData (not `--login`'s).
pub const FOLLOW_PROFILE_DIR: &str = "instagram-follow-chrome";

/// Creators to follow, in slug order: an Instagram account, a source
/// favourite or swiped, not swiped no, not followed (or requested) yet.
pub fn due(file: &CreatorsFile, state: &State, swipes: &Swipes) -> Vec<String> {
    file.creators
        .iter()
        .filter(|(slug, c)| {
            c.instagram.is_some()
                && c.sources.iter().any(|s| s == "favourite" || s == "swiped")
                && !swipes.creator_is_no(slug)
                && state
                    .follows
                    .get(*slug)
                    .is_none_or(|f| f.status == "failed")
        })
        .map(|(slug, _)| slug.clone())
        .collect()
}

/// What a profile page shows.
#[derive(Debug, Clone, PartialEq)]
pub enum PageState {
    /// A "Follow" / "Follow Back" button.
    Follow,
    Following,
    /// A private account asked.
    Requested,
    /// A login form: the session is gone.
    LoginWall,
    /// "Try Again Later", action blocked: stop for today.
    Blocked,
    /// "Sorry, this page isn't available".
    Missing,
    /// Not loaded (yet), or a layout this doesn't know.
    Unknown,
}

/// The page as [`DOM_PROBE`] reads it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dom {
    /// The texts of the profile header's buttons.
    pub buttons: Vec<String>,
    /// The page's visible text (the start of it).
    pub text: String,
    pub login_form: bool,
    pub url: String,
}

/// Reads [`Dom`] in the page.
pub const DOM_PROBE: &str = "(()=>{const q=(s)=>[...document.querySelectorAll(s)];\
const b=q('header button, header div[role=button], main header [role=button]').map(x=>x.innerText.trim()).filter(Boolean);\
return {buttons:b.slice(0,30),text:(document.body?document.body.innerText:'').slice(0,4000),\
login:!!document.querySelector('input[name=\"password\"]'),url:location.href}})()";

/// Clicks the "Follow" / "Follow Back" button; whether there was one.
pub const CLICK_FOLLOW: &str = "(()=>{const b=[...document.querySelectorAll('header button, header div[role=button], main header [role=button]')]\
.find(x=>/^(Follow|Follow Back)$/.test(x.innerText.trim())); if(!b) return false; b.click(); return true})()";

impl Dom {
    pub fn from_value(v: &Value) -> Dom {
        Dom {
            buttons: v["buttons"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|b| b.as_str().map(str::to_string))
                .collect(),
            text: v["text"].as_str().unwrap_or_default().to_string(),
            login_form: v["login"] == true,
            url: v["url"].as_str().unwrap_or_default().to_string(),
        }
    }

    pub fn state(&self) -> PageState {
        let text = self.text.to_lowercase();
        if self.login_form || self.url.contains("/accounts/login") {
            return PageState::LoginWall;
        }
        if text.contains("try again later")
            || text.contains("action blocked")
            || text.contains("we restrict certain activity")
        {
            return PageState::Blocked;
        }
        if text.contains("this page isn't available") || text.contains("this page isn’t available")
        {
            return PageState::Missing;
        }
        let has = |t: &str| self.buttons.iter().any(|b| b == t);
        if has("Following") {
            PageState::Following
        } else if has("Requested") {
            PageState::Requested
        } else if has("Follow") || has("Follow Back") {
            PageState::Follow
        } else {
            PageState::Unknown
        }
    }
}

/// The browser `--follow` drives (a Chrome over DevTools; a fake in tests).
pub trait FollowBrowser {
    /// Open `url` and wait (15 s at most) until the page shows a state.
    fn open(&mut self, url: &str) -> Result<PageState, String>;
    /// Click "Follow" and wait (8 s at most) for the answer; `Ok(None)`
    /// when there was no button to click.
    fn click_follow(&mut self) -> Result<Option<PageState>, String>;
}

/// A Chrome over DevTools on the `--login` profile, the session's cookies
/// set.
pub struct Chrome {
    b: slr_scout::login::Browser,
}

/// Cookies as `Network.setCookies` takes them.
fn cookie_params(cookies: &[Value]) -> Vec<Value> {
    cookies
        .iter()
        .filter_map(|c| {
            let mut p = json!({
                "name": c["name"].as_str()?,
                "value": c["value"].as_str()?,
                "domain": c["domain"].as_str()?,
                "path": c["path"].as_str().unwrap_or("/"),
                "secure": c["secure"].as_bool().unwrap_or(true),
                "httpOnly": c["httpOnly"].as_bool().unwrap_or(false),
            });
            if let Some(s) = c["sameSite"].as_str() {
                p["sameSite"] = json!(s);
            }
            if let Some(e) = c["expires"].as_f64().filter(|e| *e > 0.0) {
                p["expires"] = json!(e);
            }
            Some(p)
        })
        .collect()
}

impl Chrome {
    pub fn open(
        user_data: &Path,
        session: &instagram::Session,
        headless: bool,
    ) -> Result<Chrome, String> {
        let extra: &[&str] = if headless { &["--headless=new"] } else { &[] };
        // Its own profile, not --login's: a Chrome left open on that one
        // (an interrupted login) holds it, and a second Chrome then hands
        // over to it without opening a DevTools port. The session's cookies
        // are set below, so the profile needs nothing of its own.
        let mut b = slr_scout::login::open_chrome_with(&user_data.join(FOLLOW_PROFILE_DIR), extra)?;
        let s = b.session.clone();
        b.cdp.call(
            "Network.setCookies",
            json!({ "cookies": cookie_params(&session.cookies) }),
            Some(&s),
        )?;
        Ok(Chrome { b })
    }

    fn dom(&mut self) -> Result<PageState, String> {
        let s = self.b.session.clone();
        Ok(Dom::from_value(&self.b.cdp.eval(&s, DOM_PROBE)?).state())
    }

    /// Poll the page until `done` accepts its state or `limit` passes.
    fn wait(
        &mut self,
        limit: Duration,
        done: impl Fn(&PageState) -> bool,
    ) -> Result<PageState, String> {
        let started = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let state = self.dom()?;
            if done(&state) || started.elapsed() > limit {
                return Ok(state);
            }
        }
    }
}

impl FollowBrowser for Chrome {
    fn open(&mut self, url: &str) -> Result<PageState, String> {
        let s = self.b.session.clone();
        self.b
            .cdp
            .call("Page.navigate", json!({ "url": url }), Some(&s))?;
        self.wait(Duration::from_secs(15), |st| *st != PageState::Unknown)
    }

    fn click_follow(&mut self) -> Result<Option<PageState>, String> {
        let s = self.b.session.clone();
        if self.b.cdp.eval(&s, CLICK_FOLLOW)? != true {
            return Ok(None);
        }
        self.wait(Duration::from_secs(8), |st| {
            !matches!(st, PageState::Follow | PageState::Unknown)
        })
        .map(Some)
    }
}

/// 20–40 s between creators.
fn pause() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    std::thread::sleep(Duration::from_millis(20_000 + u64::from(nanos) % 20_001));
}

pub struct FollowRun {
    pub code: i32,
    pub following: usize,
    pub requested: usize,
    pub failed: usize,
    pub left: usize,
}

/// Opens the browser: headless first, headed (`false`) after a login wall.
pub type Opener<'a> = dyn FnMut(bool) -> Result<Box<dyn FollowBrowser>, String> + 'a;

/// `--follow N`. Writes creators.json and the state after each creator.
#[allow(clippy::too_many_arguments)]
pub fn run(
    open: &mut Opener,
    base: &str,
    user_data: &Path,
    file: &mut CreatorsFile,
    state: &mut State,
    swipes: &Swipes,
    limit: usize,
    fake: bool,
    out: &mut dyn Write,
) -> Result<FollowRun, String> {
    let due = due(file, state, swipes);
    let mut run = FollowRun {
        code: 0,
        following: 0,
        requested: 0,
        failed: 0,
        left: due.len().saturating_sub(limit),
    };
    if due.is_empty() {
        return Ok(run);
    }
    let mut headless = true;
    let mut browser = open(headless)?;
    for (i, slug) in due.iter().take(limit).enumerate() {
        if i > 0 && !fake {
            pause();
        }
        let Some(handle) = file
            .creators
            .get(slug)
            .and_then(|c| c.instagram.as_ref())
            .map(|ig| ig.handle.clone())
        else {
            continue;
        };
        let url = format!("{base}/{handle}/");
        let mut seen = browser.open(&url)?;
        if seen == PageState::LoginWall && headless {
            // Headless may be refused where a visible window isn't.
            headless = false;
            browser = open(headless)?;
            seen = browser.open(&url)?;
        }
        let outcome = match seen {
            PageState::Follow => browser.click_follow()?.unwrap_or(PageState::Unknown),
            other => other,
        };
        let now = video_tags::timefmt::now_iso();
        let record = |status: &str, error: Option<String>| Follow {
            handle: handle.clone(),
            at: now.clone(),
            status: status.into(),
            error,
        };
        match outcome {
            PageState::Following | PageState::Requested => {
                let status = if outcome == PageState::Following {
                    run.following += 1;
                    "following"
                } else {
                    run.requested += 1;
                    "requested"
                };
                let _ = writeln!(out, "  @{handle}: {status}");
                state.follows.insert(slug.clone(), record(status, None));
                if let Some(ig) = file
                    .creators
                    .get_mut(slug)
                    .and_then(|c| c.instagram.as_mut())
                {
                    ig.followed = Some(status.into());
                }
            }
            PageState::LoginWall => {
                let _ = writeln!(
                    out,
                    "Instagram asks to log in: the session expired; run --login."
                );
                run.code = 3;
                break;
            }
            PageState::Blocked => {
                let _ = writeln!(
                    out,
                    "Instagram: try again later (action blocked); no more follows today."
                );
                run.code = 2;
                break;
            }
            PageState::Missing | PageState::Follow | PageState::Unknown => {
                let why = match outcome {
                    PageState::Missing => "account not found",
                    PageState::Follow => "the click didn't take",
                    _ => "no Follow button on her page",
                };
                let _ = writeln!(out, "  @{handle}: {why}; tried again next run.");
                run.failed += 1;
                run.code = run.code.max(2);
                state
                    .follows
                    .insert(slug.clone(), record("failed", Some(why.into())));
            }
        }
        store::write(user_data, file)?;
        crate::state::write(user_data, state)?;
    }
    if run.code == 0 && run.left > 0 {
        run.code = 2;
    }
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Creator, Instagram};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    fn dom(buttons: &[&str], text: &str) -> Dom {
        Dom {
            buttons: buttons.iter().map(|b| b.to_string()).collect(),
            text: text.into(),
            ..Dom::default()
        }
    }

    #[test]
    fn reads_page_states() {
        assert_eq!(dom(&["Follow", "Message"], "").state(), PageState::Follow);
        assert_eq!(dom(&["Follow Back"], "").state(), PageState::Follow);
        assert_eq!(
            dom(&["Following", "Message"], "").state(),
            PageState::Following
        );
        assert_eq!(dom(&["Requested"], "").state(), PageState::Requested);
        assert_eq!(
            dom(&["Follow"], "Try Again Later\nWe limit how often…").state(),
            PageState::Blocked
        );
        assert_eq!(
            dom(&[], "Sorry, this page isn't available.").state(),
            PageState::Missing
        );
        assert_eq!(dom(&[], "Loading").state(), PageState::Unknown);
        let login = Dom {
            login_form: true,
            ..dom(&["Log in"], "")
        };
        assert_eq!(login.state(), PageState::LoginWall);
        let v = json!({"buttons": ["Following"], "text": "x", "login": false, "url": "https://www.instagram.com/accounts/login/?next=/a/"});
        assert_eq!(Dom::from_value(&v).state(), PageState::LoginWall);
        let cookies = cookie_params(&[
            json!({"name": "sessionid", "value": "S", "domain": ".instagram.com",
            "path": "/", "expires": 1.9e9, "size": 9, "httpOnly": true, "secure": true, "sameSite": "Lax"}),
        ]);
        assert_eq!(cookies[0]["sameSite"], "Lax");
        assert!(cookies[0].get("size").is_none());
    }

    /// Page states per URL; each `open` answers the next one, each click the
    /// URL's click answer.
    #[derive(Clone, Default)]
    struct Fake {
        pages: Rc<RefCell<HashMap<String, Vec<PageState>>>>,
        clicks: Rc<RefCell<HashMap<String, PageState>>>,
        log: Rc<RefCell<Vec<String>>>,
        current: String,
    }

    impl FollowBrowser for Fake {
        fn open(&mut self, url: &str) -> Result<PageState, String> {
            self.current = url.into();
            self.log.borrow_mut().push(format!("open {url}"));
            let mut pages = self.pages.borrow_mut();
            let list = pages.get_mut(url).ok_or("no page")?;
            Ok(if list.len() > 1 {
                list.remove(0)
            } else {
                list[0].clone()
            })
        }

        fn click_follow(&mut self) -> Result<Option<PageState>, String> {
            self.log
                .borrow_mut()
                .push(format!("click {}", self.current));
            Ok(self.clicks.borrow().get(&self.current).cloned())
        }
    }

    fn setup() -> (CreatorsFile, std::path::PathBuf) {
        let mut file = CreatorsFile::empty();
        for (slug, sources) in [
            ("a1", vec!["favourite"]),
            ("a2", vec!["swiped"]),
            ("a3", vec!["favourite", "pornstar"]),
            ("a4", vec!["favourite"]),
            ("a5", vec!["favourite"]),
            ("star", vec!["pornstar"]),
        ] {
            file.creators.insert(
                slug.into(),
                Creator {
                    instagram: Some(Instagram {
                        handle: slug.into(),
                        ..Instagram::default()
                    }),
                    sources: sources.into_iter().map(String::from).collect(),
                    ..Creator::default()
                },
            );
        }
        // One dir per call: the tests run in parallel threads of one process, and a
        // shared dir was wiped by the next test's remove_dir_all mid-run.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("creator-scout-follow-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (file, dir)
    }

    #[test]
    fn follows_by_clicking_and_stops_when_blocked_or_logged_out() {
        let (mut file, dir) = setup();
        let fake = Fake::default();
        let u = |h: &str| format!("https://ig/{h}/");
        for (h, s) in [
            ("a1", PageState::Follow),
            ("a2", PageState::Follow),
            ("a3", PageState::Following),
            ("a4", PageState::Follow),
            ("a5", PageState::Follow),
        ] {
            fake.pages.borrow_mut().insert(u(h), vec![s]);
        }
        fake.clicks
            .borrow_mut()
            .insert(u("a1"), PageState::Following);
        fake.clicks
            .borrow_mut()
            .insert(u("a2"), PageState::Requested);
        fake.clicks.borrow_mut().insert(u("a4"), PageState::Blocked);
        let opened = Rc::new(RefCell::new(Vec::new()));
        let (f, o) = (fake.clone(), opened.clone());
        let mut open = move |headless: bool| -> Result<Box<dyn FollowBrowser>, String> {
            o.borrow_mut().push(headless);
            Ok(Box::new(f.clone()))
        };
        let mut state = State::default();
        let mut out = Vec::new();
        let r = run(
            &mut open,
            "https://ig",
            &dir,
            &mut file,
            &mut state,
            &Swipes::default(),
            10,
            true,
            &mut out,
        )
        .unwrap();
        // Blocked at a4: stop for today.
        assert_eq!((r.code, r.following, r.requested), (2, 2, 1));
        assert_eq!(state.follows["a1"].status, "following");
        assert_eq!(state.follows["a2"].status, "requested");
        // Followed already: no click.
        assert_eq!(state.follows["a3"].status, "following");
        assert!(!fake.log.borrow().contains(&format!("click {}", u("a3"))));
        assert!(!state.follows.contains_key("a4") && !state.follows.contains_key("star"));
        assert_eq!(
            file.creators["a2"]
                .instagram
                .as_ref()
                .unwrap()
                .followed
                .as_deref(),
            Some("requested")
        );
        assert_eq!(*opened.borrow(), [true]);

        // Next day: headless meets a login wall, the visible window too.
        fake.pages
            .borrow_mut()
            .insert(u("a4"), vec![PageState::LoginWall]);
        let r = run(
            &mut open,
            "https://ig",
            &dir,
            &mut file,
            &mut state,
            &Swipes::default(),
            10,
            true,
            &mut out,
        )
        .unwrap();
        assert_eq!(r.code, 3);
        assert_eq!(*opened.borrow(), [true, true, false]);
        // A headed retry that works: followed.
        fake.pages
            .borrow_mut()
            .insert(u("a4"), vec![PageState::LoginWall, PageState::Follow]);
        fake.clicks
            .borrow_mut()
            .insert(u("a4"), PageState::Following);
        fake.pages
            .borrow_mut()
            .insert(u("a5"), vec![PageState::Missing]);
        let r = run(
            &mut open,
            "https://ig",
            &dir,
            &mut file,
            &mut state,
            &Swipes::default(),
            1,
            true,
            &mut out,
        )
        .unwrap();
        assert_eq!((r.code, r.following, r.left), (2, 1, 1));
        assert_eq!(state.follows["a4"].status, "following");
        let r = run(
            &mut open,
            "https://ig",
            &dir,
            &mut file,
            &mut state,
            &Swipes::default(),
            5,
            true,
            &mut out,
        )
        .unwrap();
        assert_eq!((r.code, r.failed), (2, 1));
        assert_eq!(
            state.follows["a5"].error.as_deref(),
            Some("account not found")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn follows_favourites_and_swiped_yes_with_instagram() {
        let (mut file, _) = setup();
        file.creators.insert(
            "noig".into(),
            Creator {
                sources: vec!["favourite".into()],
                ..Creator::default()
            },
        );
        let mut state = State::default();
        let rec = |status: &str| Follow {
            handle: "h".into(),
            at: "t".into(),
            status: status.into(),
            error: None,
        };
        state.follows.insert("a1".into(), rec("requested"));
        state.follows.insert("a2".into(), rec("failed"));
        let mut swipes = Swipes::default();
        swipes.creators.insert("a5".into(), "no".into());
        assert_eq!(due(&file, &state, &swipes), ["a2", "a3", "a4"]);
    }
}
