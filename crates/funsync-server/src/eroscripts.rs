//! EroScripts in the app (`electron/eroscripts-api.js`): Discourse login
//! (with 2FA, or a pasted `_t` cookie), topic search with thumbnails and the
//! Paid badge, a topic's every script across the post and its replies
//! (labelled by poster), pixeldrain listings, other hosts named, and saving
//! a script next to its video or a post's picture as the video's poster.
//!
//! HTTP goes through `slr_scout::http::Curl` (blocking, on the blocking
//! pool). The session lives in `<data dir>/eroscripts-session.json` (0600).
//! `FUNSYNC_EROSCRIPTS_URL` / `FUNSYNC_PIXELDRAIN_URL` point at a fake
//! forum in tests.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use axum::Router;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use slr_scout::forum::{FORUM, decode_entities, script_axis};
use slr_scout::http::{Curl, Http, Pacer, Request};

use crate::server::AppState;

const SESSION_FILE: &str = "eroscripts-session.json";
const PAID_SLUG: &str = "paid-scripts";
const MAX_POSTS: usize = 100;
const POST_BATCH: usize = 20;
const MAX_PIXELDRAIN_LOOKUPS: usize = 10;
const MIN_THUMB_WIDTH: u64 = 400;
const POSTER_WIDTH: u64 = 640;
const MIN_POSTER_SOURCE_WIDTH: u64 = 320;
const UPLOAD_CDN: &str = "https://eroscripts-discourse.eroscripts.com/";
const EXTERNAL_HOSTS: &[(&str, &str)] = &[
    ("mega.nz", "MEGA"),
    ("mega.io", "MEGA"),
    ("drive.google.com", "Google Drive"),
    ("mediafire.com", "MediaFire"),
    ("gofile.io", "Gofile"),
    ("dropbox.com", "Dropbox"),
    ("files.fm", "Files.fm"),
    ("workupload.com", "Workupload"),
    ("1drv.ms", "OneDrive"),
    ("onedrive.live.com", "OneDrive"),
    ("terabox.com", "TeraBox"),
    ("pixeldrain.com", "pixeldrain"),
    ("pixeldrain.net", "pixeldrain"),
];

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn base() -> String {
    env_or("FUNSYNC_EROSCRIPTS_URL", FORUM)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub cookie: String,
    pub username: String,
}

pub fn load_session(dir: &Path) -> Option<Session> {
    let s: Session = serde_json::from_slice(&std::fs::read(dir.join(SESSION_FILE)).ok()?).ok()?;
    (!s.cookie.is_empty()).then_some(s)
}

/// Written owner-only (0600): the cookie is a login.
pub fn save_session(dir: &Path, s: Option<&Session>) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let path = dir.join(SESSION_FILE);
    let Some(s) = s else {
        return std::fs::remove_file(&path).or_else(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(e),
        });
    };
    let tmp = dir.join(format!("{SESSION_FILE}.tmp"));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(&serde_json::to_vec(s)?)?;
    std::fs::rename(tmp, path)
}

/// Why a forum call failed; each becomes the answer the UI shows.
#[derive(Debug, PartialEq)]
pub enum Fail {
    Auth,
    RateLimited,
    Other(String),
}

