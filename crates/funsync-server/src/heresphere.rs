//! The headset players' library (ported from the old backend): HereSphere
//! (`/heresphere`, `/heresphere/<id>`) and DeoVR (`/deovr`, `/deovr/<id>`),
//! both sectioned by the same rules. HereSphere
//! is two-way: a scene POST sets the rating, the heart, `Playlist:<name>`
//! and `Queue:Up next` tags (only tags changed since the scene was served
//! count), and `/heresphere/events/<id>` takes plays out of the queue and
//! files unrated videos watched to the end under Needs review. No access PIN.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Extension, Path as UrlPath, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::access::HeadsetKeys;
use crate::data::{Library, now_ms};
use crate::remote::{category_names, chip_tags, duration, is_watched, rating_entry, watched_last};
use crate::scan::Video;
use crate::server::AppState;
use crate::vr::{self, Format};

const SLOW_BELOW_SPEED: f64 = 250.0;
const FAST_FROM_SPEED: f64 = 350.0;
const SHORT_BELOW_SECONDS: f64 = 300.0;
const LONG_ABOVE_SECONDS: f64 = 900.0;
const RECENT_MS: f64 = 14.0 * 24.0 * 3600.0 * 1000.0;
const TOP_RATED_MIN: f64 = 4.0;
const NEEDS_REVIEW: &str = "needs review";

/// Sections in display order (`SECTION_ORDER`); all but Recently Added list
/// by score and sink watched videos. The triage sections follow.
const SECTION_ORDER: [&str; 13] = [
    "Score 20+",
    "Score 15-19.5",
    "Score 10-14.5",
    "VR",
    "Flat",
    "Slow",
    "Medium",
    "Fast",
    "Short (<5m)",
    "Medium (5-15m)",
    "Long (15m+)",
    "Hand-made",
    "Recently Added",
];
const TRIAGE_ORDER: [&str; 4] = ["Favorites", "Top Rated", "Unrated", "Watched"];

/// What the sections and filter tags of one video depend on.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub path: String,
    pub is_3d: bool,
    pub avg_speed: f64,
    pub duration: f64,
    pub handmade: bool,
    pub date_added: f64,
    pub score: Option<f64>,
}

impl Facts {
    /// `embedded`: videos whose main script carries axes (`Snapshot::embedded_axes`).
    pub fn of(lib: &Library, v: &Video, fmt: Format, embedded: &HashSet<String>) -> Self {
        let l = lib.lib();
        let speed = v
            .funscript_path
            .as_deref()
            .and_then(|p| l["speedStatsCache"][p]["avgSpeed"].as_f64());
        Facts {
            path: v.path.clone(),
            is_3d: fmt.is_3d,
            avg_speed: speed.unwrap_or(0.0),
            duration: duration(lib, &v.path),
            handmade: embedded.contains(&v.path) || v.axes.iter().any(|a| !a.generated),
            date_added: v.date_added,
            score: l["preferenceScores"][&v.path]
                .as_f64()
                .filter(|s| s.is_finite()),
        }
    }

    fn speed(&self) -> Option<&'static str> {
        (self.avg_speed > 0.0).then_some(if self.avg_speed < SLOW_BELOW_SPEED {
            "Slow"
        } else if self.avg_speed < FAST_FROM_SPEED {
            "Medium"
        } else {
            "Fast"
        })
    }

    fn length(&self) -> Option<&'static str> {
        (self.duration > 0.0).then_some(if self.duration < SHORT_BELOW_SECONDS {
            "Short (<5m)"
        } else if self.duration <= LONG_ABOVE_SECONDS {
            "Medium (5-15m)"
        } else {
            "Long (15m+)"
        })
    }

    fn sections(&self, lib: &Library, now: f64) -> Vec<&'static str> {
        let mut out = Vec::new();
        match self.score {
            Some(s) if s >= 20.0 => out.push("Score 20+"),
            Some(s) if s >= 15.0 => out.push("Score 15-19.5"),
            Some(s) if s >= 10.0 => out.push("Score 10-14.5"),
            _ => {}
        }
        out.push(if self.is_3d { "VR" } else { "Flat" });
        out.extend(self.speed());
        out.extend(self.length());
        if self.handmade {
            out.push("Hand-made");
        }
        if self.date_added > 0.0 && now - self.date_added <= RECENT_MS {
            out.push("Recently Added");
        }
        let (rating, favorite) = rating_entry(lib, &self.path);
        if favorite {
            out.push("Favorites");
        }
        if rating >= TOP_RATED_MIN {
            out.push("Top Rated");
        }
        if rating == 0.0 {
            out.push("Unrated");
        }
        out
    }

    /// `Format:`, `Speed:`, `Length:` and the hand-made tag.
    fn filter_tags(&self) -> Vec<String> {
        let mut t = vec![format!("Format:{}", if self.is_3d { "VR" } else { "Flat" })];
        t.extend(self.speed().map(|s| format!("Speed:{s}")));
        t.extend(self.length().map(|s| format!("Length:{s}")));
        if self.handmade {
            t.push("Feature:Hand-made multi-axis".into());
        }
        t
    }
}

