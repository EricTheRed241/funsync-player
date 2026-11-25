//! The web remote's read API (`backend/routes/remote.py`): the same JSON,
//! so `backend/web-remote/` runs against this server unchanged.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::response::Json;
use axum::routing::{get, post};
use serde_json::{Value, json};

use crate::data::Library;
use crate::scan::Video;
use crate::server::AppState;
use crate::vr;

pub const WATCHED_COLLECTION_ID: &str = ":watched";

/// Post tags worth a chip (`backend/services/video_tags.py`).
const NOISE_TAGS: [&str; 13] = [
    "the-handy",
    "the-handy-2-pro",
    "osr2",
    "sr6",
    "funsr-pro",
    "syncbot",
    "multi-axis",
    "twist",
    "vr",
    "non-vr",
    "no-free-links",
    "straight",
    "",
];

pub fn chip_tags(tags: &Value) -> Vec<String> {
    let mut out: Vec<String> = tags
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !NOISE_TAGS.contains(&t.as_str()) && !t.starts_with("len-"))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// `(rating, favourite)`; ratings are half stars 0.5 … 5.
pub fn rating_entry(lib: &Library, path: &str) -> (f64, bool) {
    let e = &lib.lib()["ratings"][path];
    let r = e["rating"]
        .as_f64()
        .filter(|r| r.is_finite() && *r > 0.0)
        .map_or(0.0, |r| (r * 2.0).round() / 2.0);
    (r.min(5.0), e["favorite"] == true)
}

pub fn is_watched(lib: &Library, path: &str) -> bool {
    rating_entry(lib, path).0 > 0.0
}

/// The library setting "watched videos last" (on unless turned off).
pub fn watched_last(lib: &Library) -> bool {
    lib.lib()["watchedLast"] != false
}

pub fn duration(lib: &Library, path: &str) -> f64 {
    lib.lib()["durationCache"][path].as_f64().unwrap_or(0.0)
}

/// Every path that is watched, newest rating first.
pub fn watched_paths(lib: &Library) -> Vec<String> {
    let mut v: Vec<(f64, String)> = lib.lib()["ratings"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(p, _)| is_watched(lib, p))
        .map(|(p, e)| (e["updatedAt"].as_f64().unwrap_or(0.0), p.clone()))
        .collect();
    v.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    v.into_iter().map(|x| x.1).collect()
}

pub fn video_json(lib: &Library, v: &Video, categories: &HashMap<String, Vec<String>>) -> Value {
    let l = lib.lib();
    let stem = v.name.strip_suffix(&v.ext).unwrap_or(&v.name);
    let speed = v
        .funscript_path
        .as_deref()
        .map(|p| &l["speedStatsCache"][p]);
    let (rating, favorite) = rating_entry(lib, &v.path);
    let variants: Vec<Value> = v
        .variants
        .iter()
        .filter(|x| !x.label.trim().is_empty())
        .map(|x| json!({"label": x.label, "scriptUrl": format!("/api/media/script/{}?variant={}", v.id, x.label)}))
        .collect();
    json!({
        "id": v.id,
        "name": stem,
        "path": v.path,
        "hasFunscript": v.funscript_path.is_some(),
        "hasSubtitle": v.subtitle_path.is_some(),
        "duration": duration(lib, &v.path),
        "avgSpeed": speed.and_then(|s| s["avgSpeed"].as_f64()),
        "maxSpeed": speed.and_then(|s| s["maxSpeed"].as_f64()),
        "dateAdded": v.date_added,
        "sourceName": v.source_name,
        "streamUrl": format!("/api/media/stream/{}", v.id),
        "scriptUrl": v.funscript_path.as_ref().map(|_| format!("/api/media/script/{}", v.id)),
        "subtitleUrl": v.subtitle_path.as_ref().map(|_| format!("/api/media/subtitle/{}", v.id)),
        "thumbUrl": format!("/api/media/thumb/{}", v.id),
        "variants": variants,
        "manualVRType": l["manualVRType"].get(&v.path),
        "isVR": vr::is_vr(lib, v),
        "tags": chip_tags(&l["scriptTags"][&v.path]["tags"]),
        "categories": categories.get(&v.path).cloned().unwrap_or_default(),
        "rating": rating,
        "favorite": favorite,
        "watched": rating > 0.0,
        "score": l["preferenceScores"].get(&v.path),
    })
}

/// path → category names, in category order.
pub fn category_names(lib: &Library) -> HashMap<String, Vec<String>> {
    let names: HashMap<&str, &str> = lib.config["categories"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| Some((c["id"].as_str()?, c["name"].as_str()?.trim())))
        .filter(|(_, n)| !n.is_empty())
        .collect();
    let order: Vec<&str> = lib.config["categories"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["id"].as_str())
        .collect();
    lib.config["videoCategories"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(path, ids)| {
            let ids: Vec<&str> = ids
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let list = order
                .iter()
                .filter(|id| ids.contains(id))
                .filter_map(|id| names.get(id).map(|n| n.to_string()))
                .collect();
            (path.clone(), list)
        })
        .collect()
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/remote/videos", get(videos))
        .route("/api/remote/sources", get(sources))
        .route("/api/remote/playlists", get(playlists))
        .route("/api/remote/collections", get(collections))
        .route("/api/remote/categories", get(categories))
        .route("/api/remote/locale", get(locale))
        .route("/api/remote/page-load", post(ok))
        .route("/api/remote/seek-timing", post(ok))
        .route("/api/remote/request-rescan", post(request_rescan))
}

async fn ok() -> Json<Value> {
    Json(json!({"ok": true}))
}