impl IntoResponse for Fail {
    fn into_response(self) -> Response {
        let (code, body) = match self {
            Fail::Auth => (
                StatusCode::UNAUTHORIZED,
                json!({"error": "Access denied — log in to EroScripts again", "authExpired": true}),
            ),
            Fail::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                json!({"error": "Rate limited — try again in a moment", "rateLimited": true}),
            ),
            Fail::Other(e) => (StatusCode::BAD_GATEWAY, json!({ "error": e })),
        };
        (code, Json(body)).into_response()
    }
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `name=value` of every `Set-Cookie`, joined for a `Cookie` header.
fn cookies(res: &slr_scout::http::Response) -> Vec<(String, String)> {
    res.header_values("set-cookie")
        .filter_map(|c| {
            let (n, v) = c.split(';').next()?.split_once('=')?;
            Some((n.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

fn get_json(url: &str, cookie: Option<&str>) -> Result<Value, Fail> {
    let mut req = Request::get(url).header("Accept", "application/json");
    if let Some(c) = cookie {
        req = req.header("Cookie", format!("_t={c}"));
    }
    let res = Curl.send(&req).map_err(Fail::Other)?;
    match res.status {
        401 | 403 => return Err(Fail::Auth),
        429 => return Err(Fail::RateLimited),
        200..=299 => {}
        s => return Err(Fail::Other(format!("EroScripts answered HTTP {s}"))),
    }
    serde_json::from_str(&res.body)
        // An HTML page (Cloudflare, a login redirect) means no session.
        .map_err(|_| Fail::Auth)
}

/// The outcome of a login attempt.
#[derive(Debug, PartialEq)]
pub enum Login {
    In(Session),
    NeedsCode,
}

/// Discourse's own login: a CSRF token, then `POST /session` with the
/// one-time code when the account has 2FA. The `_t` cookie is the session.
pub fn login(user: &str, password: &str, code: &str) -> Result<Login, Fail> {
    let base = base();
    let csrf_res = Curl
        .send(
            &Request::get(format!("{base}/session/csrf.json"))
                .header("Accept", "application/json")
                .header("X-Requested-With", "XMLHttpRequest"),
        )
        .map_err(Fail::Other)?;
    let csrf: Value = serde_json::from_str(&csrf_res.body).unwrap_or_default();
    let csrf = csrf["csrf"]
        .as_str()
        .ok_or_else(|| Fail::Other("EroScripts gave no login token".into()))?;
    let jar: Vec<String> = cookies(&csrf_res)
        .into_iter()
        .map(|(n, v)| format!("{n}={v}"))
        .collect();
    let form = [
        ("login", user),
        ("password", password),
        ("second_factor_token", code),
        ("second_factor_method", "1"),
    ]
    .iter()
    .map(|(k, v)| format!("{k}={}", enc(v)))
    .collect::<Vec<_>>()
    .join("&");
    let res = Curl
        .send(
            &Request::post(format!("{base}/session"), form)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .header("Accept", "application/json")
                .header("X-Requested-With", "XMLHttpRequest")
                .header("X-CSRF-Token", csrf)
                .header("Cookie", jar.join("; ")),
        )
        .map_err(Fail::Other)?;
    if res.status == 429 {
        return Err(Fail::RateLimited);
    }
    let v: Value = serde_json::from_str(&res.body).unwrap_or_default();
    if let Some((_, t)) = cookies(&res).into_iter().find(|(n, _)| n == "_t") {
        let username = v["user"]["username"].as_str().unwrap_or(user).to_string();
        return Ok(Login::In(Session {
            cookie: t,
            username,
        }));
    }
    if v["reason"] == "invalid_second_factor" && code.is_empty() {
        return Ok(Login::NeedsCode);
    }
    Err(Fail::Other(
        v["error"].as_str().unwrap_or("Login failed").to_string(),
    ))
}

/// A pasted `_t` cookie, checked against `/session/current.json`.
pub fn login_with_cookie(cookie: &str) -> Result<Session, Fail> {
    let cookie = cookie.trim();
    let cookie = cookie.strip_prefix("_t=").unwrap_or(cookie);
    let v = get_json(&format!("{}/session/current.json", base()), Some(cookie))?;
    let username = v["current_user"]["username"].as_str().ok_or(Fail::Auth)?;
    Ok(Session {
        cookie: cookie.to_string(),
        username: username.to_string(),
    })
}

fn absolute(base: &str, url: &str) -> String {
    if url.starts_with("http") {
        url.to_string()
    } else {
        format!("{base}{url}")
    }
}

/// The smallest `thumbnails[]` entry at least `min` wide, else the first.
fn pick_thumbnail(t: &Value, min: u64) -> Option<&str> {
    let usable: Vec<&Value> = t
        .as_array()?
        .iter()
        .filter(|x| x["url"].as_str().is_some_and(|u| !u.is_empty()))
        .collect();
    let width = |x: &Value| x["width"].as_u64().unwrap_or(0);
    usable
        .iter()
        .filter(|x| width(x) >= min)
        .min_by_key(|x| width(x))
        .or(usable.first())
        .and_then(|x| x["url"].as_str())
}

fn widest(t: &Value) -> u64 {
    t.as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x["width"].as_u64())
        .max()
        .unwrap_or(0)
}

static CATEGORIES: Mutex<Option<HashMap<u64, String>>> = Mutex::new(None);

/// Category id → slug, fetched once.
fn category_slugs(cookie: &str) -> HashMap<u64, String> {
    if let Some(m) = CATEGORIES.lock().unwrap().clone() {
        return m;
    }
    let mut m = HashMap::new();
    if let Ok(v) = get_json(
        &format!("{}/categories.json?include_subcategories=true", base()),
        Some(cookie),
    ) {
        for c in v["category_list"]["categories"]
            .as_array()
            .into_iter()
            .flatten()
        {
            for c in
                std::iter::once(c).chain(c["subcategory_list"].as_array().into_iter().flatten())
            {
                if let (Some(id), Some(slug)) = (c["id"].as_u64(), c["slug"].as_str()) {
                    m.insert(id, slug.to_string());
                }
            }
        }
    }
    if !m.is_empty() {
        *CATEGORIES.lock().unwrap() = Some(m.clone());
    }
    m
}

fn tag_names(tags: &Value) -> Vec<String> {
    tags.as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_str().or(t["name"].as_str()).map(str::to_string))
        .collect()
}

/// Search results (`search` in eroscripts-api.js), topics or, when the
/// forum answers with posts only, one result per topic of the posts.
pub fn parse_search(base: &str, data: &Value, slugs: &HashMap<u64, String>) -> Vec<Value> {
    static IMG: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)<img[^>]+src="([^"]+)""#).unwrap());
    let posts = data["posts"].as_array().map_or(&[][..], Vec::as_slice);
    let topics = data["topics"].as_array().map_or(&[][..], Vec::as_slice);
    let topic_of = |id: &Value| topics.iter().find(|t| t["id"] == *id);
    let avatar = |p: Option<&Value>| {
        p.and_then(|p| p["avatar_template"].as_str())
            .map(|a| absolute(base, &a.replace("{size}", "90")))
    };
    let mut seen = Vec::new();
    let rows: Vec<(Option<&Value>, Option<&Value>, u64)> = if !topics.is_empty() {
        topics
            .iter()
            .map(|t| {
                let p = posts.iter().find(|p| p["topic_id"] == t["id"]);
                (Some(t), p, t["id"].as_u64().unwrap_or(0))
            })
            .collect()
    } else {
        posts
            .iter()
            .filter_map(|p| {
                let id = p["topic_id"].as_u64()?;
                (!seen.contains(&id)).then(|| {
                    seen.push(id);
                    (topic_of(&p["topic_id"]), Some(p), id)
                })
            })
            .collect()
    };
    rows.into_iter()
        .map(|(t, p, id)| {
            let t = t.unwrap_or(&Value::Null);
            let p = p.unwrap_or(&Value::Null);
            let thumbnail = pick_thumbnail(&t["thumbnails"], MIN_THUMB_WIDTH)
                .or(t["image_url"].as_str())
                .map(str::to_string)
                .or_else(|| {
                    IMG.captures(p["blurb"].as_str().unwrap_or(""))
                        .map(|c| decode_entities(&c[1]))
                })
                .map(|u| absolute(base, &u));
            let preview = (widest(&t["thumbnails"]) >= MIN_POSTER_SOURCE_WIDTH)
                .then(|| pick_thumbnail(&t["thumbnails"], POSTER_WIDTH))
                .flatten()
                .map(|u| absolute(base, u));
            let category = t["category_id"]
                .as_u64()
                .and_then(|c| slugs.get(&c))
                .cloned();
            let slug = t["slug"].as_str().map(str::to_string);
            json!({
                "id": id,
                "title": t["title"].as_str().or(p["topic_title_headline"].as_str())
                    .map(str::to_string).unwrap_or_else(|| format!("Topic {id}")),
                "createdAt": t["created_at"].as_str().or(p["created_at"].as_str()),
                "likeCount": p["like_count"].as_u64().or(t["like_count"].as_u64()).unwrap_or(0),
                "tags": tag_names(&t["tags"]),
                "thumbnail": thumbnail,
                "preview": preview,
                "avatar": avatar(Some(p)),
                "creator": p["username"].as_str(),
                "paid": category.as_deref() == Some(PAID_SLUG),
                "category": category,
                "url": format!("{base}/t/{}/{id}", slug.unwrap_or_else(|| id.to_string())),
            })
        })
        .collect()
}