fn playlists(lib: &Library) -> &[Value] {
    lib.config["playlists"]
        .as_array()
        .map_or(&[], Vec::as_slice)
}

fn holds(p: &Value, path: &str) -> bool {
    p["videoPaths"]
        .as_array()
        .is_some_and(|a| a.iter().any(|x| x == path))
}

fn pl_name(p: &Value) -> &str {
    p["name"].as_str().unwrap_or("").trim()
}

fn is_needs_review(p: &Value) -> bool {
    pl_name(p).to_lowercase() == NEEDS_REVIEW
}

/// `[(name, items)]` (`build_sections`): `Playlist: Needs review`, Queue
/// (`queued`: the user's queue then auto-fill), SECTION_ORDER, the triage
/// sections, then one per playlist; empty ones left out. `entries` come in
/// title order.
pub fn build_sections<T: Clone>(
    lib: &Library,
    entries: &[(T, Facts)],
    queued: &[String],
    now: f64,
) -> Vec<(String, Vec<T>)> {
    let mut grouped: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut by_path: HashMap<&str, usize> = HashMap::new();
    for (i, (_, f)) in entries.iter().enumerate() {
        for name in f.sections(lib, now) {
            grouped.entry(name).or_default().push(i);
        }
        by_path.insert(&f.path, i);
    }
    let watched: Vec<usize> = crate::remote::watched_paths(lib)
        .iter()
        .filter_map(|p| by_path.get(p.as_str()).copied())
        .collect();
    grouped.insert("Watched", watched);
    let score = |i: &usize| entries[*i].1.score;
    for (name, list) in grouped.iter_mut() {
        if *name == "Recently Added" {
            list.sort_by(|a, b| {
                entries[*b]
                    .1
                    .date_added
                    .total_cmp(&entries[*a].1.date_added)
            });
        } else if SECTION_ORDER.contains(name) {
            list.sort_by(|a, b| match (score(a), score(b)) {
                (Some(x), Some(y)) => y.total_cmp(&x),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            });
        }
    }
    let sink = watched_last(lib);
    let finish = |mut list: Vec<usize>, sinks: bool| -> Vec<T> {
        if sinks {
            list.sort_by_key(|i| is_watched(lib, &entries[*i].1.path));
        }
        list.into_iter().map(|i| entries[i].0.clone()).collect()
    };
    let playlist_section = |p: &Value| -> Option<(String, Vec<T>)> {
        let list: Vec<usize> = p["videoPaths"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| by_path.get(x.as_str()?).copied())
            .collect();
        let name = match pl_name(p) {
            "" => "(unnamed)",
            n => n,
        };
        (!list.is_empty()).then(|| (format!("Playlist: {name}"), finish(list, false)))
    };
    let pls = playlists(lib);
    let mut out: Vec<(String, Vec<T>)> = pls
        .iter()
        .filter(|p| is_needs_review(p))
        .find_map(playlist_section)
        .into_iter()
        .collect();
    let queued: Vec<usize> = queued
        .iter()
        .filter_map(|p| by_path.get(p.as_str()).copied())
        .collect();
    if !queued.is_empty() {
        out.push(("Queue".into(), finish(queued, false)));
    }
    for name in SECTION_ORDER.iter().chain(&TRIAGE_ORDER) {
        if let Some(list) = grouped.remove(name).filter(|l| !l.is_empty()) {
            let sinks = sink && SECTION_ORDER.contains(name);
            out.push((name.to_string(), finish(list, sinks)));
        }
    }
    out.extend(
        pls.iter()
            .filter(|p| !is_needs_review(p))
            .filter_map(playlist_section),
    );
    out
}

