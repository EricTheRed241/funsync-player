//! The headset players' library (`backend/routes/vr_api.py`): HereSphere
//! (`/heresphere`, `/heresphere/<id>`) and DeoVR (`/deovr`, `/deovr/<id>`),
//! both sectioned by `backend/services/vr_sections.py`'s rules. Read-only
//! for now: no rating / favourite / playlist / queue edits from the
//! headset, no playback events, no access PIN.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};

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
    pub fn of(lib: &Library, v: &Video, fmt: Format) -> Self {
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
            handmade: v.axes.iter().any(|a| !a.generated),
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

/// The play queue: the user's queue (`player.userQueue`). The desktop's
/// auto-fill isn't stored, so it isn't here.
fn queue(lib: &Library) -> Vec<&str> {
    lib.config["settings"]["player"]["userQueue"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

/// `[(name, items)]` (`build_sections`): `Playlist: Needs review`, Queue,
/// SECTION_ORDER, the triage sections, then one per playlist; empty ones
/// left out. `entries` come in title order.
pub fn build_sections<T: Clone>(
    lib: &Library,
    entries: &[(T, Facts)],
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
    let queued: Vec<usize> = queue(lib)
        .iter()
        .filter_map(|p| by_path.get(p).copied())
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
pub fn scene_tags(lib: &Library, v: &Video, f: &Facts) -> Vec<String> {
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
    if queue(lib).contains(&v.path.as_str()) {
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
            let f = Facts::of(&lib, v, vr::detect(&lib, v, dims));
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
    item: impl Fn(&Video, &Facts) -> T,
) -> Vec<(String, Vec<T>)> {
    let (videos, now) = listing(st, filter).await;
    let entries: Vec<(T, Facts)> = videos.into_iter().map(|(v, f)| (item(&v, &f), f)).collect();
    build_sections(&*st.library.read().await, &entries, now)
}

async fn hs_library(
    State(st): State<Arc<AppState>>,
    Query(q): Query<Filter>,
    headers: HeaderMap,
) -> Response {
    let base = base_url(&headers);
    let library: Vec<Value> =
        sections_for(&st, &q.filter, |v, _| format!("{base}/heresphere/{}", v.id))
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
) -> Response {
    let base = base_url(&headers);
    let scenes: Vec<Value> = sections_for(&st, &q.filter, |v, f| {
        json!({
            "title": stem(v),
            "videoLength": f.duration as u64,
            "thumbnailUrl": format!("{base}/api/media/thumb/{}", v.id),
            "video_url": format!("{base}/deovr/{}", v.id),
        })
    })
    .await
    .into_iter()
    .map(|(name, list)| json!({"name": name, "list": list}))
    .collect();
    cors(json!({"scenes": scenes, "authorized": "0"}), false)
}

/// What both scene details need: the video, its probed size, layout, tags.
struct Scene {
    v: Video,
    width: u32,
    height: u32,
    fmt: Format,
    duration: u64,
    tags: Vec<String>,
    rating: f64,
    favorite: bool,
}

async fn scene(st: &AppState, id: &str) -> Option<Scene> {
    let v = st.video(id).await?;
    // Probe this one video (cached), so the layout sees real dimensions.
    let (width, height) = match st.media.metadata(Path::new(&v.path)).await {
        Ok(m) if m.width > 0 && m.height > 0 => (m.width, m.height),
        _ => (1920, 1080),
    };
    let lib = st.library.read().await;
    let fmt = vr::detect(&lib, &v, Some((width, height)));
    let facts = Facts::of(&lib, &v, fmt);
    let (rating, favorite) = rating_entry(&lib, &v.path);
    Some(Scene {
        width,
        height,
        fmt,
        duration: facts.duration as u64,
        tags: scene_tags(&lib, &v, &facts),
        rating,
        favorite,
        v,
    })
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
) -> Response {
    let Some(s) = scene(&st, &id).await else {
        return not_found();
    };
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
            .map(|p| json!({"name": file_name(p), "url": format!("{base}/api/media/script/{id}")}))
            .collect();
    let subtitles: Vec<Value> = s.v.subtitle_path.iter().map(|p| {
        json!({"name": file_name(p), "language": "English", "url": format!("{base}/api/media/subtitle/{id}")})
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
            "thumbnailImage": format!("{base}/api/media/thumb/{id}"),
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
                "size": s.v.size, "url": format!("{base}/api/media/stream/{id}")}]}],
            // Headset edits and playback events aren't taken yet.
            "writeFavorite": false,
            "writeRating": false,
            "writeTags": false,
            "writeHSP": false,
        }),
        true,
    )
}

async fn deovr_scene(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
) -> Response {
    let Some(s) = scene(&st, &id).await else {
        return not_found();
    };
    let base = base_url(&headers);
    let id = &s.v.id;
    let fleshlight: Vec<Value> =
        s.v.funscript_path
            .iter()
            .map(|p| json!({"title": file_name(p), "url": format!("{base}/api/media/script/{id}")}))
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
            "thumbnailUrl": format!("{base}/api/media/thumb/{id}"),
            "encodings": [{"name": "h264", "videoSources": [{
                "resolution": s.height, "height": s.height, "width": s.width,
                "size": s.v.size, "url": format!("{base}/api/media/stream/{id}")}]}],
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let got = build_sections(&lib, &entries, now);
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
}
