//! The play queue (ported from the old Electron app): `player.userQueue` (the
//! user's additions), `player.queuePlayed` (`{path: epochMs}` played /
//! skipped stamps) and `player.queueAutoFill`; the auto-filled part is
//! computed on every read. API under `/api/remote/queue`.

use std::collections::HashSet;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, post};
use serde_json::{Map, Value, json};

use crate::data::{Library, now_ms};
use crate::edits::{HttpResult, NEEDS_REVIEW_PLAYLIST, applied, bad, body, video_path};
use crate::prefs::content_preferences as prefs;
use crate::remote::rating_entry;
use crate::scan::Video;
use crate::server::AppState;

const DAY_MS: f64 = 86_400_000.0;
const PLAYED_CAP: usize = 1000;
/// `QUEUE_AUTOFILL` of `shared/content-preferences.js`.
const SIZE: usize = 10;
const REPLAY_AFTER_MS: f64 = 30.0 * DAY_MS;

pub fn manual(lib: &Library) -> Vec<String> {
    lib.config["settings"]["player"]["userQueue"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str().map(str::to_string))
        .collect()
}

pub fn auto_fill_on(lib: &Library) -> bool {
    lib.config["settings"]["player"]["queueAutoFill"] != false
}

/// Paths of "Underage review" and "Needs review": never auto-filled.
fn review_paths(lib: &Library) -> HashSet<String> {
    let names = [
        prefs()["ageScreen"]["playlist"].as_str().unwrap_or(""),
        NEEDS_REVIEW_PLAYLIST,
    ];
    lib.config["playlists"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| {
            let n = p["name"].as_str().unwrap_or("").trim().to_lowercase();
            names.iter().any(|x| x.to_lowercase() == n)
        })
        .flat_map(|p| p["videoPaths"].as_array().cloned().unwrap_or_default())
        .filter_map(|p| p.as_str().map(str::to_string))
        .collect()
}

/// The whole queue in play order: the user's queue, then auto-fill.
pub fn queued_paths(lib: &Library, videos: &[Video], now: f64) -> Vec<String> {
    let mut q = manual(lib);
    q.extend(auto_fill(lib, videos, now));
    q
}

/// `autoFillQueue`: scripted videos not queued, not under review, not
/// rated at or below the delete rating (hearts excepted); not played
/// lately first (else the longest ago), unrated first, score high to low,
/// newest, path.
pub fn auto_fill(lib: &Library, videos: &[Video], now: f64) -> Vec<String> {
    if !auto_fill_on(lib) {
        return Vec::new();
    }
    let manual = manual(lib);
    let take = SIZE.saturating_sub(manual.len());
    let mut skip: HashSet<String> = review_paths(lib);
    skip.extend(manual);
    let delete_at = prefs()["ratings"]["deleteAtOrBelow"]
        .as_f64()
        .unwrap_or(0.0);
    let played = &lib.config["settings"]["player"]["queuePlayed"];
    let scores = &lib.lib()["preferenceScores"];
    let mut c: Vec<(u8, f64, u8, f64, f64, &str)> = Vec::new();
    for v in videos {
        if v.funscript_path.is_none() || !skip.insert(v.path.clone()) {
            continue;
        }
        let (rating, fav) = rating_entry(lib, &v.path);
        if rating > 0.0 && rating <= delete_at && !fav {
            continue;
        }
        let stamp = played[&v.path]
            .as_f64()
            .filter(|s| now - s < REPLAY_AFTER_MS);
        c.push((
            stamp.is_some() as u8,
            stamp.unwrap_or(0.0),
            (rating > 0.0) as u8,
            scores[&v.path].as_f64().unwrap_or(0.0),
            v.date_added,
            &v.path,
        ));
    }
    c.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.total_cmp(&b.1))
            .then(a.2.cmp(&b.2))
            .then(b.3.total_cmp(&a.3))
            .then(b.4.total_cmp(&a.4))
            .then(a.5.cmp(b.5))
    });
    c.into_iter().take(take).map(|x| x.5.to_string()).collect()
}

fn stamp_played(lib: &mut Library, path: &str, now: f64) {
    let p = lib
        .section_mut("player")
        .entry("queuePlayed")
        .or_insert(json!({}));
    let mut v: Vec<(String, f64)> = p
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| Some((k.clone(), v.as_f64()?)))
        .filter(|(k, _)| k != path)
        .chain([(path.to_string(), now)])
        .filter(|(_, at)| now - at < REPLAY_AFTER_MS)
        .collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v.truncate(PLAYED_CAP);
    *p = Value::Object(
        v.into_iter()
            .map(|(k, at)| (k, json!(at)))
            .collect::<Map<_, _>>(),
    );
}

