//! The HTTP side: one request at a time (big bodies are sent on their own thread, see handle).
//! `GET /` (redirects to the deck page),
//! `GET /swipe` (the deck page), `GET /downloads` and `GET /api/downloads` (every right swipe:
//! waiting, downloaded lately or turned down, and the download job, crate::downloads),
//! `GET /api/deck?stack=multi|vr|flat|slr|free|dupes&limit=N&seed=N&jitter=N&exclude=<source>:<id>,…`
//! (the next cards of a tab, every source mixed, crate::deck; `free` is the Free space tab's
//! library videos, crate::free_space; `dupes` the Duplicates tab's pairs, crate::duplicates),
//! `GET /api/drive` (the library drive's free space and whether swiping is off, crate::drive),
//! `GET /frames/<eroscripts|slr>/<id>/<25|50|75>.jpg` (the scouts' preview frames,
//! crate::frames), `GET /pictures/<slug>/<file>` (the creators' pictures,
//! crate::creator_pictures), `POST /api/swipe {id, dir}` and `POST /api/swipe/undo {id}` (waiting
//! posts), `POST /api/scenes/<slr|vrb|pmvhaven|faptap>/swipe {id, dir}` and `…/undo {id}`
//! (that scout's scenes), `POST /api/scenes/creators/swipe {id, dir}` and `…/undo {id}` (a yes
//! creator's post, `<kind>-<id>`, into creator-swipes.json, crate::creators),
//! `POST /api/free-space/swipe {id, dir}`, `…/undo {id}` (marks on
//! library videos), `POST /api/free-space/delete` (rates the left-marked ones for deletion
//! through the app's backend), `POST /api/duplicates/swipe {id, dir: left|right|up}`,
//! `…/undo {id}` (marks on pairs) and `POST /api/duplicates/delete` (the app deletes each
//! marked copy as a duplicate), `POST /api/slr-priority {first: [key…]}` (the SLR tab's queue:
//! the keys put ahead of the score order, crate::slr_queue). The swipes files and
//! slr-priority.json are the only writes, crate::swipes, crate::creators and crate::slr_queue.
//! Post and
//! scene swipes are refused (507) while the library drive is full; undos and the Free space
//! and Duplicates tabs' marks never are. In front of them all, crate::gate: the access PIN,
//! and `/login` and `/logout`.