/// Scene tags shared by HereSphere and DeoVR (`_scene_tag_names`).
pub fn scene_tags(lib: &Library, v: &Video, f: &Facts, queued: &[String]) -> Vec<String> {
    let l = lib.lib();
    let mut t = Vec::new();
    let studio = if v.source_name.is_empty() {
        Path::new(&v.path)
            .parent()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        v.source_name.clone()
    };
    if !studio.is_empty() {
        t.push(format!("Studio:{studio}"));
    }
    if v.funscript_path.is_some() {
        t.push("Feature:Is scripted".into());
    }
    t.extend(f.filter_tags());
    if queued.contains(&v.path) {
        t.push("Queue:Up next".into());
    }
    for p in playlists(lib) {
        if holds(p, &v.path) && !pl_name(p).is_empty() {
            t.push(format!("Playlist:{}", pl_name(p)));
        }
    }
    for c in category_names(lib).remove(&v.path).unwrap_or_default() {
        t.push(format!("Category:{c}"));
    }
    for c in chip_tags(&l["scriptTags"][&v.path]["tags"]) {
        t.push(format!("Tag:{c}"));
    }
    if let Some(s) = f.score {
        t.push(format!("Score:{:.1}", (s * 2.0 + 0.5).floor() / 2.0));
    }
    if is_watched(lib, &v.path) {
        t.push("Status:Watched".into());
    }
    if playlists(lib)
        .iter()
        .any(|p| is_needs_review(p) && holds(p, &v.path))
    {
        t.push("Review:Needed".into());
    }
    t
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/heresphere", get(hs_library).post(hs_library))
        .route("/heresphere/{id}", get(hs_scene).post(hs_scene))
        .route("/heresphere/events/{id}", post(hs_event))
        .route("/deovr", get(deovr_library).post(deovr_library))
        .route("/deovr/{id}", get(deovr_scene).post(deovr_scene))
}

#[derive(Deserialize)]
struct Filter {
    #[serde(default)]
    filter: String,
}

fn base_url(h: &HeaderMap) -> String {
    let host = h
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1:5300");
    format!("http://{host}")
}

/// `(video, facts)` for the listing, title order, `?filter=vr|flat|scripted`.
/// Uses only cached dimensions: a listing never probes a file.
async fn listing(st: &AppState, filter: &str) -> (Vec<(Video, Facts)>, f64) {
    let lib = st.library.read().await;
    let snap = st.snapshot.read().await.clone();
    let mut out: Vec<(Video, Facts)> = snap
        .videos
        .iter()
        .filter_map(|v| {
            let dims = st.media.cached(&v.path).map(|m| (m.width, m.height));
            let f = Facts::of(&lib, v, vr::detect(&lib, v, dims), &snap.embedded_axes);
            let keep = match filter {
                "vr" => f.is_3d,
                "flat" => !f.is_3d,
                "scripted" => v.funscript_path.is_some(),
                _ => true,
            };
            keep.then(|| (v.clone(), f))
        })
        .collect();
    out.sort_by_cached_key(|(v, _)| stem(v).to_lowercase());
    (out, now_ms() as f64)
}

fn stem(v: &Video) -> &str {
    v.name.strip_suffix(&v.ext).unwrap_or(&v.name)
}

fn cors(body: Value, heresphere: bool) -> Response {
    let mut r = Json(body).into_response();
    let h = r.headers_mut();
    h.insert("Access-Control-Allow-Origin", "*".parse().unwrap());
    if heresphere {
        h.insert("HereSphere-JSON-Version", "1".parse().unwrap());
    }
    r
}