async fn request_rescan(State(st): State<Arc<AppState>>) -> Json<Value> {
    let s = st.clone();
    tokio::spawn(async move { s.rescan().await });
    Json(json!({"seq": crate::data::now_ms()}))
}

async fn locale(State(st): State<Arc<AppState>>) -> Json<Value> {
    let lib = st.library.read().await;
    let lang = lib.config["settings"]["player"]["language"]
        .as_str()
        .unwrap_or("en")
        .to_string();
    Json(json!({"locale": lang}))
}

async fn videos(State(st): State<Arc<AppState>>) -> Json<Value> {
    let lib = st.library.read().await;
    let snap = st.snapshot.read().await.clone();
    let cats = category_names(&lib);
    let mut out: Vec<Value> = snap
        .videos
        .iter()
        .map(|v| video_json(&lib, v, &cats))
        .collect();
    out.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_lowercase());
    Json(json!({"videos": out, "watchedLast": watched_last(&lib)}))
}

async fn sources(State(st): State<Arc<AppState>>) -> Json<Value> {
    let lib = st.library.read().await;
    let mut out: Vec<Value> = lib
        .sources()
        .iter()
        .filter(|s| s.enabled)
        .map(|s| json!({"id": s.id, "name": s.name, "path": s.path}))
        .collect();
    out.sort_by_key(|s| s["name"].as_str().unwrap_or("").to_lowercase());
    Json(json!({"sources": out}))
}

/// `{id, name, videoCount, totalDuration, videoIds, …}` for a list of paths;
/// paths not in the library are dropped.
fn grouping(
    lib: &Library,
    ids: &HashMap<&str, &str>,
    g: &Value,
    paths: &[String],
    sink: bool,
) -> Value {
    let video_ids: Vec<&str> = paths
        .iter()
        .filter_map(|p| ids.get(p.as_str()).copied())
        .collect();
    let total: f64 = paths
        .iter()
        .filter(|p| ids.contains_key(p.as_str()))
        .map(|p| duration(lib, p))
        .sum();
    let mut out = json!({
        "id": g["id"],
        "name": g["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("(unnamed)"),
        "videoCount": video_ids.len(),
        "totalDuration": total,
        "videoIds": video_ids,
        "sinkWatched": sink,
    });
    for k in ["createdAt", "color", "kind"] {
        if let Some(v) = g.get(k) {
            out[k] = v.clone();
        }
    }
    out
}

fn paths_of(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str().map(str::to_string))
        .collect()
}

fn by_name(mut v: Vec<Value>) -> Vec<Value> {
    v.sort_by_key(|g| g["name"].as_str().unwrap_or("").to_lowercase());
    v
}

async fn playlists(State(st): State<Arc<AppState>>) -> Json<Value> {
    let lib = st.library.read().await;
    let snap = st.snapshot.read().await.clone();
    let ids = snap.ids_by_path();
    let list = lib.config["playlists"].as_array().into_iter().flatten();
    let out = list
        .map(|p| grouping(&lib, &ids, p, &paths_of(&p["videoPaths"]), false))
        .collect();
    Json(json!({"playlists": by_name(out)}))
}

async fn collections(State(st): State<Arc<AppState>>) -> Json<Value> {
    let lib = st.library.read().await;
    let snap = st.snapshot.read().await.clone();
    let ids = snap.ids_by_path();
    let sink = watched_last(&lib);
    let list = lib.lib()["collections"].as_array().into_iter().flatten();
    let mut out = by_name(
        list.map(|c| grouping(&lib, &ids, c, &paths_of(&c["videoPaths"]), sink))
            .collect(),
    );
    let watched = watched_paths(&lib);
    if watched.iter().any(|p| ids.contains_key(p.as_str())) {
        let g = json!({"id": WATCHED_COLLECTION_ID, "name": "Watched", "kind": "watched"});
        out.push(grouping(&lib, &ids, &g, &watched, false));
    }
    Json(json!({"collections": out}))
}

async fn categories(State(st): State<Arc<AppState>>) -> Json<Value> {
    let lib = st.library.read().await;
    let snap = st.snapshot.read().await.clone();
    let ids = snap.ids_by_path();
    let sink = watched_last(&lib);
    let mut members: HashMap<&str, Vec<String>> = HashMap::new();
    for (path, cat_ids) in lib.config["videoCategories"]
        .as_object()
        .into_iter()
        .flatten()
    {
        for id in cat_ids
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            members.entry(id).or_default().push(path.clone());
        }
    }
    let list = lib.config["categories"].as_array().into_iter().flatten();
    let out = list
        .map(|c| {
            let mut g = grouping(
                &lib,
                &ids,
                c,
                members
                    .get(c["id"].as_str().unwrap_or(""))
                    .map_or(&[][..], |v| v),
                sink,
            );
            g["color"] = c.get("color").cloned().unwrap_or(json!("#888"));
            g
        })
        .collect();
    Json(json!({"categories": by_name(out)}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_drop_noise_and_sort() {
        let t = json!(["Riding", "len-5-10", "osr2", "vr", "pov", "riding", 3]);
        assert_eq!(chip_tags(&t), ["pov", "riding"]);
    }

    #[test]
    fn watched_is_rated_newest_first() {
        let lib = Library {
            config: json!({"settings": {"library": {"ratings": {
                "/a": {"rating": 3, "updatedAt": 1}, "/b": {"rating": 4, "updatedAt": 2},
                "/c": {"favorite": true}}}}}),
            ..Default::default()
        };
        assert_eq!(watched_paths(&lib), ["/b", "/a"]);
        assert_eq!(rating_entry(&lib, "/c"), (0.0, true));
    }
}