use crate::gate::{self, Gate};
use crate::scenes::{self, Source};
use crate::swipes::{self, Dir, Store};
use crate::{
    creator_pictures, creators, deck, downloads, drive, duplicates, frames, free_space, report,
    slr_queue,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

/// The picture lookahead both sites' pages share; see preload.js.
pub const PRELOAD_JS: &str = include_str!("preload.js");
/// The line of a page's script that becomes PRELOAD_JS.
const PRELOAD_MARK: &str = "// @preload.js";

/// `page` with PRELOAD_JS inlined at its `// @preload.js` line, so each page stays one
/// self-contained file (no external scripts).
pub fn with_preloader(page: &str) -> String {
    debug_assert!(page.contains(PRELOAD_MARK), "page lacks {PRELOAD_MARK}");
    page.replacen(PRELOAD_MARK, PRELOAD_JS, 1)
}

/// The swipe deck page: HTML, CSS and JS in one file, no external scripts.
pub static SWIPE_PAGE: LazyLock<String> =
    LazyLock::new(|| with_preloader(include_str!("swipe.html")));

/// The Downloads page, likewise self-contained.
pub const DOWNLOADS_PAGE: &str = include_str!("downloads.html");

/// This site's name (log lines) and its pages, for the gate.
pub const NAME: &str = "scout-report";
pub const PAGES: &[&str] = &["/", "/swipe", "/downloads"];

/// Longest request head read before giving up on a request.
const MAX_HEAD: usize = 16 * 1024;
/// Longest request body accepted (a swipe is a few dozen bytes).
const MAX_BODY: usize = 4 * 1024;
/// Bodies at least this big go out on their own thread (see handle).
const BACKGROUND_WRITE: usize = 256 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    /// `Cache-Control`.
    pub cache: &'static str,
    pub body: Vec<u8>,
    /// Extra headers (`Location`, `Set-Cookie`, `Retry-After`).
    pub headers: Vec<(&'static str, String)>,
}

/// Everything but the pictures: rebuilt from the files on every request.
const NO_STORE: &str = "no-store";
/// Frames and pictures (both sites) change only when a scout takes them again: a day, and
/// `immutable` so a page reload (Safari revalidates on reload, and these have no validators)
/// doesn't fetch the multi-MB card GIFs again either.
pub const PICTURE_CACHE: &str = "max-age=86400, immutable";

pub fn text(status: u16, body: &str) -> Response {
    Response {
        status,
        content_type: "text/plain; charset=utf-8",
        cache: NO_STORE,
        body: body.as_bytes().to_vec(),
        headers: Vec::new(),
    }
}

pub fn html(page: &str) -> Response {
    Response {
        status: 200,
        content_type: "text/html; charset=utf-8",
        cache: NO_STORE,
        body: page.as_bytes().to_vec(),
        headers: Vec::new(),
    }
}

pub fn json_ok<T: Serialize>(value: &T) -> Response {
    match serde_json::to_vec(value) {
        Ok(body) => Response {
            status: 200,
            content_type: "application/json",
            cache: NO_STORE,
            body,
            headers: Vec::new(),
        },
        Err(e) => text(500, &format!("json: {e}\n")),
    }
}

pub fn json(status: u16, value: &Value) -> Response {
    Response {
        status,
        ..json_ok(value)
    }
}

/// `GET /frames/…`: the frame file (crate::frames::file_for), else 404.
fn frame(path: &str, user_data: &Path) -> Response {
    match frames::file_for(user_data, path).map(std::fs::read) {
        Some(Ok(body)) => Response {
            status: 200,
            content_type: "image/jpeg",
            cache: PICTURE_CACHE,
            body,
            headers: Vec::new(),
        },
        _ => text(404, "Not found\n"),
    }
}

/// Answer one request. HEAD gets GET's headers (the caller drops the body).
pub fn route(method: &str, target: &str, body: &[u8], user_data: &Path) -> Response {
    let target = target.split('#').next().unwrap_or("");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match (method, path) {
        ("GET" | "HEAD", "/") => {
            let mut resp = text(303, "");
            resp.headers.push(("Location", "/swipe".into()));
            resp
        }
        ("GET" | "HEAD", "/swipe") => html(&SWIPE_PAGE),
        ("GET" | "HEAD", "/api/drive") => json_ok(&drive::status(user_data)),
        ("GET" | "HEAD", "/downloads") => html(DOWNLOADS_PAGE),
        ("GET" | "HEAD", "/api/downloads") => json_ok(&downloads::load(user_data)),
        ("POST", "/api/slr-priority") => slr_priority(body, user_data),
        ("GET" | "HEAD", "/api/deck") => match deal(query) {
            Ok(d) if d.stack == deck::Stack::Free => {
                json_ok(&free_space::load(user_data, d.limit, &d.exclude))
            }
            Ok(d) if d.stack == deck::Stack::Dupes => {
                json_ok(&duplicates::load(user_data, d.limit, &d.exclude))
            }
            Ok(d) => json_ok(&deck::load(user_data, &d)),
            Err(e) => text(400, &e),
        },
        ("GET" | "HEAD", p) if p.starts_with("/frames/") => frame(p, user_data),
        ("GET" | "HEAD", p) if p.starts_with("/pictures/") => {
            match creator_pictures::file_for(user_data, p)
                .map(|(file, kind)| (std::fs::read(file), kind))
            {
                Some((Ok(body), content_type)) => Response {
                    status: 200,
                    content_type,
                    cache: PICTURE_CACHE,
                    body,
                    headers: Vec::new(),
                },
                _ => text(404, "Not found\n"),
            }
        }
        ("GET" | "HEAD", p) if p.starts_with("/dupes-frames/") => {
            match duplicates::frame_image(user_data, p) {
                Ok(body) => Response {
                    status: 200,
                    content_type: "image/jpeg",
                    cache: PICTURE_CACHE,
                    body,
                    headers: Vec::new(),
                },
                Err((status, e)) => text(status, &format!("{e}\n")),
            }
        }
        ("POST", "/api/swipe") => swipe(body, user_data, None),
        ("POST", "/api/swipe/undo") => undo(body, user_data, None),
        ("POST", "/api/scenes/creators/swipe") => creator_swipe(body, user_data),
        ("POST", "/api/scenes/creators/swipe/undo") => creator_undo(body, user_data),
        ("POST", p) if p.starts_with("/api/scenes/") => {
            let action = p["/api/scenes/".len()..]
                .split_once('/')
                .and_then(|(src, action)| Some((Source::parse(src)?, action)));
            match action {
                Some((src, "swipe")) => swipe(body, user_data, Some(src)),
                Some((src, "swipe/undo")) => undo(body, user_data, Some(src)),
                _ => text(404, "Not found\n"),
            }
        }
        ("POST", "/api/free-space/swipe") => free_swipe(body, user_data),
        ("POST", "/api/free-space/swipe/undo") => match free_id(body) {
            Ok(id) => match free_space::SWIPES.undo(user_data, &id) {
                Ok(removed) => json_ok(&json!({"id": id, "removed": removed})),
                Err(e) => text(500, &format!("{e}\n")),
            },
            Err(resp) => resp,
        },
        ("POST", "/api/free-space/delete") => match free_space::delete(user_data) {
            Ok(v) => json_ok(&v),
            Err((status, e)) => text(status, &format!("{e}\n")),
        },
        ("POST", "/api/duplicates/swipe") => dupes_swipe(body, user_data),
        ("POST", "/api/duplicates/swipe/undo") => match dupes_request(body) {
            Ok((_, id)) => match duplicates::Marks::undo(user_data, &id) {
                Ok(removed) => json_ok(&json!({"id": id, "removed": removed})),
                Err(e) => text(500, &format!("{e}\n")),
            },
            Err(resp) => resp,
        },
        ("POST", "/api/duplicates/delete") => match duplicates::delete(user_data) {
            Ok(v) => json_ok(&v),
            Err((status, e)) => text(status, &format!("{e}\n")),
        },
        _ => text(404, "Not found\n"),
    }
}

/// The deck request from the query: `limit=N` (default DEFAULT_LIMIT, at most MAX_LIMIT),
/// `seed=N` (shuffle a little), `jitter=N` (how much, in score points: 0 ..= MAX_JITTER;
/// default the tab's), `exclude=<source>:<id>,…` (the cards the page holds: `eroscripts:<topic
/// id>`, a scene source's `<source>:<id>` or `library:<video id>`; at most MAX_EXCLUDE), and
/// `stack=multi|vr|flat|slr|free|dupes` (default multi).
fn deal(query: &str) -> Result<deck::Deal, String> {
    let mut d = deck::Deal::default();
    for (key, v) in query.split('&').filter_map(|p| p.split_once('=')) {
        match key {
            "limit" => {
                d.limit = v
                    .parse::<usize>()
                    .map(|n| n.min(deck::MAX_LIMIT))
                    .map_err(|_| format!("limit must be a whole number, got {v:?}\n"))?;
            }
            "seed" => {
                d.seed = Some(
                    v.parse::<u64>()
                        .map_err(|_| format!("seed must be a whole number, got {v:?}\n"))?,
                );
            }
            "jitter" => {
                let n = v
                    .parse::<f64>()
                    .ok()
                    .filter(|n| (0.0..=deck::MAX_JITTER).contains(n))
                    .ok_or_else(|| {
                        format!(
                            "jitter must be a number from 0 to {}, got {v:?}\n",
                            deck::MAX_JITTER
                        )
                    })?;
                d.jitter = Some(n);
            }
            "exclude" => {
                for item in v.split(',').filter(|s| !s.is_empty()) {
                    d.exclude.insert(exclude_key(item).ok_or_else(|| {
                        format!("exclude takes <source>:<id> items, got {item:?}\n")
                    })?);
                }
                if d.exclude.len() > deck::MAX_EXCLUDE {
                    return Err(format!(
                        "exclude takes at most {} items\n",
                        deck::MAX_EXCLUDE
                    ));
                }
            }
            "stack" => {
                d.stack = deck::Stack::parse(v).ok_or_else(|| {
                    format!("stack is multi, vr, flat, slr, free or dupes, got {v:?}\n")
                })?;
            }
            _ => {}
        }
    }
    Ok(d)
}

/// One `exclude` item as a card key (deck::DeckCard::key): `eroscripts:<topic id>` (plain
/// digits, written back the way the deck writes it), `<scene source>:<id>`,
/// `library:<video id>` (swipes::is_id) or `dupes:<id>-<id>` (duplicates::parse_pair).
fn exclude_key(item: &str) -> Option<String> {
    let (source, id) = item.split_once(':')?;
    if source == deck::EROSCRIPTS {
        let id: u64 = id
            .parse()
            .ok()
            .filter(|_| id.bytes().all(|b| b.is_ascii_digit()))?;
        return Some(format!("{source}:{id}"));
    }
    if source == duplicates::SOURCE {
        return duplicates::parse_pair(id).map(|_| item.to_string());
    }
    let known = Source::parse(source).is_some()
        || source == free_space::SOURCE
        || source == creators::SOURCE;
    (known && swipes::is_id(id)).then(|| item.to_string())
}

/// The JSON object a POST carries, and its `id` as the swipes key: a topic id (number or
/// numeric string) for posts, a scene id (swipes::id_key) for `source`'s scenes.
fn swipe_request(body: &[u8], source: Option<Source>) -> Result<(Value, String), Response> {
    let req: Value = serde_json::from_slice(body)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| text(400, "body must be a JSON object\n"))?;
    let id = req.get("id");
    let id = match source {
        None => id.and_then(report::id_of).map(|n| n.to_string()),
        Some(_) => id.and_then(swipes::id_key),
    }
    .ok_or_else(|| text(400, "id must be a topic or scene id\n"))?;
    Ok((req, id))
}

/// A Free space POST's JSON object and its `id`: a backend video id (swipes::id_key).
fn free_request(body: &[u8]) -> Result<(Value, String), Response> {
    let req: Value = serde_json::from_slice(body)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| text(400, "body must be a JSON object\n"))?;
    let id = req
        .get("id")
        .and_then(swipes::id_key)
        .ok_or_else(|| text(400, "id must be a video id\n"))?;
    Ok((req, id))
}

fn free_id(body: &[u8]) -> Result<String, Response> {
    free_request(body).map(|(_, id)| id)
}

/// `POST /api/slr-priority {first: [key…]}`: the SLR keys to put first, in order.
fn slr_priority(body: &[u8], user_data: &Path) -> Response {
    let first: Option<Vec<String>> = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.get("first").cloned())
        .and_then(|f| serde_json::from_value(f).ok());
    let Some(first) = first else {
        return text(400, "body must be {\"first\": [\"<source>:<id>\", …]}\n");
    };
    if let Err(e) = slr_queue::check(&first) {
        return text(400, &format!("{e}\n"));
    }
    match slr_queue::write(user_data, first) {
        Ok(p) => json_ok(&p),
        Err(e) => text(500, &format!("{e}\n")),
    }
}

/// `POST /api/free-space/swipe {id, dir}`: mark a library video (left = delete, right = keep).
fn free_swipe(body: &[u8], user_data: &Path) -> Response {
    let (req, id) = match free_request(body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    let Some(dir) = req.get("dir").and_then(Value::as_str).and_then(Dir::parse) else {
        return text(400, "dir must be \"right\" or \"left\"\n");
    };
    match free_space::swipe(user_data, &id, dir) {
        Ok(v) => json_ok(&v),
        Err((status, e)) => text(status, &format!("{e}\n")),
    }
}

/// A Duplicates POST's JSON object and its `id`: a pair (`<id>-<id>`, the smaller id first).
fn dupes_request(body: &[u8]) -> Result<(Value, String), Response> {
    let req: Value = serde_json::from_slice(body)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| text(400, "body must be a JSON object\n"))?;
    let id = req
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| duplicates::parse_pair(s).is_some())
        .map(str::to_string)
        .ok_or_else(|| text(400, "id must be a pair <id>-<id>, the smaller id first\n"))?;
    Ok((req, id))
}