pub fn search(cookie: &str, q: &str, page: u32) -> Result<Vec<Value>, Fail> {
    let base = base();
    let data = get_json(
        &format!(
            "{base}/search.json?q={}&page={page}",
            enc(&format!("{q} #scripts"))
        ),
        Some(cookie),
    )?;
    Ok(parse_search(&base, &data, &category_slugs(cookie)))
}

/// A forum upload link as a canonical forum URL, or None for other hosts.
fn upload_url(base: &str, href: &str) -> Option<String> {
    if let Some(rest) = href.strip_prefix("upload://") {
        return Some(format!("{base}/uploads/short-url/{rest}"));
    }
    let url = absolute(base, href);
    let path = url.strip_prefix(base)?.split(['?', '#']).next()?;
    path.starts_with("/uploads/")
        .then(|| format!("{base}{path}"))
}

fn url_file_name(url: &str) -> String {
    let last = url.rsplit('/').next().unwrap_or("");
    let bytes = last.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = last
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_script_file(name: &str) -> bool {
    let l = name.to_lowercase();
    l.ends_with(".funscript") || l.ends_with(".zip")
}

/// Script links in one post's HTML (`parsePostLinks`): forum uploads
/// (`.funscript` / `.zip`), pixeldrain `(kind, id)`s, off-forum hosts.
pub type PostLinks = (
    Vec<(String, String)>,
    Vec<(String, String)>,
    Vec<(String, String)>,
);

pub fn parse_post_links(base: &str, html: &str) -> PostLinks {
    static A: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?is)<a\s([^>]*)>(.*?)</a>"#).unwrap());
    static HREF: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)(?:^|\s)href\s*=\s*(?:"([^"]*)"|'([^']*)')"#).unwrap());
    static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
    static EXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\.[a-z0-9]+$").unwrap());
    static PIXELDRAIN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)https?://(?:www\.)?pixeldrain\.(?:com|net)/(u|l)/([A-Za-z0-9]+)").unwrap()
    });
    static EXTERNAL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)href="(https?://[^"]+)""#).unwrap());
    let mut attachments: Vec<(String, String)> = Vec::new();
    for m in A.captures_iter(html) {
        let Some(h) = HREF.captures(&m[1]) else {
            continue;
        };
        let href = decode_entities(h.get(1).or(h.get(2)).map_or("", |x| x.as_str()));
        let Some(url) = upload_url(base, &href) else {
            continue;
        };
        if attachments.iter().any(|(_, u)| *u == url) {
            continue;
        }
        let file = url_file_name(&url);
        if !is_script_file(&file) && EXT.is_match(&file) {
            continue;
        }
        let text = decode_entities(TAG.replace_all(&m[2], "").trim());
        let name = if is_script_file(&text) { text } else { file };
        if is_script_file(&name) {
            attachments.push((name, url));
        }
    }
    let pixeldrain = PIXELDRAIN
        .captures_iter(html)
        .map(|c| (c[1].to_lowercase(), c[2].to_string()))
        .collect();
    let mut external: Vec<(String, String)> = Vec::new();
    for c in EXTERNAL.captures_iter(html) {
        let url = decode_entities(&c[1]);
        let host = url
            .split("://")
            .nth(1)
            .and_then(|r| r.split(['/', '?', '#', ':']).next())
            .unwrap_or("")
            .trim_start_matches("www.")
            .to_lowercase();
        if let Some((_, label)) = EXTERNAL_HOSTS
            .iter()
            .find(|(h, _)| host == *h || host.ends_with(&format!(".{h}")))
            && !external.iter().any(|(_, u)| *u == url)
        {
            external.push((label.to_string(), url));
        }
    }
    (attachments, pixeldrain, external)
}

