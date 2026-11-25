//! Manual script association (`library.associations[videoPath]`, the
//! renderer's association-shape.js): `{active: single|multi|custom, single,
//! multi: {main, axes}, custom}`. An active `single` / `multi` entry whose
//! files exist replaces the scan's pairing; `custom` is device routing
//! (devices::routing). The Associate page sets entries; suggestions are
//! ranked by the scan's fuzzy title score.

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::scan::{AXIS_SUFFIXES, Axis, Video, fuzzy_score};
use crate::server::AppState;

/// Apply every active `single` / `multi` association to the scanned videos.
pub fn apply(associations: &Value, videos: &mut [Video]) {
    let exists = |p: &str| Path::new(p).is_file();
    for v in videos.iter_mut() {
        let e = &associations[&v.path];
        match e["active"].as_str() {
            Some("single") => {
                if let Some(p) = e["single"].as_str().filter(|p| exists(p)) {
                    v.funscript_path = Some(p.to_string());
                }
            }
            Some("multi") => {
                if let Some(p) = e["multi"]["main"].as_str().filter(|p| exists(p)) {
                    v.funscript_path = Some(p.to_string());
                }
                let axes: Vec<Axis> = e["multi"]["axes"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter_map(|(axis, p)| {
                        let p = p.as_str().filter(|p| exists(p))?;
                        Some(Axis {
                            axis: axis.clone(),
                            path: p.to_string(),
                            generated: p.contains(".generated."),
                        })
                    })
                    .collect();
                if !axes.is_empty() {
                    v.axes = axes;
                }
            }
            _ => {}
        }
    }
}

/// The stored entry with `active` and its slot set, the other slots kept
/// (association-shape.js `buildAssociationEntry`, mirror fields included).
pub fn entry(
    old: &Value,
    active: Option<&str>,
    single: Option<&str>,
    multi: Option<Value>,
) -> Value {
    let mut e = Map::new();
    e.insert("active".into(), json!(active));
    e.insert(
        "single".into(),
        single.map_or_else(|| old["single"].clone(), |s| json!(s)),
    );
    e.insert(
        "multi".into(),
        multi.unwrap_or_else(|| old["multi"].clone()),
    );
    e.insert("custom".into(), old["custom"].clone());
    for k in ["single", "multi", "custom"] {
        if e[k].is_null() {
            e.insert(k.into(), Value::Null);
        }
    }
    if active == Some("custom") && !e["custom"].is_null() {
        e.insert("mode".into(), json!("custom"));
        e.insert("routes".into(), e["custom"]["routes"].clone());
    } else if active == Some("multi") && !e["multi"].is_null() {
        e.insert("main".into(), e["multi"]["main"].clone());
        e.insert(
            "axes".into(),
            e["multi"].get("axes").cloned().unwrap_or(json!({})),
        );
        e.insert(
            "buttplugVib".into(),
            json!(e["multi"]["buttplugVib"] == true),
        );
    }
    Value::Object(e)
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/api/associate/{id}", get(show).post(set))
}

fn stem(p: &str) -> &str {
    let name = p.rsplit('/').next().unwrap_or(p);
    name.strip_suffix(".funscript").unwrap_or(name)
}

async fn show(State(st): State<Arc<AppState>>, UrlPath(id): UrlPath<String>) -> Response {
    let Some(v) = st.video(&id).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no such video"})),
        )
            .into_response();
    };
    let snap = st.snapshot.read().await.clone();
    let vstem = v.name.strip_suffix(&v.ext).unwrap_or(&v.name).to_string();
    let dir = Path::new(&v.path)
        .parent()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut ranked: Vec<(u32, bool, &String)> = snap
        .funscripts
        .iter()
        .map(|p| {
            (
                fuzzy_score(&vstem, stem(p)),
                p.starts_with(&format!("{dir}/")),
                p,
            )
        })
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)).then(a.2.cmp(b.2)));
    let suggestions: Vec<Value> = ranked
        .iter()
        .filter(|(s, local, _)| *local || *s > 0)
        .take(30)
        .map(|(s, local, p)| json!({"path": p, "score": s, "sameFolder": local}))
        .collect();
    let entry = st.library.read().await.lib()["associations"][&v.path].clone();
    Json(json!({
        "video": {"id": v.id, "name": v.name, "path": v.path},
        "current": {"funscriptPath": v.funscript_path, "axes": v.axes},
        "entry": entry,
        "axisNames": AXIS_SUFFIXES,
        "suggestions": suggestions,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct SetBody {
    /// `single`, `multi`, or null to clear (back to the scan's pairing).
    active: Option<String>,
    single: Option<String>,
    main: Option<String>,
    #[serde(default)]
    axes: Map<String, Value>,
}

async fn set(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Json(b): Json<SetBody>,
) -> Response {
    let Some(v) = st.video(&id).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no such video"})),
        )
            .into_response();
    };
    let known: Vec<String> = st.snapshot.read().await.funscripts.clone();
    let ok = |p: &Option<String>| p.as_ref().is_none_or(|p| known.contains(p));
    let axes_ok = b
        .axes
        .values()
        .all(|p| p.as_str().is_some_and(|p| known.contains(&p.to_string())));
    if !ok(&b.single) || !ok(&b.main) || !axes_ok {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "not a funscript of the library"})),
        )
            .into_response();
    }
    {
        let mut lib = st.library.write().await;
        let old = lib.lib()["associations"][&v.path].clone();
        let multi = (b.active.as_deref() == Some("multi"))
            .then(|| json!({"main": b.main, "axes": b.axes, "buttplugVib": false}));
        let e = entry(&old, b.active.as_deref(), b.single.as_deref(), multi);
        lib.lib_mut("associations")[&v.path] = e;
    }
    if let Err(e) = st.library_changed().await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response();
    }
    let s = st.clone();
    tokio::spawn(async move { s.rescan().await });
    Json(json!({"ok": true})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(path: &str) -> Video {
        serde_json::from_value(json!({"id": "x", "name": "V.mp4", "path": path, "ext": ".mp4",
            "funscriptPath": null, "fuzzyMatched": false, "subtitlePath": null, "variants": [], "axes": [],
            "sourceName": "", "dateAdded": 0.0, "size": 0})).unwrap()
    }

    #[test]
    fn active_entries_override_the_scan_and_missing_files_are_ignored() {
        let t = tempfile::tempdir().unwrap();
        let main = t.path().join("Other.funscript");
        let twist = t.path().join("Other.generated.twist.funscript");
        std::fs::write(&main, "{}").unwrap();
        std::fs::write(&twist, "{}").unwrap();
        let (m, tw) = (
            main.to_string_lossy().into_owned(),
            twist.to_string_lossy().into_owned(),
        );
        let assoc = json!({
            "/a.mp4": {"active": "multi", "multi": {"main": m, "axes": {"twist": tw, "roll": "/gone.funscript"}}},
            "/b.mp4": {"active": "single", "single": "/gone.funscript"},
            "/c.mp4": {"active": "custom", "single": m},
        });
        let mut vids = vec![video("/a.mp4"), video("/b.mp4"), video("/c.mp4")];
        apply(&assoc, &mut vids);
        assert_eq!(vids[0].funscript_path.as_deref(), Some(m.as_str()));
        assert_eq!(vids[0].axes.len(), 1);
        assert!(vids[0].axes[0].generated);
        assert!(vids[1].funscript_path.is_none());
        assert!(vids[2].funscript_path.is_none());
    }

    #[test]
    fn entry_keeps_other_slots_and_mirrors_multi() {
        let old = json!({"active": "custom", "custom": {"routes": [1]}, "single": "/s"});
        let e = entry(
            &old,
            Some("multi"),
            None,
            Some(json!({"main": "/m", "axes": {"roll": "/r"}})),
        );
        assert_eq!(e["custom"]["routes"], json!([1]));
        assert_eq!(e["single"], "/s");
        assert_eq!(e["main"], "/m");
        assert_eq!(e["axes"]["roll"], "/r");
        let cleared = entry(&e, None, None, None);
        assert_eq!(cleared["active"], Value::Null);
        assert_eq!(cleared["multi"]["main"], "/m");
    }
}