pub enum Change<'a> {
    Add(&'a str, bool),
    Remove(&'a str),
    Played(&'a str),
    Clear,
    AutoFill(bool),
}

/// `applyQueueChange`: add (`next` also moves a queued one first), remove
/// (an unqueued one is stamped as skipped), played (out and stamped),
/// clear, auto-fill on/off.
pub fn apply(lib: &mut Library, change: Change, now: f64) {
    let mut m = manual(lib);
    match change {
        Change::Add(p, next) => {
            let queued = m.iter().any(|x| x == p);
            if next {
                m.retain(|x| x != p);
                m.insert(0, p.to_string());
            } else if !queued {
                m.push(p.to_string());
            }
        }
        Change::Remove(p) => {
            if m.iter().any(|x| x == p) {
                m.retain(|x| x != p);
            } else {
                stamp_played(lib, p, now);
            }
        }
        Change::Played(p) => {
            m.retain(|x| x != p);
            stamp_played(lib, p, now);
        }
        Change::Clear => m.clear(),
        Change::AutoFill(on) => {
            lib.section_mut("player")
                .insert("queueAutoFill".into(), json!(on));
        }
    }
    lib.section_mut("player")
        .insert("userQueue".into(), json!(m));
}

async fn queue_body(st: &AppState) -> Value {
    let lib = st.library.read().await;
    let snap = st.snapshot.read().await.clone();
    let ids = snap.ids_by_path();
    let item = |p: &String, auto: bool| {
        ids.get(p.as_str())
            .map(|id| json!({"videoId": id, "auto": auto}))
    };
    let items: Vec<Value> = manual(&lib)
        .iter()
        .filter_map(|p| item(p, false))
        .chain(
            auto_fill(&lib, &snap.videos, now_ms() as f64)
                .iter()
                .filter_map(|p| item(p, true)),
        )
        .collect();
    json!({"items": items, "autoFill": auto_fill_on(&lib)})
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/remote/queue", get(get_queue))
        .route("/api/remote/queue/add", post(add))
        .route("/api/remote/queue/remove", post(remove))
        .route("/api/remote/queue/played", post(played))
        .route("/api/remote/queue/clear", post(clear))
        .route("/api/remote/queue/auto-fill", post(set_auto_fill))
}

async fn get_queue(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(queue_body(&st).await)
}

async fn edit(st: &AppState, change: Change<'_>) -> HttpResult {
    apply(&mut *st.library.write().await, change, now_ms() as f64);
    let q = queue_body(st).await;
    applied(st, json!({"queue": q})).await
}

async fn add(State(st): State<Arc<AppState>>, b: Bytes) -> HttpResult {
    let b = body(&b);
    let path = video_path(&st, &b).await?;
    let next = match b["position"].as_str() {
        None if b["position"].is_null() => false,
        Some("end") => false,
        Some("next") => true,
        _ => {
            return Err(bad(
                StatusCode::BAD_REQUEST,
                "invalid position: end or next",
            ));
        }
    };
    edit(&st, Change::Add(&path, next)).await
}

async fn remove(State(st): State<Arc<AppState>>, b: Bytes) -> HttpResult {
    let path = video_path(&st, &body(&b)).await?;
    edit(&st, Change::Remove(&path)).await
}

async fn played(State(st): State<Arc<AppState>>, b: Bytes) -> HttpResult {
    let path = video_path(&st, &body(&b)).await?;
    edit(&st, Change::Played(&path)).await
}

async fn clear(State(st): State<Arc<AppState>>) -> HttpResult {
    edit(&st, Change::Clear).await
}

async fn set_auto_fill(State(st): State<Arc<AppState>>, b: Bytes) -> HttpResult {
    let Some(on) = body(&b)["enabled"].as_bool() else {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "enabled must be true or false",
        ));
    };
    edit(&st, Change::AutoFill(on)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(path: &str, added: f64) -> Video {
        serde_json::from_value(json!({
            "id": path, "name": path, "path": path, "ext": ".mp4", "fuzzyMatched": false,
            "variants": [], "axes": [], "sourceName": "", "dateAdded": added, "size": 0,
            "funscriptPath": format!("{path}.funscript"),
        }))
        .unwrap()
    }

    #[test]
    fn add_next_played_remove() {
        let mut l = Library::default();
        apply(&mut l, Change::Add("/a", false), 0.0);
        apply(&mut l, Change::Add("/b", false), 0.0);
        apply(&mut l, Change::Add("/b", true), 0.0);
        apply(&mut l, Change::Add("/a", false), 0.0);
        assert_eq!(manual(&l), ["/b", "/a"]);
        apply(&mut l, Change::Played("/b"), 5.0);
        assert_eq!(manual(&l), ["/a"]);
        assert_eq!(
            l.config["settings"]["player"]["queuePlayed"],
            json!({"/b": 5.0})
        );
        apply(&mut l, Change::Remove("/c"), 6.0);
        assert_eq!(
            l.config["settings"]["player"]["queuePlayed"]["/c"],
            json!(6.0)
        );
        apply(&mut l, Change::Clear, 0.0);
        assert!(manual(&l).is_empty());
        apply(&mut l, Change::AutoFill(false), 0.0);
        assert!(!auto_fill_on(&l));
    }

    #[test]
    fn auto_fill_selection() {
        let now = 100.0 * DAY_MS;
        let mut l = Library::default();
        let mut vids: Vec<Video> = [
            "/new", "/old", "/rated", "/low", "/review", "/played", "/queued", "/hi",
        ]
        .iter()
        .enumerate()
        .map(|(i, p)| video(p, i as f64))
        .collect();
        let mut noscript = video("/noscript", 99.0);
        noscript.funscript_path = None;
        vids.push(noscript);
        vids[0].date_added = 50.0;
        crate::edits::set_rating(&mut l, "/rated", Some(4.0), None);
        crate::edits::set_rating(&mut l, "/low", Some(2.0), None);
        crate::edits::edit_playlist(&mut l, "/review", None, Some("needs review"), true);
        *l.lib_mut("preferenceScores") = json!({"/hi": 9});
        apply(&mut l, Change::Played("/played"), now - DAY_MS);
        apply(&mut l, Change::Add("/queued", false), now);
        assert_eq!(
            auto_fill(&l, &vids, now),
            ["/hi", "/new", "/old", "/rated", "/played"]
        );
        apply(&mut l, Change::AutoFill(false), now);
        assert!(auto_fill(&l, &vids, now).is_empty());
    }
}
