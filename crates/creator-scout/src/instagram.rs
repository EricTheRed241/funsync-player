//! Instagram: the session (`--login`: a visible Chrome signs in with the
//! credentials file, or the user does; its cookies go to
//! `instagram-session.json`, 0600; credentials and cookies are never
//! printed) and the pictures step (`--pictures`): with a session, the
//! newest timeline posts the profile page embeds (fetched like Chrome
//! navigating to it; Instagram's JSON endpoints answer 404 since
//! 2026-11); without, the profile photo from the public page.

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
    /// The media id (`pk`).
    pub id: String,
    pub shortcode: String,
    /// The biggest rendition at most 1080 px wide (else the post's
    /// `/media/?size=l` redirect to its picture).
    pub image: String,
    pub taken_at: i64,
}

/// A profile page as a logged-in browser gets it.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    /// The counts, name and bio of its description meta, when it has them.
    pub meta: Option<verify::IgPage>,
    /// Newest first, at most 12.
    pub items: Vec<Item>,
}

/// The timeline the profile page embeds (`{"edges": [{"node": …}], …}`).
const TIMELINE_KEY: &str = "\"xdt_api__v1__feed__user_timeline_graphql_connection\":";

/// The JSON object or array `s` starts with, by bracket matching outside
/// strings.
fn json_span(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    let (open, close) = match b.first()? {
        b'[' => (b'[', b']'),
        b'{' => (b'{', b'}'),
        _ => return None,
    };
    let (mut depth, mut in_str, mut esc) = (0usize, false, false);
    for (i, &c) in b.iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else if c == b'"' {
            in_str = true;
        } else if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(&s[..=i]);
            }
        }
    }
    None
}

/// The biggest of `image_versions2.candidates` at most 1080 px wide.
fn best_candidate(v: &Value) -> Option<String> {
    v["image_versions2"]["candidates"]
        .as_array()?
        .iter()
        .filter_map(|c| Some((c["width"].as_u64()?, c["url"].as_str()?)))
        .filter(|(w, u)| *w <= MAX_WIDTH && u.starts_with("https://"))
        .max_by_key(|(w, _)| *w)
        .map(|(_, u)| u.to_string())
}