/// `POST /api/duplicates/swipe {id, dir}`: left = keep the left video, right = keep the right
/// one, up = not duplicates.
fn dupes_swipe(body: &[u8], user_data: &Path) -> Response {
    let (req, id) = match dupes_request(body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    let Some(dir) = req
        .get("dir")
        .and_then(Value::as_str)
        .and_then(duplicates::Dir::parse)
    else {
        return text(400, "dir must be \"left\", \"right\" or \"up\"\n");
    };
    match duplicates::swipe(user_data, &id, dir) {
        Ok(v) => json_ok(&v),
        Err((status, e)) => text(status, &format!("{e}\n")),
    }
}

fn store(source: Option<Source>) -> Store {
    source.map_or(swipes::EROSCRIPTS, Source::swipes)
}

/// `POST /api/swipe {id, dir}` (a waiting post, `source` None) or
/// `POST /api/scenes/<source>/swipe {id, dir}` (a scene in that scout's file): record a swipe
/// in that source's swipes file.
fn swipe(body: &[u8], user_data: &Path, source: Option<Source>) -> Response {
    let (req, id) = match swipe_request(body, source) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    let Some(dir) = req.get("dir").and_then(Value::as_str).and_then(Dir::parse) else {
        return text(400, "dir must be \"right\" or \"left\"\n");
    };
    let mut problems = Vec::new();
    let known = match source {
        Some(src) => {
            let file = report::read_json(user_data, src.file(), &mut problems);
            scenes::is_scene(src, &file, &id)
                .then_some(())
                .ok_or(format!("is not in {}", src.file()))
        }
        None => {
            let waiting = report::read_json(user_data, report::WAITING_FILE, &mut problems);
            id.parse()
                .is_ok_and(|n| deck::is_waiting(&waiting, n))
                .then_some(())
                .ok_or_else(|| "is not on the waiting list".to_string())
        }
    };
    if !problems.is_empty() {
        return text(500, &format!("{}\n", problems.join("\n")));
    }
    if let Err(why) = known {
        return text(400, &format!("{id} {why}\n"));
    }
    if let Some(refused) = refuse_when_full(&drive::status(user_data)) {
        return refused;
    }
    match store(source).set(user_data, &id, dir, video_tags::timefmt::now_iso()) {
        Ok(s) => json_ok(&json!({"id": id_json(&id, source), "dir": s.dir, "at": s.at})),
        Err(e) => text(500, &format!("{e}\n")),
    }
}

/// 507 while the library drive is full (drive::Status::full): a right swipe would queue a
/// download with no room for it, so the download tabs take no swipes until space is freed.
fn refuse_when_full(status: &drive::Status) -> Option<Response> {
    let (Some(d), true) = (&status.drive, status.full) else {
        return None;
    };
    let gb = |b: u64| format!("{:.1} GB", b as f64 / 1e9);
    Some(text(
        507,
        &format!(
            "The library drive is full: {} free on {}, swiping stops under {}. Delete videos (the Free space tab) to swipe again.\n",
            gb(d.free),
            d.mount,
            gb(status.min_free_bytes.unwrap_or(0))
        ),
    ))
}

/// The id as the answer gives it back: a post's as a number (as its card has it), a
/// scene's as a string.
fn id_json(id: &str, source: Option<Source>) -> Value {
    match (source, id.parse::<u64>()) {
        (None, Ok(n)) => json!(n),
        _ => json!(id),
    }
}

/// `POST /api/swipe/undo {id}` or `/api/scenes/<source>/swipe/undo {id}`: forget a swipe in
/// that source's file (any id; `removed` says whether there was one).
fn undo(body: &[u8], user_data: &Path, source: Option<Source>) -> Response {
    let id = match swipe_request(body, source) {
        Ok((_, id)) => id,
        Err(resp) => return resp,
    };
    match store(source).undo(user_data, &id) {
        Ok(removed) => json_ok(&json!({"id": id_json(&id, source), "removed": removed})),
        Err(e) => text(500, &format!("{e}\n")),
    }
}

/// A creator card POST's JSON object, its card `id` and that id's post key (`<kind>:<id>`).
fn creator_request(body: &[u8]) -> Result<(Value, String, String), Response> {
    let req: Value = serde_json::from_slice(body)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| text(400, "body must be a JSON object\n"))?;
    let (id, key) = req
        .get("id")
        .and_then(swipes::id_key)
        .and_then(|id| creators::post_key(&id).map(|key| (id, key)))
        .ok_or_else(|| text(400, "id must be <eroscripts|faptap>-<id>\n"))?;
    Ok((req, id, key))
}

/// `POST /api/scenes/creators/swipe {id, dir}`: a yes creator's post into creator-swipes.json
/// (right = want, left = hide), refused (507) while the library drive is full.
fn creator_swipe(body: &[u8], user_data: &Path) -> Response {
    let (req, id, key) = match creator_request(body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    let Some(dir) = req.get("dir").and_then(Value::as_str).and_then(Dir::parse) else {
        return text(400, "dir must be \"right\" or \"left\"\n");
    };
    let mut problems = Vec::new();
    let all = creators::Creators::read(user_data, &mut problems);
    if !problems.is_empty() {
        return text(500, &format!("{}\n", problems.join("\n")));
    }
    let Some(who) = all.owner(&key) else {
        return text(400, &format!("{key} is not a post of a yes creator\n"));
    };
    if let Some(refused) = refuse_when_full(&drive::status(user_data)) {
        return refused;
    }
    let at = video_tags::timefmt::now_iso();
    match creators::set_post(user_data, &key, &who.slug, dir, at) {
        Ok(at) => json_ok(&json!({"id": id, "dir": dir, "at": at})),
        Err(e) => text(500, &format!("{e}\n")),
    }
}

/// `POST /api/scenes/creators/swipe/undo {id}`: forget a creator post's swipe.
fn creator_undo(body: &[u8], user_data: &Path) -> Response {
    let (id, key) = match creator_request(body) {
        Ok((_, id, key)) => (id, key),
        Err(resp) => return resp,
    };
    match creators::undo_post(user_data, &key) {
        Ok(removed) => json_ok(&json!({"id": id, "removed": removed})),
        Err(e) => text(500, &format!("{e}\n")),
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        303 => "See Other",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        413 => "Content Too Large",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        507 => "Insufficient Storage",
        _ => "Internal Server Error",
    }
}

/// The request body: `Content-Length` bytes (none without the header), the
/// part already read with the head first, the rest from `more`. Over
/// MAX_BODY → 413 (not read); a bad length or a short body → 400.
fn read_body(head: &str, rest: &[u8], more: &mut impl Read) -> Result<Vec<u8>, Response> {
    let mut len = None;
    for line in head.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("content-length") {
            continue;
        }
        let n: usize = value
            .trim()
            .parse()
            .map_err(|_| text(400, "Bad Content-Length\n"))?;
        if len.is_some_and(|l| l != n) {
            return Err(text(400, "Conflicting Content-Length\n"));
        }
        len = Some(n);
    }
    let len = len.unwrap_or(0);
    if len > MAX_BODY {
        return Err(text(413, "Body over 4 KB\n"));
    }
    let mut body = rest[..rest.len().min(len)].to_vec();
    let start = body.len();
    body.resize(len, 0);
    more.read_exact(&mut body[start..])
        .map_err(|_| text(400, "Body shorter than its Content-Length\n"))?;
    Ok(body)
}

/// Bind `127.0.0.1:<port>` (0 = any free port), or every interface (`0.0.0.0`) when `lan`.
pub fn bind(port: u16, lan: bool) -> io::Result<TcpListener> {
    let ip = if lan {
        Ipv4Addr::UNSPECIFIED
    } else {
        Ipv4Addr::LOCALHOST
    };
    TcpListener::bind((ip, port))
}

/// The Mac's address on the home network: the source address the OS would use to reach
/// a private address (a UDP `connect` sends nothing). None without a network.
pub fn lan_address() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket
        .connect((Ipv4Addr::new(192, 168, 255, 255), 9))
        .ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