/// The funscripts behind pixeldrain file / list links, as page URLs (no
/// forum cookie goes there; pixeldrain refuses programmatic downloads).
fn resolve_pixeldrain(links: &[(String, String, Value)]) -> Vec<Value> {
    let api = env_or("FUNSYNC_PIXELDRAIN_URL", "https://pixeldrain.com");
    let mut seen = Vec::new();
    let mut found: Vec<Value> = Vec::new();
    for (kind, id, from) in links {
        if seen.contains(&(kind, id)) {
            continue;
        }
        seen.push((kind, id));
        if seen.len() > MAX_PIXELDRAIN_LOOKUPS {
            break;
        }
        let url = if kind == "l" {
            format!("{api}/api/list/{id}")
        } else {
            format!("{api}/api/file/{id}/info")
        };
        let Ok(info) = get_json(&url, None) else {
            continue;
        };
        let files = if kind == "l" {
            info["files"].as_array().cloned().unwrap_or_default()
        } else {
            vec![info]
        };
        for f in files {
            let (Some(fid), Some(name)) = (f["id"].as_str(), f["name"].as_str()) else {
                continue;
            };
            let page = format!("https://pixeldrain.com/u/{fid}");
            if !name.to_lowercase().ends_with(".funscript")
                || found.iter().any(|x| x["url"] == page)
            {
                continue;
            }
            let mut row = from.clone();
            row["name"] = json!(name);
            row["url"] = json!(page);
            row["host"] = json!("pixeldrain");
            found.push(row);
        }
    }
    found
}

