//! The web remote's write API (`backend/routes/remote.py` +
//! `backend/services/library_edits.py`): ratings, playlists, tag scores and
//! favourite artists, applied straight to `Library.config` (the old app
//! queued them for the desktop). Every edit goes through
//! `AppState::library_changed`. Nothing here deletes a file: a rating
//! ≤ 2★ only stores the rating.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use serde_json::{Map, Value, json};

use crate::data::{Library, now_ms};
use crate::prefs::{
    artist_of, canonical_artist, content_preferences as prefs, effective_tag_scores, fold,
    normalize_tag_score,
};
use crate::remote::{duration, rating_entry};
use crate::server::AppState;

pub const NEEDS_REVIEW_PLAYLIST: &str = "Needs review";
pub type HttpResult = Result<Json<Value>, Response>;

pub fn bad(code: StatusCode, detail: impl Into<String>) -> Response {
    (code, Json(json!({"detail": detail.into()}))).into_response()
}

/// A request body as JSON; anything else is `null` (fields then missing).
pub fn body(b: &Bytes) -> Value {
    serde_json::from_slice(b).unwrap_or(Value::Null)
}

/// The path of `body.videoId`, else 404.
pub async fn video_path(st: &AppState, b: &Value) -> Result<String, Response> {
    let id = b["videoId"].as_str().unwrap_or("");
    match st.video(id).await {
        Some(v) => Ok(v.path),
        None => Err(bad(StatusCode::NOT_FOUND, "Unknown video")),
    }
}

/// What the old routes answered once an edit was queued; here it is
/// applied and saved.
pub async fn applied(st: &AppState, extra: Value) -> HttpResult {
    st.library_changed()
        .await
        .map_err(|e| bad(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let mut out = json!({"ok": true, "seq": now_ms(), "delivered": true});
    if let (Some(o), Some(x)) = (out.as_object_mut(), extra.as_object()) {
        o.extend(x.clone());
    }
    Ok(Json(out))
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|n| n.is_finite())
}

/// 0–5 in half stars; anything else (off-step, out of range) is rejected.
pub fn normalize_rating(v: &Value) -> Option<f64> {
    number(v).filter(|r| (0.0..=5.0).contains(r) && (r * 2.0).fract() == 0.0)
}

pub fn tag_key(t: &str) -> String {
    t.trim().to_lowercase()
}

fn same_name(a: &Value, b: &str) -> bool {
    a.as_str()
        .is_some_and(|a| a.trim().to_lowercase() == b.trim().to_lowercase())
}

/// Set a rating and/or the heart (`ratings.js::applyRatingChange`); rating 0
/// without a heart clears the entry. A rating > 0 takes the video out of
/// Needs review (`DataService.setRating`).
pub fn set_rating(lib: &mut Library, path: &str, rating: Option<f64>, favorite: Option<bool>) {
    let (cur, fav) = rating_entry(lib, path);
    let (rating, favorite) = (rating.unwrap_or(cur), favorite.unwrap_or(fav));
    let ratings = lib.lib_mut("ratings");
    if !ratings.is_object() {
        *ratings = json!({});
    }
    let map = ratings.as_object_mut().unwrap();
    if rating == 0.0 && !favorite {
        map.remove(path);
    } else {
        map.insert(
            path.into(),
            json!({"rating": rating, "favorite": favorite, "updatedAt": now_ms()}),
        );
    }
    if rating > 0.0 {
        for p in lib.top_array_mut("playlists") {
            if same_name(&p["name"], NEEDS_REVIEW_PLAYLIST)
                && let Some(v) = p["videoPaths"].as_array_mut()
            {
                v.retain(|x| x != path);
            }
        }
    }
}

/// Add `path` to the playlist with `id`, else the one named `name`
/// (any case), else a new playlist `name`; or remove it. False when there
/// is no such playlist to add to or take from.
pub fn edit_playlist(
    lib: &mut Library,
    path: &str,
    id: Option<&str>,
    name: Option<&str>,
    add: bool,
) -> bool {
    let lists = lib.top_array_mut("playlists");
    let found = id
        .and_then(|id| lists.iter().position(|p| p["id"] == id))
        .or_else(|| name.and_then(|n| lists.iter().position(|p| same_name(&p["name"], n))));
    let i = match (found, name) {
        (Some(i), _) => i,
        (None, Some(n)) if add => {
            let now = now_ms();
            let h = format!("{:x}", md5::compute(format!("{n}{now}")));
            lists.push(json!({
                "id": format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..]),
                "name": n,
                "createdAt": now,
                "videoPaths": [],
            }));
            lists.len() - 1
        }
        _ => return false,
    };
    let p = &mut lists[i]["videoPaths"];
    if !p.is_array() {
        *p = json!([]);
    }
    let v = p.as_array_mut().unwrap();
    if !add {
        v.retain(|x| x != path);
    } else if !v.iter().any(|x| x == path) {
        v.push(json!(path));
    }
    true
}