/// Serve scout-report's routes behind its gate, forever.
pub fn serve(listener: &TcpListener, user_data: &Path, err: &mut dyn Write) {
    serve_site(listener, user_data, Gate::new(NAME, PAGES), err, |r| {
        route(r.method, r.target, r.body, user_data)
    });
}

/// Serve requests one after another, forever: each through `gate`, then `route`.
/// Connection problems are logged under the gate's site name.
pub fn serve_site(
    listener: &TcpListener,
    user_data: &Path,
    mut gate: Gate,
    err: &mut dyn Write,
    mut route: impl FnMut(&gate::Request) -> Response,
) {
    for stream in listener.incoming() {
        let result = stream.and_then(|s| handle(s, user_data, &mut gate, err, &mut route));
        if let Err(e) = result {
            // A client that went away or timed out; keep serving.
            let _ = writeln!(err, "{}: {e}", gate.name());
        }
    }
}

/// Read the request (head, then body), pass it through the gate and write the response.
fn handle(
    mut stream: TcpStream,
    user_data: &Path,
    gate: &mut Gate,
    err: &mut dyn Write,
    route: &mut impl FnMut(&gate::Request) -> Response,
) -> io::Result<()> {
    let client = stream.peer_addr()?.ip();
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        let n = stream.read(&mut buf)?;
        if n == 0 || raw.len() + n > MAX_HEAD {
            break raw.len();
        }
        raw.extend_from_slice(&buf[..n]);
    };
    let head = String::from_utf8_lossy(&raw[..head_end]);
    let rest = raw.get(head_end + 4..).unwrap_or(&[]);
    let line = head.lines().next().unwrap_or("");
    let mut parts = line.split_whitespace();
    let (method, target) = match (parts.next(), parts.next(), parts.next()) {
        (Some(m), Some(t), Some(v)) if v.starts_with("HTTP/") => (m, t),
        _ => ("", ""),
    };
    let resp = if method.is_empty() {
        text(400, "Bad request\n")
    } else {
        match read_body(&head, rest, &mut stream) {
            Ok(body) => {
                let (cookie, accept) = cookie_and_accept(&head);
                let req = gate::Request {
                    method,
                    target,
                    body: &body,
                    client,
                    cookie: &cookie,
                    accept: &accept,
                };
                gate.respond(&req, user_data, video_tags::access::now_secs(), err, route)
            }
            Err(resp) => resp,
        }
    };
    let mut extra = String::new();
    for (name, value) in &resp.headers {
        extra.push_str(&format!("{name}: {value}\r\n"));
    }
    let reply_head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: {}\r\n{extra}X-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        resp.status,
        reason(resp.status),
        resp.content_type,
        resp.body.len(),
        resp.cache
    );
    if method != "HEAD" && resp.body.len() >= BACKGROUND_WRITE {
        // A card picture (forum GIFs run to 8 MB) takes a phone a second or more to receive;
        // sent from here it would hold up every request behind it, the other pictures the
        // page is fetching ahead and the next deal included. The response is complete, so
        // only the sending moves off the request loop; a client that goes away just ends it.
        std::thread::spawn(move || {
            let _ = stream
                .write_all(reply_head.as_bytes())
                .and_then(|()| stream.write_all(&resp.body))
                .and_then(|()| stream.flush());
        });
        return Ok(());
    }
    stream.write_all(reply_head.as_bytes())?;
    if method != "HEAD" {
        stream.write_all(&resp.body)?;
    }
    stream.flush()?;
    if resp.status == 413 {
        // The body was never read: drain what the client sends (until it hangs up, the
        // read timeout or 1 MB) so closing with unread data doesn't reset the connection
        // before the client sees the answer.
        stream.shutdown(Shutdown::Write)?;
        let _ = io::copy(&mut (&mut stream).take(1 << 20), &mut io::sink());
    }
    Ok(())
}