/// The topic's poster-size picture (`topicPreviewUrl`): its representative
/// image when big enough, else the first post's first wide-enough image.
fn topic_preview(base: &str, data: &Value) -> Option<String> {
    static IMG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<img\s([^>]*)>").unwrap());
    static ATTR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)(src|width|class)="([^"]*)""#).unwrap());
    let representative =
        pick_thumbnail(&data["thumbnails"], POSTER_WIDTH).or(data["image_url"].as_str());
    if let Some(r) = representative
        && widest(&data["thumbnails"]) >= MIN_POSTER_SOURCE_WIDTH
    {
        return Some(absolute(base, r));
    }
    let cooked = data["post_stream"]["posts"][0]["cooked"]
        .as_str()
        .unwrap_or("");
    let mut first = None;
    for m in IMG.captures_iter(cooked) {
        let attrs: HashMap<String, String> = ATTR
            .captures_iter(&m[1])
            .map(|c| (c[1].to_lowercase(), decode_entities(&c[2])))
            .collect();
        let (Some(src), class) = (attrs.get("src"), attrs.get("class").map_or("", |c| c)) else {
            continue;
        };
        if class.contains("emoji") || class.contains("avatar") || class.contains("site-icon") {
            continue;
        }
        let url = absolute(base, src);
        first.get_or_insert(url.clone());
        if attrs
            .get("width")
            .and_then(|w| w.parse::<u64>().ok())
            .is_some_and(|w| w >= MIN_POSTER_SOURCE_WIDTH)
        {
            return Some(url);
        }
    }
    representative.map(|r| absolute(base, r)).or(first)
}

