//! Settings the desktop app kept in its own windows: library sources (add,
//! remove, enable, reorder, overlap warning, drives coming and going), the
//! library's display setting, and `.funsync-backup` export / import
//! (`electron/data-export.js`: a zip with `config.json` + `metadata.json`).

use std::collections::HashSet;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::data::{SourceEntry, now_ms};
use crate::server::AppState;

/// Derived caches a backup leaves out (`BACKUP_BLACKLIST`, electron/data-backup.js).
const BACKUP_SKIP: [&str; 3] = ["speedStatsCache", "durationCache", "thumbnailCache"];

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/settings", get(get_settings))
        .route("/api/settings/sources", post(set_sources))
        .route("/api/settings/library", post(set_library))
        .route("/api/backup", get(export).post(import))
}

/// Pairs of sources where one folder holds the other (the scan would read
/// those videos twice; it drops the second copy).
pub fn overlaps(sources: &[SourceEntry]) -> Vec<(String, String)> {
    let norm = |p: &str| format!("{}/", p.trim_end_matches('/'));
    let mut out = Vec::new();
    for (i, a) in sources.iter().enumerate() {
        for b in &sources[i + 1..] {
            let (x, y) = (norm(&a.path), norm(&b.path));
            if x.starts_with(&y) || y.starts_with(&x) {
                out.push((a.name.clone(), b.name.clone()));
            }
        }
    }
    out
}

async fn get_settings(State(st): State<Arc<AppState>>) -> Json<Value> {
    let lib = st.library.read().await;
    let snap = st.snapshot.read().await.clone();
    let sources = lib.sources();
    let list: Vec<Value> = sources
        .iter()
        .map(|s| {
            json!({"id": s.id, "name": s.name, "path": s.path, "enabled": s.enabled,
                "present": Path::new(&s.path).is_dir(),
                "videos": snap.videos.iter().filter(|v| v.path.starts_with(&s.path)).count()})
        })
        .collect();
    Json(json!({
        "sources": list,
        "overlaps": overlaps(&sources),
        "watchedLast": crate::remote::watched_last(&lib),
        "libraryMaxVideos": crate::prefs::content_preferences()["libraryMaxVideos"],
        "ratings": crate::prefs::content_preferences()["ratings"],
    }))
}

#[derive(Deserialize)]
struct SourcesBody {
    sources: Vec<SourceEntry>,
}

async fn set_sources(State(st): State<Arc<AppState>>, Json(body): Json<SourcesBody>) -> Response {
    let mut list = body.sources;
    for s in &mut list {
        s.path = s.path.trim().trim_end_matches('/').to_string();
        if s.path.is_empty() || !s.path.starts_with('/') {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "a source needs an absolute path"})),
            )
                .into_response();
        }
        if s.name.trim().is_empty() {
            s.name = Path::new(&s.path)
                .file_name()
                .map_or("Library".into(), |n| n.to_string_lossy().into_owned());
        }
        if s.id.is_empty() {
            s.id = format!("{:x}", md5::compute(format!("{}{}", s.path, now_ms())));
        }
    }
    st.library
        .write()
        .await
        .section_mut("library")
        .insert("sources".into(), json!(list));
    if let Err(e) = st.save().await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response();
    }
    let s = st.clone();
    tokio::spawn(async move { s.rescan().await });
    get_settings(State(st)).await.into_response()
}

async fn set_library(State(st): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
    if let Some(w) = body.get("watchedLast").and_then(Value::as_bool) {
        st.library
            .write()
            .await
            .section_mut("library")
            .insert("watchedLast".into(), json!(w));
    }
    if let Err(e) = st.save().await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response();
    }
    get_settings(State(st)).await.into_response()
}

/// The backup zip of `config` (derived caches left out).
pub fn backup_zip(config: &Value) -> zip::result::ZipResult<Vec<u8>> {
    let mut cfg = config.clone();
    if let Some(lib) = cfg
        .pointer_mut("/settings/library")
        .and_then(Value::as_object_mut)
    {
        for k in BACKUP_SKIP {
            lib.remove(k);
        }
    }
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    w.start_file("config.json", opts)?;
    w.write_all(
        serde_json::to_string_pretty(&cfg)
            .unwrap_or_default()
            .as_bytes(),
    )?;
    w.start_file("metadata.json", opts)?;
    let meta = json!({"version": "1.0", "exportedAt": now_ms(), "appVersion": env!("CARGO_PKG_VERSION"), "app": "funsync-server"});
    w.write_all(meta.to_string().as_bytes())?;
    Ok(w.finish()?.into_inner())
}