async fn sections_for<T: Clone>(
    st: &AppState,
    filter: &str,
    item: impl Fn(&Library, &Video, &Facts) -> T,
) -> Vec<(String, Vec<T>)> {
    let (videos, now) = listing(st, filter).await;
    let snap = st.snapshot.read().await.clone();
    let lib = st.library.read().await;
    let entries: Vec<(T, Facts)> = videos
        .into_iter()
        .map(|(v, f)| (item(&lib, &v, &f), f))
        .collect();
    let queued = crate::queue::queued_paths(&lib, &snap.videos, now);
    build_sections(&lib, &entries, &queued, now)
}

async fn hs_library(
    State(st): State<Arc<AppState>>,
    Query(q): Query<Filter>,
    headers: HeaderMap,
) -> Response {
    let base = base_url(&headers);
    let library: Vec<Value> = sections_for(&st, &q.filter, |_, v, _| {
        format!("{base}/heresphere/{}", v.id)
    })
    .await
    .into_iter()
    .map(|(name, list)| json!({"name": name, "list": list}))
    .collect();
    cors(json!({"access": 1, "library": library}), true)
}

async fn deovr_library(
    State(st): State<Arc<AppState>>,
    Query(q): Query<Filter>,
    headers: HeaderMap,
    keys: Option<Extension<HeadsetKeys>>,
) -> Response {
    let keys = keys.map(|e| e.0).unwrap_or_default();
    let base = base_url(&headers);
    let scenes: Vec<Value> = sections_for(&st, &q.filter, |lib, v, f| {
        json!({
            "title": stem(v),
            "videoLength": f.duration as u64,
            "thumbnailUrl": key(format!("{base}{}", crate::remote::thumb_url(lib, v)), &keys.thumb),
            "video_url": format!("{base}/deovr/{}", v.id),
        })
    })
    .await
    .into_iter()
    .map(|(name, list)| json!({"name": name, "list": list}))
    .collect();
    cors(
        json!({"scenes": scenes, "authorized": if keys.media.is_some() { "1" } else { "0" }}),
        false,
    )
}

/// What both scene details need: the video, its probed size, layout, tags.
struct Scene {
    v: Video,
    width: u32,
    height: u32,
    codec: String,
    fmt: Format,
    duration: u64,
    tags: Vec<String>,
    rating: f64,
    favorite: bool,
    queued: bool,
    thumb: String,
}

async fn scene(st: &AppState, id: &str) -> Option<Scene> {
    let v = st.video(id).await?;
    // Probe this one video (cached), so the layout sees real dimensions.
    let (width, height, codec) = match st.media.metadata(Path::new(&v.path)).await {
        Ok(m) if m.width > 0 && m.height > 0 => (m.width, m.height, m.codec),
        _ => (1920, 1080, "h264".into()),
    };
    let snap = st.snapshot.read().await.clone();
    let lib = st.library.read().await;
    let queued = crate::queue::queued_paths(&lib, &snap.videos, now_ms() as f64);
    let fmt = vr::detect(&lib, &v, Some((width, height)));
    let facts = Facts::of(&lib, &v, fmt, &snap.embedded_axes);
    let (rating, favorite) = rating_entry(&lib, &v.path);
    Some(Scene {
        width,
        height,
        codec,
        fmt,
        duration: facts.duration as u64,
        tags: scene_tags(&lib, &v, &facts, &queued),
        rating,
        favorite,
        queued: queued.contains(&v.path),
        thumb: crate::remote::thumb_url(&lib, &v),
        v,
    })
}

fn key(url: String, k: &Option<String>) -> String {
    video_tags::access::with_key(&url, k.as_deref())
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"detail": "Scene not found"})),
    )
        .into_response()
}