/// Every script in a topic (`getTopicAttachments`): the author's first,
/// then other posters', each in post order.
pub fn topic(cookie: &str, id: u64) -> Result<Value, Fail> {
    let base = base();
    let data = get_json(&format!("{base}/t/{id}.json"), Some(cookie))?;
    let mut posts: Vec<Value> = data["post_stream"]["posts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let have: Vec<u64> = posts.iter().filter_map(|p| p["id"].as_u64()).collect();
    let missing: Vec<u64> = data["post_stream"]["stream"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .filter(|i| !have.contains(i))
        .take(MAX_POSTS.saturating_sub(posts.len()))
        .collect();
    for batch in missing.chunks(POST_BATCH) {
        let ids: Vec<String> = batch.iter().map(|i| format!("post_ids[]={i}")).collect();
        // A missing batch only hides replies.
        if let Ok(v) = get_json(
            &format!("{base}/t/{id}/posts.json?{}", ids.join("&")),
            Some(cookie),
        ) {
            posts.extend(
                v["post_stream"]["posts"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            );
        }
    }
    posts.sort_by_key(|p| p["post_number"].as_u64().unwrap_or(0));
    let author = posts
        .first()
        .map(|p| p["username"].clone())
        .unwrap_or_default();
    let (mut attachments, mut pixeldrain, mut external) = (Vec::new(), Vec::new(), Vec::new());
    for p in &posts {
        let from = json!({"postNumber": p["post_number"], "username": p["username"],
            "byAuthor": p["username"] == author});
        let (a, px, ext) = parse_post_links(&base, p["cooked"].as_str().unwrap_or(""));
        for (name, url) in a {
            if !attachments.iter().any(|x: &Value| x["url"] == url) {
                let mut row = from.clone();
                row["name"] = json!(name);
                row["url"] = json!(url);
                row["host"] = json!("eroscripts");
                attachments.push(row);
            }
        }
        pixeldrain.extend(px.into_iter().map(|(k, i)| (k, i, from.clone())));
        for (host, url) in ext {
            if !external.iter().any(|x: &Value| x["url"] == url) {
                external.push(json!({"host": host, "url": url}));
            }
        }
    }
    let mut linked = resolve_pixeldrain(&pixeldrain);
    let author_first = |a: &Value, b: &Value| {
        (b["byAuthor"] == true)
            .cmp(&(a["byAuthor"] == true))
            .then(a["postNumber"].as_u64().cmp(&b["postNumber"].as_u64()))
    };
    attachments.sort_by(author_first);
    linked.sort_by(author_first);
    let paid = data["category_id"]
        .as_u64()
        .and_then(|c| category_slugs(cookie).get(&c).cloned())
        .is_some_and(|s| s == PAID_SLUG);
    Ok(json!({
        "id": id,
        "title": data["title"],
        "attachments": attachments,
        "linked": linked,
        "external": external,
        "paid": paid,
        "preview": topic_preview(&base, &data),
    }))
}

/// Where a downloaded script goes next to `video`: `<stem>.funscript`, an
/// axis keeps its suffix (`<stem>.twist.funscript`), a variant is
/// `<stem> (<label>).funscript`. None for a name that isn't a funscript.
pub fn script_target(video: &Path, name: &str, variant: Option<&str>) -> Option<PathBuf> {
    let (_, axis) = script_axis(name)?;
    let stem = video.file_stem()?.to_string_lossy();
    let label = variant.map(str::trim).filter(|l| !l.is_empty());
    let file = match (label, axis) {
        (Some(l), Some(a)) => format!("{stem} ({l}).{a}.funscript"),
        (Some(l), None) => format!("{stem} ({l}).funscript"),
        (None, Some(a)) => format!("{stem}.{a}.funscript"),
        (None, None) => format!("{stem}.funscript"),
    };
    Some(video.with_file_name(file))
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/eroscripts/session", get(session))
        .route("/api/eroscripts/login", post(do_login))
        .route("/api/eroscripts/logout", post(logout))
        .route("/api/eroscripts/search", get(do_search))
        .route("/api/eroscripts/topic/{id}", get(do_topic))
        .route("/api/eroscripts/download", post(download))
        .route("/api/eroscripts/preview", post(preview))
}

fn bad(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({"error": msg.into()}))).into_response()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.expect("blocking task")
}

async fn session(State(st): State<Arc<AppState>>) -> Json<Value> {
    let s = load_session(&st.data_dir);
    Json(
        json!({"loggedIn": s.is_some(), "username": s.map(|s| s.username),
        "ownsLibrary": crate::data::owns_library()}),
    )
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LoginBody {
    username: String,
    password: String,
    code: String,
    cookie: String,
}

async fn do_login(State(st): State<Arc<AppState>>, Json(b): Json<LoginBody>) -> Response {
    let res = blocking(move || {
        if !b.cookie.trim().is_empty() {
            login_with_cookie(&b.cookie).map(Login::In)
        } else {
            login(&b.username, &b.password, b.code.trim())
        }
    })
    .await;
    match res {
        Ok(Login::In(s)) => match save_session(&st.data_dir, Some(&s)) {
            Ok(()) => Json(json!({"ok": true, "username": s.username})).into_response(),
            Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        },
        Ok(Login::NeedsCode) => Json(json!({"ok": false, "needsCode": true})).into_response(),
        Err(Fail::Auth) => bad(StatusCode::UNAUTHORIZED, "That cookie isn't logged in"),
        Err(e) => e.into_response(),
    }
}

async fn logout(State(st): State<Arc<AppState>>) -> Response {
    match save_session(&st.data_dir, None) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

fn cookie(st: &AppState) -> Result<String, Fail> {
    load_session(&st.data_dir)
        .map(|s| s.cookie)
        .ok_or(Fail::Auth)
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default)]
    page: Option<u32>,
}

async fn do_search(State(st): State<Arc<AppState>>, Query(q): Query<SearchQuery>) -> Response {
    let c = match cookie(&st) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    match blocking(move || search(&c, &q.q, q.page.unwrap_or(1))).await {
        Ok(r) => Json(json!({"results": r})).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn do_topic(State(st): State<Arc<AppState>>, UrlPath(id): UrlPath<u64>) -> Response {
    let c = match cookie(&st) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    match blocking(move || topic(&c, id)).await {
        Ok(r) => Json(r).into_response(),
        Err(e) => e.into_response(),
    }
}

#[derive(Deserialize)]
struct DownloadBody {
    #[serde(rename = "videoId")]
    video_id: String,
    url: String,
    name: String,
    #[serde(default)]
    replace: bool,
    #[serde(default)]
    variant: Option<String>,
}

/// Save a forum upload next to its video (only while this app owns the
/// library), then rescan.
async fn download(State(st): State<Arc<AppState>>, Json(b): Json<DownloadBody>) -> Response {
    if !crate::data::owns_library() {
        return bad(
            StatusCode::CONFLICT,
            "The library belongs to the old app for now: scripts are saved only when FunSync in Rust owns the library (FUNSYNC_OWNS_LIBRARY=1).",
        );
    }
    let Some(v) = st.video(&b.video_id).await else {
        return bad(StatusCode::NOT_FOUND, "no such video");
    };
    if !b.url.starts_with(&format!("{}/uploads/", base())) {
        return bad(StatusCode::BAD_REQUEST, "not an EroScripts upload");
    }
    let Some(dest) = script_target(Path::new(&v.path), &b.name, b.variant.as_deref()) else {
        return bad(
            StatusCode::BAD_REQUEST,
            "Only .funscript files can be saved here; open a .zip in the browser",
        );
    };
    if dest.exists() && !b.replace {
        return (
            StatusCode::CONFLICT,
            Json(
                json!({"error": format!("{} already exists: replace it or save as a variant",
                dest.file_name().unwrap_or_default().to_string_lossy()), "exists": true}),
            ),
        )
            .into_response();
    }
    let c = match cookie(&st) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    let part = dest.with_extension("funscript.part");
    let (url, p) = (b.url.clone(), part.clone());
    let got = blocking(move || {
        slr_scout::forum::download_upload(
            &Curl,
            &mut Pacer::new(std::time::Duration::ZERO),
            &c,
            &url,
            &p,
        )
        .map_err(|e| e.to_string())
        .and_then(|_| {
            let bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
            serde_json::from_slice::<Value>(&bytes)
                .map(|_| ())
                .map_err(|_| "the download isn't a funscript (logged out?)".to_string())
        })
    })
    .await;
    if let Err(e) = got.and_then(|_| std::fs::rename(&part, &dest).map_err(|e| e.to_string())) {
        let _ = std::fs::remove_file(&part);
        return bad(StatusCode::BAD_GATEWAY, e);
    }
    let _ = st.rescan().await;
    Json(json!({"ok": true, "path": dest})).into_response()
}

#[derive(Deserialize)]
struct PreviewBody {
    #[serde(rename = "videoId")]
    video_id: String,
    url: String,
}

/// A post's picture as the video's poster: saved in `<data dir>/custom-thumbs/`
/// and recorded in `customThumbnails` (the old app's shape).
async fn preview(State(st): State<Arc<AppState>>, Json(b): Json<PreviewBody>) -> Response {
    let Some(v) = st.video(&b.video_id).await else {
        return bad(StatusCode::NOT_FOUND, "no such video");
    };
    if !(b.url.starts_with(&format!("{}/", base())) || b.url.starts_with(UPLOAD_CDN)) {
        return bad(StatusCode::BAD_REQUEST, "not an EroScripts picture");
    }
    let ext = match url_file_name(&b.url)
        .rsplit_once('.')
        .map(|(_, e)| e.to_lowercase())
    {
        Some(e) if ["jpg", "jpeg", "png", "webp"].contains(&e.as_str()) => e,
        _ => "jpg".into(),
    };
    let dir = st.data_dir.join("custom-thumbs");
    let dest = dir.join(format!(
        "{}-{}.{ext}",
        v.id,
        &format!("{:x}", md5::compute(&b.url))[..8]
    ));
    let (url, d) = (b.url.clone(), dest.clone());
    let got = blocking(move || {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Curl.download(&Request::get(url), &d)
    })
    .await;
    if let Err(e) = got {
        let _ = std::fs::remove_file(&dest);
        return bad(StatusCode::BAD_GATEWAY, e);
    }
    st.library.write().await.lib_mut("customThumbnails")[&v.path] = json!({
        "type": "image", "imagePath": dest, "source": "eroscripts", "url": b.url,
        "setAt": crate::data::now_ms(),
    });
    match st.save().await {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: &str = "https://f.test";

    #[test]
    fn post_links() {
        let html = r#"<p><a class="attachment" href="/uploads/short-url/abc.funscript">Scene.funscript</a>
            <a href="upload://def.funscript">Scene.twist.funscript</a>
            <a href="/uploads/short-url/img.png">Scene.funscript</a>
            <a href="https://f.test/uploads/short-url/pack.zip?dl=1">pack</a>
            <a href="https://other.test/uploads/x.funscript">x</a>
            <a href="https://pixeldrain.com/l/AbC1">list</a>
            <a href="https://www.mega.nz/file/x">mega</a></p>"#;
        let (a, px, ext) = parse_post_links(B, html);
        assert_eq!(
            a,
            [
                (
                    "Scene.funscript".into(),
                    format!("{B}/uploads/short-url/abc.funscript")
                ),
                (
                    "Scene.twist.funscript".into(),
                    format!("{B}/uploads/short-url/def.funscript")
                ),
                ("pack.zip".into(), format!("{B}/uploads/short-url/pack.zip")),
            ]
        );
        assert_eq!(px, [("l".to_string(), "AbC1".to_string())]);
        assert_eq!(
            ext,
            [
                (
                    "pixeldrain".to_string(),
                    "https://pixeldrain.com/l/AbC1".to_string()
                ),
                ("MEGA".to_string(), "https://www.mega.nz/file/x".to_string())
            ]
        );
    }

    #[test]
    fn search_results() {
        let data = json!({
            "topics": [{"id": 7, "title": "Scene", "slug": "scene", "category_id": 15,
                "tags": ["pov", {"name": "vr"}],
                "thumbnails": [{"url": "/t/big.jpg", "width": 1000}, {"url": "/t/mid.jpg", "width": 690}, {"url": "/t/s.jpg", "width": 200}]}],
            "posts": [{"topic_id": 7, "username": "maker", "like_count": 4, "avatar_template": "/a/{size}.png"}],
        });
        let slugs = HashMap::from([(15, PAID_SLUG.to_string())]);
        let r = &parse_search(B, &data, &slugs)[0];
        assert_eq!(r["thumbnail"], format!("{B}/t/mid.jpg"));
        assert_eq!(r["preview"], format!("{B}/t/mid.jpg"));
        assert_eq!(r["paid"], true);
        assert_eq!(r["creator"], "maker");
        assert_eq!(r["likeCount"], 4);
        assert_eq!(r["avatar"], format!("{B}/a/90.png"));
        assert_eq!(r["tags"], json!(["pov", "vr"]));
        assert_eq!(r["url"], format!("{B}/t/scene/7"));
        // Posts only: one result per topic, thumbnail from the blurb.
        let data = json!({"posts": [
            {"topic_id": 9, "username": "a", "blurb": "<img src=\"/x.jpg\">", "topic_title_headline": "Nine"},
            {"topic_id": 9, "username": "b"}]});
        let r = parse_search(B, &data, &HashMap::new());
        assert_eq!(r.len(), 1);
        assert_eq!(r[0]["thumbnail"], format!("{B}/x.jpg"));
        assert_eq!(r[0]["title"], "Nine");
        assert_eq!(r[0]["paid"], false);
    }

    #[test]
    fn targets() {
        let v = Path::new("/lib/A/Scene.mp4");
        let t = |n, var| script_target(v, n, var).map(|p| p.to_string_lossy().into_owned());
        assert_eq!(t("x.funscript", None).unwrap(), "/lib/A/Scene.funscript");
        assert_eq!(
            t("x.roll.funscript", None).unwrap(),
            "/lib/A/Scene.roll.funscript"
        );
        assert_eq!(
            t("x.funscript", Some("bob")).unwrap(),
            "/lib/A/Scene (bob).funscript"
        );
        assert_eq!(
            t("x.R0.funscript", Some("bob")).unwrap(),
            "/lib/A/Scene (bob).twist.funscript"
        );
        assert_eq!(t("x.zip", None), None);
    }

    #[test]
    fn session_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let s = Session {
            cookie: "c".into(),
            username: "u".into(),
        };
        save_session(d.path(), Some(&s)).unwrap();
        let mode = std::fs::metadata(d.path().join(SESSION_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(load_session(d.path()), Some(s));
        save_session(d.path(), None).unwrap();
        assert_eq!(load_session(d.path()), None);
    }
}