/// Every grouped tag's starting score (`DEFAULT_TAG_SCORES`) and group name.
fn tag_groups() -> Vec<(String, f64, String)> {
    let mut out = Vec::new();
    for g in prefs()["tagScoreGroups"].as_array().into_iter().flatten() {
        for t in g["tags"].as_array().into_iter().flatten() {
            let name = g["name"].as_str().unwrap_or("").to_string();
            out.push((
                tag_key(t.as_str().unwrap_or("")),
                g["score"].as_f64().unwrap_or(0.0),
                name,
            ));
        }
    }
    out
}

fn default_tag_score(key: &str) -> f64 {
    tag_groups()
        .into_iter()
        .find(|g| g.0 == key)
        .map_or(0.0, |g| g.1)
}

/// `library.tagScores` keeps only changes from the starting score.
pub fn set_tag_score(lib: &mut Library, key: &str, score: f64) {
    let m = lib.lib_mut("tagScores");
    if !m.is_object() {
        *m = json!({});
    }
    let m = m.as_object_mut().unwrap();
    if score == default_tag_score(key) {
        m.remove(key);
    } else {
        m.insert(key.into(), json!(score));
    }
}

fn artist_key(name: &str) -> String {
    fold(&canonical_artist(name.trim()))
}

/// `library.favouriteArtists` keeps only changes from FAVOURITES.
pub fn set_favourite_artist(lib: &mut Library, name: &str, favourite: bool) -> String {
    let artist = canonical_artist(name.trim());
    let key = fold(&artist);
    let built_in = prefs()["favourites"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|n| artist_key(n) == key);
    let m = lib.lib_mut("favouriteArtists");
    if !m.is_object() {
        *m = json!({});
    }
    let m = m.as_object_mut().unwrap();
    m.retain(|n, _| artist_key(n) != key);
    if favourite != built_in {
        m.insert(artist.clone(), json!(favourite));
    }
    artist
}

fn default_favourite_score() -> f64 {
    prefs()["ranking"]["favouriteArtist"]
        .as_f64()
        .unwrap_or(0.0)
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/remote/rating", post(rating))
        .route("/api/remote/playlists/add-video", post(playlist_add))
        .route("/api/remote/playlists/remove-video", post(playlist_remove))
        .route("/api/remote/tag-scores", get(tag_scores))
        .route("/api/remote/tag-score", post(tag_score))
        .route("/api/remote/recalculate-scores", post(recalculate))
        .route("/api/remote/artists", get(artists))
        .route("/api/remote/favourite-artist", post(favourite_artist))
        .route("/api/remote/favourite-score", post(favourite_score))
}

async fn rating(State(st): State<Arc<AppState>>, b: Bytes) -> HttpResult {
    let b = body(&b);
    let path = video_path(&st, &b).await?;
    let rating = match &b["rating"] {
        Value::Null => None,
        v => Some(normalize_rating(v).ok_or_else(|| {
            bad(
                StatusCode::BAD_REQUEST,
                format!("invalid rating {v}: 0-5 in steps of 0.5"),
            )
        })?),
    };
    let favorite = match &b["favorite"] {
        Value::Null => None,
        Value::Bool(f) => Some(*f),
        _ => {
            return Err(bad(
                StatusCode::BAD_REQUEST,
                "favorite must be true or false",
            ));
        }
    };
    if rating.is_none() && favorite.is_none() {
        return Err(bad(StatusCode::BAD_REQUEST, "nothing to change"));
    }
    let (r, f) = {
        let mut lib = st.library.write().await;
        set_rating(&mut lib, &path, rating, favorite);
        rating_entry(&lib, &path)
    };
    applied(&st, json!({"rating": r, "favorite": f})).await
}

async fn playlist_edit(st: &AppState, b: Bytes, add: bool) -> HttpResult {
    let b = body(&b);
    let path = video_path(st, &b).await?;
    let id = b["playlistId"].as_str().filter(|s| !s.is_empty());
    let name = b["name"].as_str().map(str::trim).filter(|s| !s.is_empty());
    if id.is_none() && name.is_none() {
        return Err(bad(StatusCode::BAD_REQUEST, "playlistId or name required"));
    }
    edit_playlist(&mut *st.library.write().await, &path, id, name, add);
    applied(st, json!({})).await
}