fn file_name(p: &str) -> String {
    Path::new(p)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Chapter marks every 5 minutes for videos over 5 minutes.
fn chapters(duration: u64) -> Vec<u64> {
    (1..)
        .map(|i| i * 300)
        .take_while(|t| *t < duration)
        .collect()
}

async fn hs_scene(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    keys: Option<Extension<HeadsetKeys>>,
    body: Bytes,
) -> Response {
    let keys = keys.map(|e| e.0).unwrap_or_default();
    // HereSphere POSTs the scene both to load it and to save edits.
    if let Some(v) = st.video(&id).await {
        let served = SERVED.lock().unwrap().get(&v.path).cloned();
        let changed = apply_scene_update(
            &mut *st.library.write().await,
            &v.path,
            &crate::edits::body(&body),
            served.as_ref(),
        );
        if changed {
            let _ = st.library_changed().await;
        }
    }
    let Some(s) = scene(&st, &id).await else {
        return not_found();
    };
    let playlists_served = s
        .tags
        .iter()
        .filter_map(|t| t.strip_prefix("Playlist:"))
        .map(fold)
        .collect();
    SERVED
        .lock()
        .unwrap()
        .insert(s.v.path.clone(), (s.queued, playlists_served));
    let base = base_url(&headers);
    let screen = s.fmt.screen;
    let projection = match screen {
        "dome" => "equirectangular",
        "sphere" => "equirectangular360",
        "mkx200" | "mkx220" | "rf52" | "fisheye" => "fisheye",
        _ => "perspective",
    };
    let lens = match screen {
        "mkx200" => "MKX200",
        "mkx220" => "MKX220",
        "rf52" => "RF52",
        _ => "Linear",
    };
    let fov = match screen {
        "dome" | "mkx200" | "mkx220" | "rf52" | "fisheye" => 180.0,
        "sphere" => 360.0,
        _ => 0.0,
    };
    let size = s.v.size as f64;
    let label = if size > 1024f64.powi(3) {
        format!("{:.1} GB", size / 1024f64.powi(3))
    } else {
        format!("{:.0} MB", size / 1024f64.powi(2))
    };
    let mut tags: Vec<Value> = s.tags.iter().map(|t| json!({"name": t})).collect();
    for t in chapters(s.duration) {
        tags.push(json!({"name": format!("{} min", t / 60), "start": t * 1000,
            "end": (t + 300).min(s.duration) * 1000, "track": 0}));
    }
    let id = &s.v.id;
    let scripts: Vec<Value> =
        s.v.funscript_path
            .iter()
            .map(|p| json!({"name": file_name(p), "url": key(format!("{base}/api/media/script/{id}"), &keys.media)}))
            .collect();
    let subtitles: Vec<Value> = s.v.subtitle_path.iter().map(|p| {
        json!({"name": file_name(p), "language": "English", "url": key(format!("{base}/api/media/subtitle/{id}"), &keys.media)})
    }).collect();
    let stereo = match s.fmt.stereo {
        "sbs" | "tb" => s.fmt.stereo,
        _ => "mono",
    };
    cors(
        json!({
            "access": 1,
            "title": stem(&s.v),
            "description": "",
            "thumbnailImage": key(format!("{base}{}", s.thumb), &keys.thumb),
            "dateReleased": "",
            "dateAdded": "",
            "duration": s.duration * 1000,
            "rating": s.rating,
            "isFavorite": s.favorite,
            "projection": projection,
            "stereo": stereo,
            "fov": fov,
            "lens": lens,
            "scripts": scripts,
            "subtitles": subtitles,
            "tags": tags,
            "media": [{"name": format!("{}p - {label}", s.height), "sources": [{
                "resolution": s.height.to_string(), "height": s.height, "width": s.width,
                "size": s.v.size, "url": key(format!("{base}/api/media/stream/{id}"), &keys.media)}]}],
            "writeFavorite": true,
            "writeRating": true,
            "writeTags": true,
            "writeHSP": false,
            "eventServer": key(format!("{base}/heresphere/events/{id}"), &keys.media),
        }),
        true,
    )
}

async fn deovr_scene(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    keys: Option<Extension<HeadsetKeys>>,
) -> Response {
    let keys = keys.map(|e| e.0).unwrap_or_default();
    let Some(s) = scene(&st, &id).await else {
        return not_found();
    };
    let base = base_url(&headers);
    let id = &s.v.id;
    let fleshlight: Vec<Value> =
        s.v.funscript_path
            .iter()
            .map(|p| json!({"title": file_name(p), "url": key(format!("{base}/api/media/script/{id}"), &keys.media)}))
            .collect();
    let timestamps: Vec<Value> = chapters(s.duration)
        .into_iter()
        .map(|t| json!({"ts": t, "name": format!("{} min", t / 60)}))
        .collect();
    let categories: Vec<Value> = s
        .tags
        .iter()
        .enumerate()
        .map(|(i, t)| json!({"tag": {"id": i + 1, "name": t}}))
        .collect();
    cors(
        json!({
            "id": id,
            "title": stem(&s.v),
            "authorized": 1,
            "description": "",
            "date": now_ms() / 1000,
            "is3d": s.fmt.is_3d,
            "screenType": s.fmt.screen,
            "stereoMode": s.fmt.stereo,
            "videoLength": s.duration,
            "thumbnailUrl": key(format!("{base}{}", s.thumb), &keys.thumb),
            "encodings": [{"name": s.codec, "videoSources": [{
                "resolution": s.height, "height": s.height, "width": s.width,
                "size": s.v.size, "url": key(format!("{base}/api/media/stream/{id}"), &keys.media)}]}],
            "fleshlight": fleshlight,
            "timeStamps": timestamps,
            "isScripted": s.v.funscript_path.is_some(),
            "categories": categories,
            "isFavorite": s.favorite,
            "fullVideoReady": true,
            "fullAccess": true,
        }),
        false,
    )
}

/// path → (served with `Queue:Up next`, case-folded `Playlist:` names) of
/// the scene detail last served, so only the user's tag changes count.
type Served = (bool, HashSet<String>);
static SERVED: LazyLock<Mutex<HashMap<String, Served>>> = LazyLock::new(Default::default);
/// The video HereSphere last reported playing: a play after a pause isn't
/// another queue `played`.
static LAST_PLAYED: Mutex<Option<String>> = Mutex::new(None);
const QUEUE_TAG: &str = "queue:up next";
const REVIEW_AT_FRACTION: f64 = 0.9;

fn fold(s: &str) -> String {
    s.trim().to_lowercase()
}

/// Apply a HereSphere scene POST body (`parse_heresphere_update`): a
/// changed `rating` / `isFavorite`; with `tags`, `Playlist:<name>` tags
/// added or removed relative to `served` (without a served record, relative
/// to the playlists the video is in) and `Queue:Up next` (only with a served
/// record). A plain load changes nothing. True when something changed.
pub fn apply_scene_update(
    lib: &mut Library,
    path: &str,
    b: &Value,
    served: Option<&Served>,
) -> bool {
    let (cur_rating, cur_fav) = rating_entry(lib, path);
    let rating = crate::edits::normalize_rating(&b["rating"]).filter(|r| *r != cur_rating);
    let fav = b["isFavorite"].as_bool().filter(|f| *f != cur_fav);
    let mut changed = rating.is_some() || fav.is_some();
    if changed {
        crate::edits::set_rating(lib, path, rating, fav);
    }
    let Some(tags) = b["tags"].as_array() else {
        return changed;
    };
    let names: Vec<&str> = tags.iter().filter_map(|t| t["name"].as_str()).collect();
    let mut wanted: Vec<String> = Vec::new();
    for n in &names {
        if n.len() >= 9 && n.is_char_boundary(9) && n[..9].eq_ignore_ascii_case("playlist:") {
            let n = n[9..].trim().to_string();
            if !n.is_empty() && !wanted.iter().any(|w| fold(w) == fold(&n)) {
                wanted.push(n);
            }
        }
    }
    let member: Vec<String> = playlists(lib)
        .iter()
        .filter(|p| holds(p, path) && !pl_name(p).is_empty())
        .map(|p| pl_name(p).to_string())
        .collect();
    let was_served = |k: &str| served.is_some_and(|s| s.1.contains(k));
    for n in &wanted {
        let k = fold(n);
        if !member.iter().any(|m| fold(m) == k) && !was_served(&k) {
            changed |= crate::edits::edit_playlist(lib, path, None, Some(n), true);
        }
    }
    for m in &member {
        let k = fold(m);
        if !wanted.iter().any(|w| fold(w) == k) && (served.is_none() || was_served(&k)) {
            changed |= crate::edits::edit_playlist(lib, path, None, Some(m), false);
        }
    }
    if let Some((was_queued, _)) = served {
        let tagged = names.iter().any(|n| fold(n) == QUEUE_TAG);
        let change = match (tagged, *was_queued) {
            (true, false) => Some(crate::queue::Change::Add(path, false)),
            (false, true) => Some(crate::queue::Change::Remove(path)),
            _ => None,
        };
        if let Some(c) = change {
            crate::queue::apply(lib, c, now_ms() as f64);
            changed = true;
        }
    }
    changed
}

/// A HereSphere playback event (`{event, time}`: 0 open, 1 play, 2 pause,
/// 3 close; time in ms): a play takes the video out of the queue once; a
/// pause / close at ≥ 90 % of an unrated video puts it in Needs review.
/// True when something changed.
pub fn apply_event(lib: &mut Library, path: &str, b: &Value, last: &mut Option<String>) -> bool {
    let event = b["event"].as_i64();
    let mut changed = false;
    if event == Some(1) && last.as_deref() != Some(path) {
        crate::queue::apply(lib, crate::queue::Change::Played(path), now_ms() as f64);
        *last = Some(path.to_string());
        changed = true;
    }
    let dur = duration(lib, path);
    let at = b["time"].as_f64().unwrap_or(-1.0) / 1000.0;
    if matches!(event, Some(2 | 3))
        && dur > 0.0
        && at >= REVIEW_AT_FRACTION * dur
        && rating_entry(lib, path).0 == 0.0
        && !playlists(lib)
            .iter()
            .any(|p| is_needs_review(p) && holds(p, path))
    {
        changed |= crate::edits::edit_playlist(
            lib,
            path,
            None,
            Some(crate::edits::NEEDS_REVIEW_PLAYLIST),
            true,
        );
    }
    changed
}

async fn hs_event(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    body: Bytes,
) -> Response {
    if let Some(v) = st.video(&id).await {
        let mut last = LAST_PLAYED.lock().unwrap().clone();
        let changed = apply_event(
            &mut *st.library.write().await,
            &v.path,
            &crate::edits::body(&body),
            &mut last,
        );
        *LAST_PLAYED.lock().unwrap() = last;
        if changed {
            let _ = st.library_changed().await;
        }
    }
    cors(json!({}), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_axes_count_as_hand_made() {
        let v: Video = serde_json::from_value(json!({"id": "x", "name": "n", "path": "/e.mp4", "ext": ".mp4",
            "funscriptPath": "/e.funscript", "fuzzyMatched": false, "subtitlePath": null, "variants": [],
            "axes": [], "sourceName": "s", "dateAdded": 0.0, "size": 0}))
        .unwrap();
        let lib = Library::default();
        assert!(!Facts::of(&lib, &v, vr::FLAT, &HashSet::new()).handmade);
        assert!(Facts::of(&lib, &v, vr::FLAT, &HashSet::from([v.path.clone()])).handmade);
    }

    fn facts(path: &str, score: Option<f64>, date_added: f64) -> Facts {
        Facts {
            path: path.into(),
            score,
            date_added,
            ..Default::default()
        }
    }

    #[test]
    fn sections() {
        let lib = Library {
            config: json!({
                "settings": {
                    "library": {"ratings": {
                        "/b": {"rating": 4.5, "favorite": true, "updatedAt": 2},
                        "/c": {"rating": 2, "updatedAt": 3}}},
                    "player": {"userQueue": ["/c", "/gone", "/a"]}},
                "playlists": [
                    {"name": "Mine", "videoPaths": ["/c", "/a"]},
                    {"name": " needs Review ", "videoPaths": ["/a"]},
                    {"name": "Empty", "videoPaths": ["/gone"]}],
            }),
            ..Default::default()
        };
        let now = 100.0 * 24.0 * 3600.0 * 1000.0;
        let mut b = facts("/b", Some(16.0), now - 1.0);
        b.is_3d = true;
        b.avg_speed = 400.0;
        b.duration = 1000.0;
        b.handmade = true;
        let entries = vec![
            ("a", facts("/a", Some(12.0), 1.0)),
            ("b", b),
            ("c", facts("/c", Some(13.0), now - 2.0)),
            ("d", facts("/d", None, 0.0)),
        ];
        let queued: Vec<String> = ["/c", "/gone", "/a"].map(String::from).to_vec();
        let got = build_sections(&lib, &entries, &queued, now);
        let names: Vec<(&str, Vec<&str>)> =
            got.iter().map(|(n, l)| (n.as_str(), l.clone())).collect();
        assert_eq!(
            names,
            vec![
                ("Playlist: needs Review", vec!["a"]),
                ("Queue", vec!["c", "a"]),
                ("Score 15-19.5", vec!["b"]),
                // by score, then watched (c is rated) last
                ("Score 10-14.5", vec!["a", "c"]),
                ("VR", vec!["b"]),
                ("Flat", vec!["a", "d", "c"]),
                ("Fast", vec!["b"]),
                ("Long (15m+)", vec!["b"]),
                ("Hand-made", vec!["b"]),
                ("Recently Added", vec!["b", "c"]),
                ("Favorites", vec!["b"]),
                ("Top Rated", vec!["b"]),
                ("Unrated", vec!["a", "d"]),
                ("Watched", vec!["c", "b"]),
                ("Playlist: Mine", vec!["c", "a"]),
            ]
        );
        assert_eq!(chapters(301), vec![300]);
        assert!(chapters(300).is_empty());
    }

    #[test]
    fn scene_updates() {
        let mut lib = Library::default();
        crate::edits::edit_playlist(&mut lib, "/a", None, Some("Old"), true);
        // A plain load (credentials only) changes nothing.
        assert!(!apply_scene_update(
            &mut lib,
            "/a",
            &json!({"username": "x"}),
            None
        ));
        let served: Served = (false, HashSet::from(["old".to_string()]));
        let body = json!({"rating": 4.5, "isFavorite": true, "tags": [
            {"name": "Playlist:New"}, {"name": "Queue:Up next"}, {"name": "Status:Watched"}]});
        assert!(apply_scene_update(&mut lib, "/a", &body, Some(&served)));
        assert_eq!(rating_entry(&lib, "/a"), (4.5, true));
        let names: Vec<(&str, bool)> = playlists(&lib)
            .iter()
            .map(|p| (pl_name(p), holds(p, "/a")))
            .collect();
        assert_eq!(names, [("Old", false), ("New", true)]);
        assert_eq!(crate::queue::manual(&lib), ["/a"]);
        // Same body again, now served with those tags: nothing changes.
        let served: Served = (true, HashSet::from(["new".to_string()]));
        assert!(!apply_scene_update(&mut lib, "/a", &body, Some(&served)));
        // Queue tag removed → unqueued; a playlist tag the scene wasn't served with stays.
        crate::edits::edit_playlist(&mut lib, "/a", None, Some("Elsewhere"), true);
        let body = json!({"tags": [{"name": "Playlist:New"}]});
        assert!(apply_scene_update(&mut lib, "/a", &body, Some(&served)));
        assert!(crate::queue::manual(&lib).is_empty());
        assert!(holds(&playlists(&lib)[2], "/a"));
    }

    #[test]
    fn events() {
        let mut lib = Library::default();
        *lib.lib_mut("durationCache") = json!({"/a": 100.0});
        crate::queue::apply(&mut lib, crate::queue::Change::Add("/a", false), 0.0);
        let mut last = None;
        assert!(apply_event(
            &mut lib,
            "/a",
            &json!({"event": 1, "time": 0}),
            &mut last
        ));
        assert!(crate::queue::manual(&lib).is_empty());
        assert!(!apply_event(
            &mut lib,
            "/a",
            &json!({"event": 1, "time": 5}),
            &mut last
        ));
        assert!(!apply_event(
            &mut lib,
            "/a",
            &json!({"event": 2, "time": 89_000}),
            &mut last
        ));
        assert!(apply_event(
            &mut lib,
            "/a",
            &json!({"event": 3, "time": 90_000}),
            &mut last
        ));
        assert!(
            playlists(&lib)
                .iter()
                .any(|p| is_needs_review(p) && holds(p, "/a"))
        );
        assert!(!apply_event(
            &mut lib,
            "/a",
            &json!({"event": 3, "time": 99_000}),
            &mut last
        ));
    }
}