/// `mergeConfig` of electron/data-export.js: settings deep-merged (the
/// backup wins), playlists and categories added when their name is new,
/// video categories unioned. `overwrite` takes the backup as it is.
pub fn merge_config(existing: &Value, imported: &Value, overwrite: bool) -> Value {
    if overwrite {
        return imported.clone();
    }
    fn deep(t: &mut Value, s: &Value) {
        match (t.as_object_mut(), s.as_object()) {
            (Some(t), Some(s)) => {
                for (k, v) in s {
                    match t.get_mut(k) {
                        Some(tv) if tv.is_object() && v.is_object() => deep(tv, v),
                        _ => {
                            t.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            _ => *t = s.clone(),
        }
    }
    let mut out = existing.clone();
    if !out.is_object() {
        out = json!({});
    }
    if let Some(s) = imported.get("settings") {
        deep(
            out.as_object_mut()
                .unwrap()
                .entry("settings")
                .or_insert(json!({})),
            s,
        );
    }
    for key in ["playlists", "categories"] {
        let list = out.as_object_mut().unwrap().entry(key).or_insert(json!([]));
        let names: HashSet<String> = list
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x["name"].as_str().map(str::to_string))
            .collect();
        for item in imported[key].as_array().into_iter().flatten() {
            if !item["name"].as_str().is_some_and(|n| names.contains(n))
                && let Some(a) = list.as_array_mut()
            {
                a.push(item.clone());
            }
        }
    }
    let vc = out
        .as_object_mut()
        .unwrap()
        .entry("videoCategories")
        .or_insert(json!({}));
    for (path, ids) in imported["videoCategories"]
        .as_object()
        .into_iter()
        .flatten()
    {
        let cur = vc
            .as_object_mut()
            .unwrap()
            .entry(path.clone())
            .or_insert(json!([]));
        for id in ids.as_array().into_iter().flatten() {
            if let Some(a) = cur.as_array_mut()
                && !a.contains(id)
            {
                a.push(id.clone());
            }
        }
    }
    out
}

async fn export(State(st): State<Arc<AppState>>) -> Response {
    let cfg = st.library.read().await.config.clone();
    match backup_zip(&cfg) {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "application/zip"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"funsync-backup.zip\"",
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct ImportQuery {
    mode: Option<String>,
}

async fn import(
    State(st): State<Arc<AppState>>,
    Query(q): Query<ImportQuery>,
    body: Bytes,
) -> Response {
    let bad = |m: &str| (StatusCode::BAD_REQUEST, Json(json!({"error": m}))).into_response();
    let Ok(mut zip) = zip::ZipArchive::new(Cursor::new(body.to_vec())) else {
        return bad("not a zip file");
    };
    let mut text = String::new();
    let read = zip
        .by_name("config.json")
        .map(|mut f| f.read_to_string(&mut text));
    if !matches!(read, Ok(Ok(_))) {
        return bad("Invalid backup: missing config.json");
    }
    let Ok(imported) = serde_json::from_str::<Value>(&text) else {
        return bad("Invalid backup: corrupt config.json");
    };
    {
        let mut lib = st.library.write().await;
        lib.config = merge_config(
            &lib.config,
            &imported,
            q.mode.as_deref() == Some("overwrite"),
        );
    }
    if let Err(e) = st.save().await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response();
    }
    let s = st.clone();
    tokio::spawn(async move { s.rescan().await });
    Json(json!({"ok": true})).into_response()
}

/// Rescan when a source folder appears or disappears (a drive plugged in or
/// out), checked every 10 s.
pub fn watch_drives(st: Arc<AppState>) {
    tokio::spawn(async move {
        let mut last: Option<Vec<bool>> = None;
        loop {
            let present: Vec<bool> = st
                .library
                .read()
                .await
                .sources()
                .iter()
                .map(|s| Path::new(&s.path).is_dir())
                .collect();
            if last.as_ref().is_some_and(|l| *l != present) {
                let _ = st.rescan().await;
            }
            last = Some(present);
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(name: &str, path: &str) -> SourceEntry {
        SourceEntry {
            id: name.into(),
            name: name.into(),
            path: path.into(),
            enabled: true,
        }
    }

    #[test]
    fn overlap_is_containment_not_prefix() {
        let s = [
            src("a", "/v/GVP"),
            src("b", "/v/GVP/EroScripts"),
            src("c", "/v/GVP2"),
        ];
        assert_eq!(overlaps(&s), [("a".to_string(), "b".to_string())]);
    }

    #[test]
    fn backup_round_trip_and_merge() {
        let cfg = json!({"settings": {"library": {"durationCache": {"/a": 1}, "ratings": {"/a": {"rating": 4}}}},
            "playlists": [{"name": "P", "videoPaths": ["/a"]}], "videoCategories": {"/a": ["x"]}});
        let bytes = backup_zip(&cfg).unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut text = String::new();
        z.by_name("config.json")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        let back: Value = serde_json::from_str(&text).unwrap();
        assert!(back.pointer("/settings/library/durationCache").is_none());
        let existing = json!({"settings": {"library": {"ratings": {"/b": {"rating": 2}}}},
            "playlists": [{"name": "P", "videoPaths": []}], "videoCategories": {"/a": ["y"]}});
        let m = merge_config(&existing, &back, false);
        assert_eq!(
            m["settings"]["library"]["ratings"]
                .as_object()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            m["playlists"].as_array().unwrap().len(),
            1,
            "same name is skipped"
        );
        assert_eq!(m["videoCategories"]["/a"], json!(["y", "x"]));
        assert_eq!(merge_config(&existing, &back, true), back);
    }
}