async fn playlist_add(State(st): State<Arc<AppState>>, b: Bytes) -> HttpResult {
    playlist_edit(&st, b, true).await
}

async fn playlist_remove(State(st): State<Arc<AppState>>, b: Bytes) -> HttpResult {
    playlist_edit(&st, b, false).await
}

async fn tag_scores(State(st): State<Arc<AppState>>) -> Json<Value> {
    let lib = st.library.read().await;
    let groups: Map<String, Value> = tag_groups()
        .into_iter()
        .map(|(k, _, g)| (k, json!(g)))
        .collect();
    let r = &prefs()["ranking"];
    Json(json!({
        "scores": effective_tag_scores(&lib.lib()["tagScores"])
            .into_iter()
            .map(|(k, s)| (k, json!(s)))
            .collect::<Map<String, Value>>(),
        "pending": [],
        "recalculating": false,
        "groups": groups,
        "rules": {
            "multiAxis": r["multiAxis"], "vr": r["vr"], "artistInLibrary": r["artistInLibrary"],
            "favouriteArtist": r["favouriteArtist"], "likes": r["likesMax"],
            "hentaiOnly": -r["hentaiOnly"].as_f64().unwrap_or(0.0),
        },
    }))
}

async fn tag_score(State(st): State<Arc<AppState>>, b: Bytes) -> HttpResult {
    let b = body(&b);
    let key = tag_key(b["tag"].as_str().unwrap_or(""));
    if key.is_empty() {
        return Err(bad(StatusCode::BAD_REQUEST, "tag required"));
    }
    let score = normalize_tag_score(&b["score"]).ok_or_else(|| {
        bad(
            StatusCode::BAD_REQUEST,
            format!(
                "invalid score {}: a number, -10 to 10 in steps of 0.5",
                b["score"]
            ),
        )
    })?;
    set_tag_score(&mut *st.library.write().await, &key, score);
    applied(&st, json!({"tag": key, "score": score})).await
}

async fn recalculate(State(st): State<Arc<AppState>>) -> HttpResult {
    applied(&st, json!({})).await
}

async fn artists(State(st): State<Arc<AppState>>) -> Json<Value> {
    let lib = st.library.read().await;
    let snap = st.snapshot.read().await.clone();
    let sources: Vec<String> = lib.sources().into_iter().map(|s| s.path).collect();
    // key → (spelling counts, video ids, total duration)
    type Group = (HashMap<String, usize>, Vec<String>, f64);
    let mut groups: HashMap<String, Group> = HashMap::new();
    for v in &snap.videos {
        if let Some(a) = artist_of(&v.path, &sources) {
            let g = groups.entry(fold(&a)).or_default();
            *g.0.entry(a).or_default() += 1;
            g.1.push(v.id.clone());
            g.2 += duration(&lib, &v.path);
        }
    }
    let favs: Vec<String> = crate::prefs::favourites(&lib)
        .into_iter()
        .map(|f| f.1)
        .collect();
    let fav_keys: Vec<String> = favs.iter().map(|n| artist_key(n)).collect();
    let entry = |key: String, name: String, ids: Vec<String>, total: f64| {
        json!({
            "id": key, "name": name, "videoCount": ids.len(), "totalDuration": total,
            "videoIds": ids, "favourite": fav_keys.contains(&key), "pending": false,
        })
    };
    let mut out: Vec<Value> = Vec::new();
    for (key, (spellings, ids, total)) in groups.iter() {
        let name = spellings
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
            .map(|x| x.0.clone())
            .unwrap_or_default();
        out.push(entry(key.clone(), name, ids.clone(), *total));
    }
    for n in &favs {
        let key = artist_key(n);
        if !groups.contains_key(&key) && !out.iter().any(|a| a["id"] == key.as_str()) {
            out.push(entry(key, n.clone(), Vec::new(), 0.0));
        }
    }
    out.sort_by_key(|a| {
        (
            a["favourite"] != true,
            fold(a["name"].as_str().unwrap_or("")),
        )
    });
    let def = default_favourite_score();
    Json(json!({
        "artists": out,
        "score": normalize_tag_score(&lib.lib()["favouriteArtistScore"]).unwrap_or(def),
        "defaultScore": def,
        "scorePending": false,
    }))
}