/// Every `Cookie` header joined with `; `, and the `Accept` header.
fn cookie_and_accept(head: &str) -> (String, String) {
    let (mut cookie, mut accept) = (String::new(), String::new());
    for line in head.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.eq_ignore_ascii_case("cookie") {
            if !cookie.is_empty() {
                cookie.push_str("; ");
            }
            cookie.push_str(value.trim());
        } else if name.eq_ignore_ascii_case("accept") && accept.is_empty() {
            accept = value.trim().to_string();
        }
    }
    (cookie, accept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn user_data(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("scout-report-route-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(report::STATE_FILE),
            br#"{"version":1,"topics":{"7":{"verdict":"known","rules":1,"at":"2026-11-01T00:00:00Z"}},"feeds":{}}"#,
        )
        .unwrap();
        dir
    }

    fn with_waiting(name: &str) -> std::path::PathBuf {
        let dir = user_data(name);
        std::fs::write(
            dir.join(report::WAITING_FILE),
            br#"{"candidates":[{"id":7,"score":1,"axisType":"multi"},{"id":8,"score":3,"axisType":"multi"},{"id":9,"score":2,"axisType":"multi"}]}"#,
        )
        .unwrap();
        dir
    }

    fn get_json(target: &str, dir: &Path) -> Value {
        let r = route("GET", target, b"", dir);
        assert_eq!(
            (r.status, r.content_type),
            (200, "application/json"),
            "{target}"
        );
        serde_json::from_slice(&r.body).unwrap()
    }

    /// Topic ids of a deck's cards (posts only).
    fn ids(deck: &Value) -> Vec<u64> {
        deck["cards"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_u64().unwrap())
            .collect()
    }

    /// `<source>:<id>` of a deck's cards.
    fn keys(deck: &Value) -> Vec<String> {
        deck["cards"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| match &c["id"] {
                Value::String(s) => format!("{}:{s}", c["source"].as_str().unwrap()),
                n => format!("{}:{n}", c["source"].as_str().unwrap()),
            })
            .collect()
    }

    #[test]
    fn serves_the_deck_page_and_sends_the_root_there() {
        let dir = user_data("pages");
        for (m, t) in [("GET", "/"), ("HEAD", "/?x=1")] {
            let r = route(m, t, b"", &dir);
            assert_eq!(r.status, 303, "{m} {t}");
            assert_eq!(r.headers, [("Location", "/swipe".to_string())]);
        }
        let r = route("GET", "/swipe#top", b"", &dir);
        assert_eq!(
            (r.status, r.content_type),
            (200, "text/html; charset=utf-8")
        );
        assert_eq!(r.body, SWIPE_PAGE.as_bytes());
        // The judgments report is gone.
        assert_eq!(route("GET", "/api/report", b"", &dir).status, 404);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn drive_status_without_a_library() {
        let dir = user_data("drive");
        let v = get_json("/api/drive", &dir);
        assert_eq!((&v["drive"], &v["full"]), (&Value::Null, &json!(false)));
        let problem = v["problems"][0].as_str().unwrap();
        assert!(problem.contains("config.json"), "{problem}");
        assert_eq!(v["minFreeBytes"], drive::min_free_bytes().unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_drive_refuses_swipes() {
        let status = |free: u64, full: bool| drive::Status {
            drive: Some(free_space::Drive {
                mount: "/Volumes/2TB".into(),
                free,
                total: 2_000_000_000_000,
            }),
            min_free_bytes: Some(50_000_000_000),
            full,
            problems: vec![],
        };
        let r = refuse_when_full(&status(18_544_640 * 1024, true)).unwrap();
        assert_eq!(r.status, 507);
        assert_eq!(
            String::from_utf8(r.body).unwrap(),
            "The library drive is full: 19.0 GB free on /Volumes/2TB, swiping stops under 50.0 GB. Delete videos (the Free space tab) to swipe again.\n"
        );
        assert_eq!(reason(507), "Insufficient Storage");
        assert_eq!(refuse_when_full(&status(60_000_000_000, false)), None);
    }

    #[test]
    fn deck_limits() {
        let dir = with_waiting("deck");
        let v = get_json("/api/deck", &dir);
        assert_eq!(ids(&v), [8, 9, 7]);
        assert_eq!(
            (v["remaining"].as_u64(), v["wanted"].as_u64()),
            (Some(3), Some(0))
        );
        assert_eq!(ids(&get_json("/api/deck?limit=1", &dir)), [8]);
        assert_eq!(
            ids(&get_json("/api/deck?x=1&limit=0", &dir)),
            [] as [u64; 0]
        );
        assert_eq!(get_json("/api/deck?limit=100000", &dir)["remaining"], 3);
        // Cards the page holds are left out; the count still covers them.
        let v = get_json("/api/deck?exclude=eroscripts:8,eroscripts:7&limit=5", &dir);
        assert_eq!((ids(&v), v["remaining"].as_u64()), (vec![9], Some(3)));
        // Another source's id 8 isn't post 8.
        assert_eq!(
            ids(&get_json("/api/deck?exclude=faptap:8,slr:9", &dir)),
            [8, 9, 7]
        );
        for bad in [
            "/api/deck?limit=",
            "/api/deck?limit=-1",
            "/api/deck?limit=x",
            "/api/deck?seed=-3",
            "/api/deck?exclude=8",
            "/api/deck?exclude=eroscripts:8,x",
            "/api/deck?exclude=eroscripts:x",
            "/api/deck?exclude=eroscripts:+8",
            "/api/deck?exclude=eroscripts:",
            "/api/deck?exclude=nope:8",
            "/api/deck?exclude=slr:",
            "/api/deck?exclude=vrb:a%20b",
            "/api/deck?exclude=vrb:a:b",
            "/api/deck?stack=single",
            "/api/deck?stack=SLR",
            "/api/deck?stack=VR",
            "/api/deck?stack=",
            "/api/deck?jitter=",
            "/api/deck?jitter=31",
            "/api/deck?jitter=30.01",
            "/api/deck?jitter=-1",
            "/api/deck?jitter=x",
            "/api/deck?jitter=NaN",
            "/api/deck?jitter=inf",
        ] {
            assert_eq!(route("GET", bad, b"", &dir).status, 400, "{bad}");
        }
        let many = (0..=deck::MAX_EXCLUDE)
            .map(|i| format!("faptap:{i}"))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            route("GET", &format!("/api/deck?exclude={many}"), b"", &dir).status,
            400
        );
        assert_eq!(deal("limit=100000").map(|d| d.limit), Ok(deck::MAX_LIMIT));
        assert_eq!(deal(""), Ok(deck::Deal::default()));
        assert_eq!(
            deal("seed=9&exclude=").map(|d| (d.seed, d.exclude.len())),
            Ok((Some(9), 0))
        );
        // jitter: 0 ..= MAX_JITTER score points; 0 deals strictly by score, seed or not.
        assert_eq!(deal("jitter=0").map(|d| d.jitter), Ok(Some(0.0)));
        assert_eq!(deal("jitter=2.5").map(|d| d.jitter), Ok(Some(2.5)));
        assert_eq!(
            deal("jitter=30").map(|d| d.jitter),
            Ok(Some(deck::MAX_JITTER))
        );
        assert_eq!(deal("seed=1").map(|d| d.jitter), Ok(None));
        assert_eq!(
            ids(&get_json("/api/deck?seed=12345&jitter=0", &dir)),
            [8, 9, 7]
        );
        for (q, stack) in [
            ("stack=multi", deck::Stack::Multi),
            ("stack=vr", deck::Stack::Vr),
            ("stack=flat", deck::Stack::Flat),
            ("stack=dupes", deck::Stack::Dupes),
        ] {
            assert_eq!(deal(q).map(|d| d.stack), Ok(stack));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exclude_items_are_card_keys() {
        for (item, key) in [
            ("eroscripts:8", Some("eroscripts:8")),
            ("eroscripts:0008", Some("eroscripts:8")),
            ("slr:66249", Some("slr:66249")),
            (
                "vrb:6accfc2cff316f0c39086f52",
                Some("vrb:6accfc2cff316f0c39086f52"),
            ),
            ("pmvhaven:abc_1-2", Some("pmvhaven:abc_1-2")),
            (
                "faptap:1957860091331874817",
                Some("faptap:1957860091331874817"),
            ),
            ("eroscripts:-1", None),
            ("eroscripts:1.0", None),
            ("eroscripts:abc", None),
            ("eroscripts", None),
            (":8", None),
            ("SLR:8", None),
            ("vrb:", None),
            ("vrb:a:b", None),
            ("vrb:a/b", None),
            ("dupes:a1b2-c3d4", Some("dupes:a1b2-c3d4")),
            ("dupes:c3d4-a1b2", None),
            ("dupes:a1b2", None),
            ("library:a1b2-c3d4", Some("library:a1b2-c3d4")),
        ] {
            assert_eq!(exclude_key(item).as_deref(), key, "{item}");
        }
    }

    #[test]
    fn duplicate_swipes_need_a_pair_and_a_direction() {
        let dir = with_waiting("dupes");
        for (body, why) in [
            (&br#"{"id":"b2-a1","dir":"left"}"#[..], "pair"),
            (br#"{"id":"a1","dir":"left"}"#, "pair"),
            (br#"{"id":"a1-b2","dir":"down"}"#, "dir"),
            (br#"[]"#, "JSON object"),
        ] {
            let r = route("POST", "/api/duplicates/swipe", body, &dir);
            assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(body));
            assert!(String::from_utf8_lossy(&r.body).contains(why));
        }
        let r = route(
            "POST",
            "/api/duplicates/swipe/undo",
            br#"{"id":"a1-b2"}"#,
            &dir,
        );
        assert_eq!(r.status, 200);
        assert_eq!(
            serde_json::from_slice::<Value>(&r.body).unwrap()["removed"],
            false
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn swipe_and_undo() {
        let dir = with_waiting("swipe");
        let r = route("POST", "/api/swipe", br#"{"id":8,"dir":"right"}"#, &dir);
        assert_eq!(r.status, 200);
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(
            (v["id"].as_u64(), v["dir"].as_str()),
            (Some(8), Some("right"))
        );
        assert!(v["at"].as_str().unwrap().ends_with('Z'));
        // A numeric string id works too.
        let r = route("POST", "/api/swipe", br#"{"id":"9","dir":"left"}"#, &dir);
        assert_eq!(r.status, 200);

        let v = get_json("/api/deck", &dir);
        assert_eq!(ids(&v), [7]);
        assert_eq!(
            (v["remaining"].as_u64(), v["wanted"].as_u64()),
            (Some(1), Some(1))
        );
        let s = swipes::EROSCRIPTS.read(&dir).unwrap();
        assert_eq!(
            (s.dir("8"), s.dir("9")),
            (Some(Dir::Right), Some(Dir::Left))
        );
        let r = route("POST", "/api/swipe/undo", br#"{"id":8}"#, &dir);
        assert_eq!(r.status, 200);
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v, serde_json::json!({"id": 8, "removed": true}));
        let r = route("POST", "/api/swipe/undo", br#"{"id":8}"#, &dir);
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v["removed"], false);
        assert_eq!(ids(&get_json("/api/deck", &dir)), [8, 7]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_swipes_are_rejected() {
        let dir = with_waiting("bad");
        for (path, body) in [
            ("/api/swipe", &b""[..]),
            ("/api/swipe", b"{not json"),
            ("/api/swipe", b"[8, \"right\"]"),
            ("/api/swipe", br#"{"dir":"right"}"#),
            ("/api/swipe", br#"{"id":"eight","dir":"right"}"#),
            ("/api/swipe", br#"{"id":-8,"dir":"right"}"#),
            ("/api/swipe", br#"{"id":8}"#),
            ("/api/swipe", br#"{"id":8,"dir":"up"}"#),
            ("/api/swipe", br#"{"id":8,"dir":"RIGHT"}"#),
            ("/api/swipe", br#"{"id":99,"dir":"right"}"#),
            ("/api/swipe/undo", b""),
            ("/api/swipe/undo", b"{}"),
            ("/api/swipe/undo", br#"{"id":null}"#),
        ] {
            let r = route("POST", path, body, &dir);
            assert_eq!(r.status, 400, "{path} {}", String::from_utf8_lossy(body));
        }
        let file = swipes::EROSCRIPTS.file;
        assert!(!dir.join(file).exists(), "nothing written");

        // A broken swipes file is never overwritten.
        std::fs::write(dir.join(file), b"oops").unwrap();
        let r = route("POST", "/api/swipe", br#"{"id":8,"dir":"right"}"#, &dir);
        assert_eq!(r.status, 500);
        assert!(String::from_utf8_lossy(&r.body).starts_with(file));
        assert_eq!(
            route("POST", "/api/swipe/undo", br#"{"id":8}"#, &dir).status,
            500
        );
        assert_eq!(std::fs::read(dir.join(file)).unwrap(), b"oops");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// with_waiting's posts (7, 8, 9: multi-axis) plus a scene file per source.
    fn with_scenes(name: &str) -> std::path::PathBuf {
        let dir = with_waiting(name);
        let write = |src: Source, v: Value| {
            std::fs::write(dir.join(src.file()), v.to_string()).unwrap();
        };
        write(
            Source::Slr,
            json!({"quota": {"scripts": {"remaining": 1, "total": 3}}, "quotaError": "session expired",
                   "scenes": [{"id": 8, "score": 1, "vr": true}, {"id": 66249, "score": 6, "vr": true}]}),
        );
        write(
            Source::Vrb,
            json!({"version": 1, "scenes": [
                {"id": "6accfc2cff316f0c39086f52", "site": "vrbangers", "score": 5, "vr": true,
                 "url": "https://vrbangers.com/video/x/", "script": {"axes": []}},
                {"id": "b2", "site": "vrconk", "score": 20, "vr": true, "script": {"axes": ["twist"]}}]}),
        );
        write(
            Source::Pmvhaven,
            json!({"version": 1, "scenes": [{"id": "p1", "score": 2, "vr": false}]}),
        );
        write(
            Source::Faptap,
            json!({"candidates": [{"id": "1957860091331874817", "score": 3, "vr": false},
                                  {"id": "f2", "score": 4, "vr": true}]}),
        );
        dir
    }

    #[test]
    fn tabs_mix_every_source() {
        let dir = with_scenes("tabs");
        assert_eq!(
            keys(&get_json("/api/deck", &dir)),
            ["vrb:b2", "eroscripts:8", "eroscripts:9", "eroscripts:7"]
        );
        let v = get_json("/api/deck?stack=slr", &dir);
        assert_eq!(keys(&v), ["slr:66249", "slr:8"]);
        assert_eq!(v["quota"]["scripts"]["remaining"], 1);
        assert_eq!(v["quotaError"], "session expired");
        assert_eq!(v["cards"][0]["id"], "66249", "scene ids are strings");
        assert_eq!(
            keys(&get_json("/api/deck?stack=vr", &dir)),
            ["vrb:6accfc2cff316f0c39086f52", "faptap:f2"]
        );
        assert_eq!(
            keys(&get_json("/api/deck?stack=flat", &dir)),
            ["faptap:1957860091331874817", "pmvhaven:p1"]
        );
        // The page's cards go in one exclude list, by source.
        assert_eq!(
            keys(&get_json(
                "/api/deck?stack=vr&exclude=slr:66249,vrb:6accfc2cff316f0c39086f52,eroscripts:8",
                &dir
            )),
            ["faptap:f2"]
        );
        assert_eq!(
            keys(&get_json("/api/deck?stack=slr&exclude=slr:66249", &dir)),
            ["slr:8"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scene_swipes_go_to_their_sources_file() {
        let dir = with_scenes("scene-swipes");
        let post = |path: &str, body: &str| {
            let r = route("POST", path, body.as_bytes(), &dir);
            (
                r.status,
                serde_json::from_slice::<Value>(&r.body).unwrap_or(Value::Null),
            )
        };
        // A number or a string id; the answer gives it back as a string.
        let (status, v) = post("/api/scenes/slr/swipe", r#"{"id":66249,"dir":"right"}"#);
        assert_eq!((status, v["id"].as_str()), (200, Some("66249")));
        assert_eq!(
            post("/api/scenes/slr/swipe", r#"{"id":"8","dir":"left"}"#).0,
            200
        );
        assert_eq!(
            post(
                "/api/scenes/vrb/swipe",
                r#"{"id":"6accfc2cff316f0c39086f52","dir":"right"}"#
            )
            .0,
            200
        );
        assert_eq!(
            post("/api/scenes/pmvhaven/swipe", r#"{"id":"p1","dir":"left"}"#).0,
            200
        );
        assert_eq!(
            post(
                "/api/scenes/faptap/swipe",
                r#"{"id":"1957860091331874817","dir":"right"}"#
            )
            .0,
            200
        );
        let read = |s: Store| s.read(&dir).unwrap();
        assert_eq!(read(swipes::SLR).dir("66249"), Some(Dir::Right));
        assert_eq!(read(swipes::SLR).dir("8"), Some(Dir::Left));
        assert_eq!(
            read(swipes::VRB).dir("6accfc2cff316f0c39086f52"),
            Some(Dir::Right)
        );
        assert_eq!(read(swipes::PMVHAVEN).dir("p1"), Some(Dir::Left));
        assert_eq!(
            read(swipes::FAPTAP).dir("1957860091331874817"),
            Some(Dir::Right)
        );
        // SLR scene 8 isn't post 8: the EroScripts file stays untouched.
        assert!(!dir.join(swipes::EROSCRIPTS.file).exists());
        assert_eq!(
            keys(&get_json("/api/deck", &dir)),
            ["vrb:b2", "eroscripts:8", "eroscripts:9", "eroscripts:7"]
        );
        let v = get_json("/api/deck?stack=vr", &dir);
        assert_eq!(
            (keys(&v), v["wanted"].as_u64()),
            (vec!["faptap:f2".to_string()], Some(1))
        );
        let v = get_json("/api/deck?stack=slr", &dir);
        assert_eq!((keys(&v).len(), v["wanted"].as_u64()), (0, Some(1)));
        assert_eq!(get_json("/api/deck?stack=flat", &dir)["wanted"], 1);

        let (status, v) = post("/api/scenes/slr/swipe/undo", r#"{"id":"66249"}"#);
        assert_eq!(
            (status, &v),
            (200, &json!({"id": "66249", "removed": true}))
        );
        assert_eq!(
            post("/api/scenes/slr/swipe/undo", r#"{"id":66249}"#).1["removed"],
            false
        );
        assert_eq!(
            post(
                "/api/scenes/faptap/swipe/undo",
                r#"{"id":"1957860091331874817"}"#
            )
            .1["removed"],
            true
        );
        assert_eq!(
            keys(&get_json("/api/deck?stack=flat", &dir)),
            ["faptap:1957860091331874817"]
        );

        let before = std::fs::read(dir.join(swipes::VRB.file)).unwrap();
        for (path, body, status) in [
            // Only scenes in that source's file.
            ("/api/scenes/vrb/swipe", r#"{"id":"p1","dir":"right"}"#, 400),
            ("/api/scenes/slr/swipe", r#"{"id":9,"dir":"right"}"#, 400),
            ("/api/scenes/vrb/swipe", r#"{"id":"b2","dir":"up"}"#, 400),
            ("/api/scenes/vrb/swipe", r#"{"id":"b2"}"#, 400),
            (
                "/api/scenes/vrb/swipe",
                r#"{"id":"a:b","dir":"right"}"#,
                400,
            ),
            ("/api/scenes/vrb/swipe", r#"{"id":1.5,"dir":"right"}"#, 400),
            ("/api/scenes/vrb/swipe", r#"{"dir":"right"}"#, 400),
            ("/api/scenes/vrb/swipe", "nope", 400),
            ("/api/scenes/vrb/swipe/undo", "{}", 400),
            // Unknown sources and actions.
            (
                "/api/scenes/eroscripts/swipe",
                r#"{"id":8,"dir":"right"}"#,
                404,
            ),
            (
                "/api/scenes/nope/swipe",
                r#"{"id":"b2","dir":"right"}"#,
                404,
            ),
            ("/api/scenes/vrb", r#"{"id":"b2","dir":"right"}"#, 404),
            ("/api/scenes/vrb/", r#"{"id":"b2","dir":"right"}"#, 404),
            (
                "/api/scenes/vrb/swipe/",
                r#"{"id":"b2","dir":"right"}"#,
                404,
            ),
            ("/api/scenes/vrb/undo", r#"{"id":"b2"}"#, 404),
            // The old SLR-only routes are gone.
            ("/api/slr/swipe", r#"{"id":66249,"dir":"right"}"#, 404),
            ("/api/slr/swipe/undo", r#"{"id":66249}"#, 404),
        ] {
            assert_eq!(post(path, body).0, status, "{path} {body}");
        }
        assert_eq!(std::fs::read(dir.join(swipes::VRB.file)).unwrap(), before);
        // A scene file that can't be read: a 500, nothing written.
        std::fs::write(dir.join(Source::Pmvhaven.file()), b"{oops").unwrap();
        assert_eq!(
            post("/api/scenes/pmvhaven/swipe", r#"{"id":"p1","dir":"right"}"#).0,
            500
        );
        assert_eq!(read(swipes::PMVHAVEN).dir("p1"), Some(Dir::Left));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn write_frame(dir: &Path, rel: &str) {
        let path = dir.join("scout-frames").join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("jpeg {rel}")).unwrap();
    }

    #[test]
    fn serves_frame_files_only() {
        let dir = user_data("frames");
        write_frame(&dir, "eroscripts/12/25.jpg");
        write_frame(&dir, "slr/66249/75.jpg");
        write_frame(&dir, "eroscripts/12/26.jpg");
        std::fs::write(dir.join("scout-frames/eroscripts/12/x.json"), b"{}").unwrap();
        for (method, target, body) in [
            (
                "GET",
                "/frames/eroscripts/12/25.jpg",
                "jpeg eroscripts/12/25.jpg",
            ),
            (
                "HEAD",
                "/frames/eroscripts/12/25.jpg",
                "jpeg eroscripts/12/25.jpg",
            ),
            (
                "GET",
                "/frames/slr/66249/75.jpg?v=2#x",
                "jpeg slr/66249/75.jpg",
            ),
        ] {
            let r = route(method, target, b"", &dir);
            assert_eq!(
                (r.status, r.content_type, r.cache),
                (200, "image/jpeg", PICTURE_CACHE),
                "{method} {target}"
            );
            assert_eq!(r.body, body.as_bytes());
        }
        assert_eq!(route("GET", "/api/deck", b"", &dir).cache, "no-store");
        for target in [
            "/frames/eroscripts/12/50.jpg",
            "/frames/eroscripts/12/26.jpg",
            "/frames/eroscripts/12/x.json",
            "/frames/eroscripts/12/../12/25.jpg",
            "/frames/eroscripts/../eroscripts-scout-state.json",
            "/frames/../eroscripts-scout-state.json",
            "/frames/x/12/25.jpg",
            "/frames/slr/12/25.jpg",
            "/frames/eroscripts/abc/25.jpg",
            "/frames/eroscripts/012/25.jpg",
            "/frames/eroscripts/12/25.jpg/",
            "/frames/eroscripts/12",
            "/frames/",
            "/scout-frames/eroscripts/12/25.jpg",
        ] {
            let r = route("GET", target, b"", &dir);
            assert_eq!((r.status, r.cache), (404, "no-store"), "{target}");
        }
        for m in ["POST", "PUT", "DELETE"] {
            assert_eq!(
                route(m, "/frames/eroscripts/12/25.jpg", b"", &dir).status,
                404
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cards_list_frames_after_their_images() {
        let dir = with_waiting("frame-cards");
        let frames = |files: &[&str]| json!({"at": "2026-11-14T00:00:00.000Z", "source": "MEGA: a.mp4", "files": files});
        let state = json!({"version": 1, "topics": {
            "7": {"verdict": "candidate", "images": []},
            "8": {"verdict": "candidate", "image": "https://x/8a.jpg",
                  "images": ["https://x/8a.jpg", "https://x/8b.jpg", "https://x/8c.jpg", "https://x/8d.jpg"]},
            // 9 and 41: frames on disk listed only where --frames kept them before
            // eroscripts-scout-frames.json: they don't count.
            "9": {"verdict": "candidate", "image": "https://x/9.jpg", "images": ["https://x/9.jpg"],
                  "frames": frames(&["25.jpg"])},
            "40": {"verdict": "candidate"},
            "41": {"verdict": "candidate", "frames": frames(&["25.jpg"])}
        }});
        std::fs::write(dir.join(report::STATE_FILE), state.to_string()).unwrap();
        let made = json!({"version": 1, "topics": {
            // 7: no images, three frames listed, two on disk.
            "7": {"frames": frames(&["25.jpg", "50.jpg", "75.jpg"])},
            // 8: four images and two frames: capped at five.
            "8": {"frames": frames(&["25.jpg", "50.jpg"])},
            // 40: a post on SLR: the eroscripts path, by topic id.
            "40": {"frames": frames(&["50.jpg"])},
            "42": {"framesFailed": {"at": "2026-11-14T00:00:00.000Z", "reason": "pixeldrain: HTTP 403"}}
        }});
        std::fs::write(dir.join(frames::EROSCRIPTS_FILE), made.to_string()).unwrap();
        for rel in [
            "eroscripts/7/25.jpg",
            "eroscripts/7/75.jpg",
            "eroscripts/8/25.jpg",
            "eroscripts/8/50.jpg",
            "eroscripts/9/25.jpg",
            "eroscripts/40/50.jpg",
            "eroscripts/41/25.jpg",
            "slr/77/25.jpg",
            "slr/77/50.jpg",
        ] {
            write_frame(&dir, rel);
        }
        let waiting = json!({"candidates": [
            {"id": 7, "score": 1, "axisType": "multi"}, {"id": 8, "score": 3, "axisType": "multi"},
            {"id": 9, "score": 2, "axisType": "multi"},
            {"id": 40, "score": 5, "vr": true, "scenes": [{"video": {"host": "slr", "slrSceneId": 66249}}]}
        ]});
        std::fs::write(dir.join(report::WAITING_FILE), waiting.to_string()).unwrap();
        std::fs::write(
            dir.join(Source::Slr.file()),
            json!({"scenes": [
                {"id": 77, "score": 3, "vr": true, "thumbnail": "https://cdn/77.webp",
                 "frames": frames(&["25.jpg", "50.jpg", "75.jpg"])},
                {"id": 78, "score": 2, "vr": true, "frames": frames(&["25.jpg"])}
            ]})
            .to_string(),
        )
        .unwrap();

        let v = get_json("/api/deck", &dir);
        let images = |v: &Value, i: usize| v["cards"][i]["images"].clone();
        assert_eq!(ids(&v), [8, 9, 7]);
        assert_eq!(
            images(&v, 0),
            json!([
                "https://x/8a.jpg",
                "https://x/8b.jpg",
                "https://x/8c.jpg",
                "https://x/8d.jpg",
                "frames/eroscripts/8/25.jpg"
            ])
        );
        assert_eq!(images(&v, 1), json!(["https://x/9.jpg"]));
        assert_eq!(
            images(&v, 2),
            json!(["frames/eroscripts/7/25.jpg", "frames/eroscripts/7/75.jpg"])
        );

        // The SLR tab: post 40 (its video on SLR) and SLR scenes with their own frames.
        let v = get_json("/api/deck?stack=slr", &dir);
        assert_eq!(v["cards"][0]["id"], 40);
        assert_eq!(images(&v, 0), json!(["frames/eroscripts/40/50.jpg"]));
        assert_eq!(
            (&v["cards"][1]["id"], &v["cards"][1]["source"]),
            (&json!("77"), &json!("slr"))
        );
        assert_eq!(
            images(&v, 1),
            json!([
                "https://cdn/77.webp",
                "frames/slr/77/25.jpg",
                "frames/slr/77/50.jpg"
            ])
        );
        // 78's frame isn't on disk.
        assert_eq!(images(&v, 2), json!([]));
        // Every frame URL a card lists is served.
        assert_eq!(route("GET", "/frames/slr/77/50.jpg", b"", &dir).status, 200);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_bodies_by_content_length() {
        let head =
            |len: &str| format!("POST /api/swipe HTTP/1.1\r\nHost: x\r\nContent-length: {len}");
        let mut none = io::empty();
        // Part with the head, the rest from the stream; extra bytes ignored.
        let mut more = io::Cursor::new(b"lo!!".to_vec());
        assert_eq!(read_body(&head("5"), b"hel", &mut more).unwrap(), b"hello");
        assert_eq!(read_body(&head("2"), b"hello", &mut none).unwrap(), b"he");
        assert_eq!(
            read_body("GET / HTTP/1.1\r\nHost: x", b"", &mut none).unwrap(),
            b""
        );
        assert_eq!(
            read_body(&head("4096"), &[b'x'; 4096], &mut none)
                .unwrap()
                .len(),
            4096
        );
        let status = |r: Result<Vec<u8>, Response>| r.unwrap_err().status;
        assert_eq!(status(read_body(&head("4097"), b"", &mut none)), 413);
        assert_eq!(
            status(read_body(&head("99999999999999999999999"), b"", &mut none)),
            400
        );
        assert_eq!(status(read_body(&head("x"), b"", &mut none)), 400);
        assert_eq!(status(read_body(&head("-1"), b"", &mut none)), 400);
        assert_eq!(status(read_body(&head("10"), b"short", &mut none)), 400);
        let twice = format!("{}\r\nContent-Length: 3", head("2"));
        assert_eq!(status(read_body(&twice, b"abc", &mut none)), 400);
    }

    #[test]
    fn everything_else_is_not_found() {
        let dir = std::env::temp_dir();
        for (m, t) in [
            ("GET", "/index.html"),
            ("GET", "/api/report"),
            ("GET", "/api"),
            ("GET", "/../eroscripts-scout.json"),
            ("GET", "/eroscripts-scout-state.json"),
            ("GET", "/eroscripts-scout-swipes.json"),
            ("GET", "/slr-scout.json"),
            ("GET", "/vrb-scout-swipes.json"),
            ("GET", "/api/slr/deck"),
            ("GET", "/api/slr/swipe"),
            ("POST", "/api/slr/deck"),
            ("GET", "/api/scenes/vrb/swipe"),
            ("PUT", "/api/scenes/vrb/swipe"),
            ("GET", "/api/swipe"),
            ("GET", "/swipe/"),
            ("POST", "/api/drive"),
            ("POST", "/api/deck"),
            ("PUT", "/api/swipe"),
            ("DELETE", "/"),
        ] {
            assert_eq!(route(m, t, b"", &dir).status, 404, "{m} {t}");
        }
    }

    #[test]
    fn binds_loopback_unless_lan() {
        let l = bind(0, false).unwrap();
        assert!(l.local_addr().unwrap().ip().is_loopback());
        let l = bind(0, true).unwrap();
        assert!(l.local_addr().unwrap().ip().is_unspecified());
    }

    #[test]
    fn creator_cards_swipe_into_creator_swipes_json_and_serve_her_pictures() {
        let dir = user_data("creator-cards");
        std::fs::write(
            dir.join(creators::FILE),
            br#"{"creators":{"ann":{"name":"Ann","sources":["favourite"],
                "pictures":[{"file":"ig-1.jpg","source":"instagram"}],
                "posts":[{"kind":"eroscripts","id":500,"title":"Own","score":6,"downloadable":true}]},
              "bea":{"name":"Bea","posts":[{"kind":"faptap","id":"9","score":1,"downloadable":true}]}}}"#,
        )
        .unwrap();
        let pics = dir.join(creator_pictures::DIR).join("ann");
        std::fs::create_dir_all(&pics).unwrap();
        std::fs::write(pics.join("ig-1.jpg"), b"jpeg").unwrap();
        let deck = get_json("/api/deck?stack=flat", &dir);
        assert_eq!(deck["cards"][0]["source"], "creators");
        assert_eq!(deck["cards"][0]["id"], "eroscripts-500");
        assert_eq!(deck["cards"][0]["creator"], "Ann");
        assert_eq!(deck["cards"][0]["images"][0], "pictures/ann/ig-1.jpg");
        let r = route("GET", "/pictures/ann/ig-1.jpg", b"", &dir);
        assert_eq!(
            (r.status, r.content_type, r.body.as_slice()),
            (200, "image/jpeg", &b"jpeg"[..])
        );
        assert_eq!(
            route("GET", "/pictures/ann/../x.jpg", b"", &dir).status,
            404
        );
        // The page leaves out the cards it holds, creator cards too.
        let left = get_json("/api/deck?stack=flat&exclude=creators:eroscripts-500", &dir);
        assert_eq!(left["cards"], json!([]));

        let post = |path: &str, body: &str| route("POST", path, body.as_bytes(), &dir).status;
        assert_eq!(
            post(
                "/api/scenes/creators/swipe",
                r#"{"id":"eroscripts-500","dir":"right"}"#
            ),
            200
        );
        let on_disk: Value =
            serde_json::from_slice(&std::fs::read(dir.join(creators::SWIPES_FILE)).unwrap())
                .unwrap();
        assert_eq!(on_disk["posts"]["eroscripts:500"]["choice"], "want");
        assert_eq!(on_disk["posts"]["eroscripts:500"]["creator"], "ann");
        let deck = get_json("/api/deck?stack=flat", &dir);
        assert_eq!(
            (deck["remaining"].as_u64(), deck["wanted"].as_u64()),
            (Some(0), Some(1))
        );
        // Bea isn't yes; a bad id is no card.
        assert_eq!(
            post(
                "/api/scenes/creators/swipe",
                r#"{"id":"faptap-9","dir":"right"}"#
            ),
            400
        );
        assert_eq!(
            post(
                "/api/scenes/creators/swipe",
                r#"{"id":"slr-1","dir":"right"}"#
            ),
            400
        );
        assert_eq!(
            post(
                "/api/scenes/creators/swipe/undo",
                r#"{"id":"eroscripts-500"}"#
            ),
            200
        );
        assert_eq!(get_json("/api/deck?stack=flat", &dir)["remaining"], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dealt_cards_name_the_creators_in_them_with_her_picture() {
        let dir = user_data("people");
        std::fs::write(
            dir.join(report::WAITING_FILE),
            br#"{"candidates":[{"id":11,"score":2,"title":"Sweetie Fox POV"}]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join(Source::Pmvhaven.file()),
            json!({"version": 1, "scenes": [{"id": "p1", "score": 1, "title": "Tribute",
                   "actors": ["Swéetie Fox"], "thumbnail": "https://pmvhaven.test/p1.jpg"}]})
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.join(creators::FILE),
            br#"{"creators":{"sweetie":{"name":"Sweetie Fox","pictures":[
                {"file":"ig-1.jpg","source":"instagram","url":"https://www.instagram.com/p/A/","likes":5}]}}}"#,
        )
        .unwrap();
        let pics = dir.join(creator_pictures::DIR).join("sweetie");
        std::fs::create_dir_all(&pics).unwrap();
        std::fs::write(pics.join("ig-1.jpg"), b"jpeg").unwrap();
        let deck = get_json("/api/deck?stack=flat", &dir);
        assert_eq!(keys(&deck), ["eroscripts:11", "pmvhaven:p1"]);
        let person = json!({"name": "Sweetie Fox", "slug": "sweetie", "picture": "pictures/sweetie/ig-1.jpg",
                            "pictureSource": "instagram", "pictureUrl": "https://www.instagram.com/p/A/"});
        let post = &deck["cards"][0];
        assert_eq!(post["people"], json!([person]));
        assert_eq!(post["images"], json!(["pictures/sweetie/ig-1.jpg"]));
        let scene = &deck["cards"][1];
        assert_eq!(scene["people"], json!([person]));
        assert_eq!(
            scene["images"],
            json!(["https://pmvhaven.test/p1.jpg", "pictures/sweetie/ig-1.jpg"])
        );
        // Without her picture, nobody: no `people` at all.
        std::fs::remove_file(pics.join("ig-1.jpg")).unwrap();
        let deck = get_json("/api/deck?stack=flat", &dir);
        assert_eq!(deck["cards"][1].get("people"), None);
        assert_eq!(
            deck["cards"][1]["images"],
            json!(["https://pmvhaven.test/p1.jpg"])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