pub fn text_id(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The timeline connection a profile page embeds (`{"edges": [...], …}`).
pub fn timeline_json(page: &str) -> Option<Value> {
    let at = page.find(TIMELINE_KEY)? + TIMELINE_KEY.len();
    serde_json::from_str(json_span(page[at..].trim_start())?).ok()
}

/// The timeline posts a profile page embeds, newest first, at most 12;
/// `None` when it embeds no timeline.
pub fn parse_timeline(page: &str) -> Option<Vec<Item>> {
    let conn = timeline_json(page)?;
    let mut items: Vec<Item> = conn["edges"]
        .as_array()?
        .iter()
        .filter_map(|e| {
            let n = &e["node"];
            let shortcode = n["code"].as_str().filter(|c| store::safe_name(c))?;
            let image = best_candidate(n)
                .or_else(|| best_candidate(&n["carousel_media"][0]))
                .or_else(|| {
                    n["display_uri"]
                        .as_str()
                        .filter(|u| u.starts_with("https://"))
                        .map(str::to_string)
                })
                .unwrap_or_else(|| {
                    format!("{}/p/{shortcode}/media/?size=l", store::INSTAGRAM_SITE)
                });
            Some(Item {
                id: text_id(&n["pk"]).or_else(|| text_id(&n["id"]))?,
                shortcode: shortcode.to_string(),
                image,
                taken_at: n["taken_at"].as_i64()?,
            })
        })
        .collect();
    items.sort_by(|a, b| b.taken_at.cmp(&a.taken_at));
    items.truncate(TIMELINE_POSTS);
    Some(items)
}

/// What a profile page answer means.
#[derive(Debug, PartialEq)]
pub enum Answer {
    Profile(Box<Profile>),
    Missing,
    NotLoggedIn,
    /// A challenge or checkpoint: the account must be confirmed in a browser.
    Challenge,
    /// 429: stop for today.
    Limited(String),
    Failed(String),
}

/// Whether the page's `<html>` says the session is gone (class
/// `not-logged-in`).
fn not_logged_in(page: &str) -> bool {
    page.find("<html")
        .and_then(|i| page[i..].find('>').map(|j| &page[i..i + j + 1]))
        .and_then(|tag| crate::html::attr(tag, "class"))
        .is_some_and(|c| c.split_whitespace().any(|x| x == "not-logged-in"))
}

/// A profile page fetched with the session ([`timeline_request`]).
pub fn classify_page(res: &Response) -> Answer {
    let location = res.header_values("location").next().unwrap_or("");
    match res.status {
        429 => Answer::Limited("HTTP 429".into()),
        300..=399 if location.contains("/challenge") => Answer::Challenge,
        300..=399 if location.contains("/accounts/login") => Answer::NotLoggedIn,
        404 => Answer::Missing,
        200..=299 => {
            if not_logged_in(&res.body) {
                return Answer::NotLoggedIn;
            }
            let meta = verify::parse_instagram_page(&res.body);
            match parse_timeline(&res.body) {
                Some(items) => Answer::Profile(Box::new(Profile { meta, items })),
                // A private account, or none posted yet.
                None if meta.is_some() => Answer::Profile(Box::new(Profile {
                    meta,
                    items: Vec::new(),
                })),
                None => Answer::Failed("a page without the profile".into()),
            }
        }
        s => Answer::Failed(format!("HTTP {s}")),
    }
}

/// The profile page as Chrome navigates to it with the session (without
/// the `Sec-Fetch-*` headers the page leaves the posts out).
pub fn timeline_request(base: &str, handle: &str, cookie: &str) -> Request {
    Request::get(format!("{base}/{handle}/"))
        .header("User-Agent", slr_scout::http::USER_AGENT)
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Sec-Fetch-Mode", "navigate")
        .header("Sec-Fetch-Site", "none")
        .header("Sec-Fetch-Dest", "document")
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
/// none fetched for 3 days): those with a source first (swiped, then
/// favourite, then pornstar list), then the rest; stalest first in each.
pub fn due(file: &CreatorsFile, state: &State, swipes: &Swipes, now: i64) -> Vec<String> {
    let mut list: Vec<(usize, Option<i64>, String)> = file
        .creators
        .iter()
        .filter(|(slug, c)| c.instagram.is_some() && !swipes.creator_is_no(slug))
        .map(|(slug, c)| {
            (
                c.source_rank(),
                state.pictures.get(slug).and_then(|t| parse_iso(t)),
                slug.clone(),
            )
        })
        .filter(|(_, at, _)| at.is_none_or(|t| now - t >= (REFRESH_DAYS * 86_400) as i64))
        .collect();
    list.sort();
    list.into_iter().map(|(_, _, s)| s).collect()
}

/// 8–15 s between Instagram requests.
pub fn pause() {
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
                let req = timeline_request(base, &ig.handle, cookie);
                let res = http.send(&req).map_err(|e| format!("Instagram: {e}"))?;
                match classify_page(&res) {
                    Answer::Profile(p) => {
                        let mut ig = ig;
                        // The counts: the page's meta, else the preview bot's.
                        let meta = match &p.meta {
                            Some(m) => Some(m.clone()),
                            None => {
                                match verify::instagram(http, &mut page_pacer, base, &ig.handle) {
                                    Ok(m) => m,
                                    Err(slr_scout::http::NetError::RateLimited(m)) => {
                                        let _ =
                                            writeln!(out, "Instagram: {m}; stopping for today.");
                                        run.code = 2;
                                        break;
                                    }
                                    Err(e) => {
                                        let _ = writeln!(out, "  {}: counts: {e}", c.name);
                                        None
                                    }
                                }
                            }
                        };
                        if let Some(m) = meta {
                            ig.followers = Some(m.followers);
                            ig.posts = Some(m.posts);
                            if !m.full_name.is_empty() {
                                ig.full_name = Some(m.full_name);
                            }
                            if m.bio.is_some() {
                                ig.bio = m.bio;
                            }
                        }
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
                    Answer::Challenge => {
                        let _ = writeln!(out, "Instagram: challenge; stopping for today.");
                        run.code = 2;
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

    fn node(id: u64, t: i64) -> Value {
        json!({"node": {"code": format!("S{id}"), "pk": id.to_string(), "id": format!("{id}_9"),
            "taken_at": t, "media_type": 1, "product_type": "feed", "timeline_pinned_user_ids": [],
            "owner_id": {"pk": "9"},
            "image_versions2": {"candidates": [
                {"width": 1440, "height": 1800, "url": format!("https://cdn.ig/{id}_1440.jpg")},
                {"width": 1080, "height": 1350, "url": format!("https://cdn.ig/{id}_1080.jpg")},
                {"width": 640, "height": 800, "url": format!("https://cdn.ig/{id}_640.jpg")}]}}})
    }

    /// A trimmed profile page: the description meta and the embedded
    /// timeline (its cursor holds a brace, as real ones may).
    fn page(class: &str, edges: &[Value]) -> String {
        let conn =
            json!({"edges": edges, "page_info": {"end_cursor": "QV}{x\"y", "has_next_page": true}});
        format!(
            r#"<!DOCTYPE html><html class="{class}" lang="en"><head>
            <meta property="og:description" content="668K Followers, 192 Following, 39 Posts - See Instagram photos and videos from Hannah (&#064;itshannahowo)" />
            </head><body><script type="application/json">{{"require":[["ScheduledServerJS",{{"__bbox":{{"result":{{"data":{{"xdt_api__v1__feed__user_timeline_graphql_connection":{conn}}}}}}}}}]]}}</script></body></html>"#
        )
    }

    #[test]
    fn parses_the_timeline_the_page_embeds() {
        let mut edges: Vec<Value> = (1..=14)
            .map(|i| node(i, 1_700_000_000 + i as i64))
            .collect();
        // A pinned old post, a carousel without its own picture, one
        // without any (its /media/ redirect), one without a code.
        edges.push(
            json!({"node": {"code": "OLD", "pk": 1, "taken_at": 1_600_000_000,
            "timeline_pinned_user_ids": ["9"], "display_uri": "https://cdn.ig/old.jpg"}}),
        );
        edges.push(json!({"node": {"code": "CAR", "pk": "50", "taken_at": 1_800_000_000,
            "carousel_media": [{"image_versions2": {"candidates": [{"width": 720, "url": "https://cdn.ig/c.jpg"}]}}]}}));
        edges.push(json!({"node": {"code": "BARE", "pk": "51", "taken_at": 1_799_999_999}}));
        edges.push(json!({"node": {"pk": "52", "taken_at": 1_900_000_000}}));
        let items = parse_timeline(&page("_9dls", &edges)).unwrap();
        assert_eq!(items.len(), TIMELINE_POSTS);
        assert_eq!(
            (
                items[0].id.as_str(),
                items[0].shortcode.as_str(),
                items[0].image.as_str()
            ),
            ("50", "CAR", "https://cdn.ig/c.jpg")
        );
        assert_eq!(
            items[1].image,
            "https://www.instagram.com/p/BARE/media/?size=l"
        );
        assert_eq!(
            (items[2].id.as_str(), items[2].image.as_str()),
            ("14", "https://cdn.ig/14_1080.jpg")
        );
        assert_eq!(items[2].taken_at, 1_700_000_014);
        assert_eq!(parse_timeline("<html><body>nothing</body></html>"), None);
        assert_eq!(
            json_span(r#"{"a":"}\"{","b":[1,{"c":2}]} tail"#),
            Some(r#"{"a":"}\"{","b":[1,{"c":2}]}"#)
        );
        assert_eq!(json_span("{unclosed"), None);
    }

    fn res(status: u16, body: &str) -> Response {
        Response {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    #[test]
    fn classifies_profile_pages() {
        let Answer::Profile(p) = classify_page(&res(200, &page("_9dls x", &[node(1, 5)]))) else {
            panic!("not a profile");
        };
        assert_eq!(p.items.len(), 1);
        let meta = p.meta.unwrap();
        assert_eq!(
            (meta.followers.as_str(), meta.posts.as_str()),
            ("668K", "39")
        );
        // A private account: the meta, no timeline.
        let private = r#"<html class="_9dls"><meta name="description" content="5M Followers, 1 Following, 2 Posts - A (&#064;a) on Instagram"></html>"#;
        let Answer::Profile(p) = classify_page(&res(200, private)) else {
            panic!("not a profile");
        };
        assert!(p.items.is_empty() && p.meta.is_some());
        assert_eq!(
            classify_page(&res(200, &page("_9dls not-logged-in", &[node(1, 5)]))),
            Answer::NotLoggedIn
        );
        assert!(matches!(
            classify_page(&res(200, "<html class=\"x\">?</html>")),
            Answer::Failed(_)
        ));
        assert_eq!(classify_page(&res(404, "")), Answer::Missing);
        assert!(matches!(classify_page(&res(429, "")), Answer::Limited(_)));
        assert!(matches!(classify_page(&res(500, "")), Answer::Failed(_)));
        let redirect = |to: &str| Response {
            status: 302,
            headers: vec![("location".into(), to.into())],
            body: String::new(),
        };
        assert_eq!(
            classify_page(&redirect(
                "https://www.instagram.com/accounts/login/?next=%2Fa%2F"
            )),
            Answer::NotLoggedIn
        );
        assert_eq!(
            classify_page(&redirect("https://www.instagram.com/challenge/?next=/a/")),
            Answer::Challenge
        );
        let req = timeline_request("https://www.instagram.com", "a.b", "sessionid=S");
        assert_eq!(req.url, "https://www.instagram.com/a.b/");
        for h in [
            ("Sec-Fetch-Mode", "navigate"),
            ("Sec-Fetch-Site", "none"),
            ("Sec-Fetch-Dest", "document"),
            ("Cookie", "sessionid=S"),
        ] {
            assert!(req.headers.contains(&(h.0.into(), h.1.into())), "{h:?}");
        }
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
        // Sources go first: swiped, favourite, pornstar list, then the rest.
        file.creators.get_mut("a").unwrap().sources = vec!["pornstar".into()];
        state.pictures.remove("a");
        file.creators.get_mut("b").unwrap().sources = vec!["favourite".into(), "pornstar".into()];
        file.creators.get_mut("c").unwrap().sources = vec!["swiped".into()];
        state
            .pictures
            .insert("c".into(), "2026-11-01T00:00:00.000Z".into());
        // A creator never fetched but without a source comes after them.
        file.creators.insert(
            "f".into(),
            store::Creator {
                name: "f".into(),
                instagram: Some(Instagram {
                    handle: "f".into(),
                    ..Instagram::default()
                }),
                ..store::Creator::default()
            },
        );
        assert_eq!(due(&file, &state, &swipes, now), ["c", "b", "a", "f"]);
        assert_eq!(parse_iso("1970-01-02T00:00:01Z"), Some(86_401));
        assert_eq!(iso_of(1_793_793_600), "2026-11-04T12:00:00.000Z");
    }
}