async fn favourite_artist(State(st): State<Arc<AppState>>, b: Bytes) -> HttpResult {
    let b = body(&b);
    let name = b["artist"].as_str().map(str::trim).unwrap_or("");
    if name.is_empty() {
        return Err(bad(StatusCode::BAD_REQUEST, "artist required"));
    }
    let Some(fav) = b["favourite"].as_bool() else {
        return Err(bad(
            StatusCode::BAD_REQUEST,
            "favourite must be true or false",
        ));
    };
    set_favourite_artist(&mut *st.library.write().await, name, fav);
    applied(&st, json!({"artist": name, "favourite": fav})).await
}

async fn favourite_score(State(st): State<Arc<AppState>>, b: Bytes) -> HttpResult {
    let b = body(&b);
    let score = normalize_tag_score(&b["score"]).ok_or_else(|| {
        bad(
            StatusCode::BAD_REQUEST,
            format!(
                "invalid score {}: a number, -10 to 10 in steps of 0.5",
                b["score"]
            ),
        )
    })?;
    let stored = if score == default_favourite_score() {
        Value::Null
    } else {
        json!(score)
    };
    *st.library.write().await.lib_mut("favouriteArtistScore") = stored;
    applied(&st, json!({"score": score})).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib() -> Library {
        Library::default()
    }

    #[test]
    fn rating_normalization() {
        assert_eq!(normalize_rating(&json!(4.5)), Some(4.5));
        assert_eq!(normalize_rating(&json!("3")), Some(3.0));
        assert_eq!(normalize_rating(&json!(0)), Some(0.0));
        for bad in [
            json!(4.3),
            json!(5.5),
            json!(-1),
            json!(true),
            json!("x"),
            Value::Null,
        ] {
            assert_eq!(normalize_rating(&bad), None, "{bad}");
        }
    }

    #[test]
    fn rating_stores_keeps_low_ratings_and_leaves_needs_review() {
        let mut l = lib();
        edit_playlist(&mut l, "/v/a.mp4", None, Some("Needs review"), true);
        set_rating(&mut l, "/v/a.mp4", Some(1.0), None);
        assert_eq!(rating_entry(&l, "/v/a.mp4"), (1.0, false));
        assert_eq!(l.config["playlists"][0]["videoPaths"], json!([]));
        set_rating(&mut l, "/v/a.mp4", None, Some(true));
        assert_eq!(rating_entry(&l, "/v/a.mp4"), (1.0, true));
        set_rating(&mut l, "/v/a.mp4", Some(0.0), Some(false));
        assert!(l.lib()["ratings"].get("/v/a.mp4").is_none());
    }

    #[test]
    fn playlist_create_add_remove() {
        let mut l = lib();
        assert!(edit_playlist(&mut l, "/a", None, Some("Mine"), true));
        assert!(edit_playlist(&mut l, "/a", None, Some("mine "), true));
        let p = l.config["playlists"][0].clone();
        assert_eq!(p["name"], "Mine");
        assert_eq!(p["videoPaths"], json!(["/a"]));
        assert_eq!(p["id"].as_str().unwrap().len(), 36);
        let id = p["id"].as_str().unwrap().to_string();
        assert!(edit_playlist(&mut l, "/b", Some(&id), None, true));
        assert!(edit_playlist(&mut l, "/a", Some(&id), None, false));
        assert_eq!(l.config["playlists"][0]["videoPaths"], json!(["/b"]));
        assert!(!edit_playlist(&mut l, "/a", None, Some("Other"), false));
        assert_eq!(l.config["playlists"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn tag_score_clamp_and_storage() {
        let mut l = lib();
        set_tag_score(&mut l, "riding", 0.0);
        set_tag_score(&mut l, "solo", 3.0);
        let eff = effective_tag_scores(&l.lib()["tagScores"]);
        assert!(!eff.contains_key("riding"));
        assert_eq!(eff["solo"], 3.0);
        assert_eq!(eff["cowgirl"], 2.0);
        set_tag_score(&mut l, "riding", 2.0);
        assert!(l.lib()["tagScores"].get("riding").is_none());
    }

    #[test]
    fn artists_and_favourites() {
        let mut l = lib();
        set_favourite_artist(&mut l, "Someone New", true);
        let favs = crate::prefs::favourites(&l);
        assert!(favs.iter().any(|f| f.1 == "Someone New"));
        set_favourite_artist(&mut l, "someone new", false);
        assert_eq!(l.lib()["favouriteArtists"], json!({}));
    }
}
