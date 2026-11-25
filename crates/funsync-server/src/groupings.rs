//! Playlists, categories and collections as the desktop managed them:
//! create, rename (and colour), delete, and add / remove / reorder videos,
//! several at once (multi-select). Same storage as the old app:
//! `playlists` and `settings.library.collections` hold `videoPaths`;
//! `categories` hold `{id, name, color}` with membership in `videoCategories`.

use std::collections::HashSet;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::post;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::data::{Library, now_ms};
use crate::server::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/groupings/{kind}", post(create))
        .route("/api/groupings/{kind}/{id}", post(update).delete(remove))
        .route("/api/groupings/{kind}/{id}/videos", post(videos))
}

/// The list a kind lives in.
fn list_mut<'a>(lib: &'a mut Library, kind: &str) -> Option<&'a mut Vec<Value>> {
    match kind {
        "playlists" | "categories" => Some(lib.top_array_mut(kind)),
        "collections" => {
            let v = lib
                .section_mut("library")
                .entry("collections")
                .or_insert(json!([]));
            if !v.is_array() {
                *v = json!([]);
            }
            v.as_array_mut()
        }
        _ => None,
    }
}

fn new_id() -> String {
    let h = format!(
        "{:x}",
        md5::compute(format!("{}{:?}", now_ms(), std::thread::current().id()))
    );
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

pub fn create_in(lib: &mut Library, kind: &str, name: &str, color: Option<&str>) -> Option<Value> {
    let mut g = json!({"id": new_id(), "name": name.trim(), "createdAt": now_ms()});
    if kind == "categories" {
        g["color"] = json!(color.unwrap_or("#888"));
    } else {
        g["videoPaths"] = json!([]);
    }
    list_mut(lib, kind)?.push(g.clone());
    Some(g)
}

pub fn delete_in(lib: &mut Library, kind: &str, id: &str) -> bool {
    let Some(list) = list_mut(lib, kind) else {
        return false;
    };
    let before = list.len();
    list.retain(|g| g["id"] != id);
    let gone = list.len() < before;
    if gone
        && kind == "categories"
        && let Some(map) = lib
            .config
            .get_mut("videoCategories")
            .and_then(Value::as_object_mut)
    {
        for ids in map.values_mut().filter_map(Value::as_array_mut) {
            ids.retain(|x| x != id);
        }
        map.retain(|_, ids| ids.as_array().is_some_and(|a| !a.is_empty()));
    }
    gone
}

/// Add and remove `paths`; `order` (paths) puts a playlist / collection in
/// that order (paths it doesn't name keep their place after them).
pub fn set_members(
    lib: &mut Library,
    kind: &str,
    id: &str,
    add: &[String],
    remove: &[String],
    order: Option<&[String]>,
) -> bool {
    if kind == "categories" {
        if !list_mut(lib, kind).is_some_and(|l| l.iter().any(|g| g["id"] == id)) {
            return false;
        }
        if !lib.config["videoCategories"].is_object() {
            lib.config["videoCategories"] = json!({});
        }
        let map = lib.config["videoCategories"].as_object_mut().unwrap();
        for p in add {
            let ids = map.entry(p.clone()).or_insert(json!([]));
            if let Some(a) = ids.as_array_mut()
                && !a.iter().any(|x| x == id)
            {
                a.push(json!(id));
            }
        }
        for p in remove {
            if let Some(a) = map.get_mut(p).and_then(Value::as_array_mut) {
                a.retain(|x| x != id);
            }
        }
        map.retain(|_, ids| ids.as_array().is_some_and(|a| !a.is_empty()));
        return true;
    }
    let Some(g) = list_mut(lib, kind).and_then(|l| l.iter_mut().find(|g| g["id"] == id)) else {
        return false;
    };
    let mut paths: Vec<String> = g["videoPaths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str().map(str::to_string))
        .collect();
    let drop: HashSet<&String> = remove.iter().collect();
    paths.retain(|p| !drop.contains(p));
    for p in add {
        if !paths.contains(p) {
            paths.push(p.clone());
        }
    }
    if let Some(order) = order {
        let named: Vec<String> = order
            .iter()
            .filter(|p| paths.contains(p))
            .cloned()
            .collect();
        let rest: Vec<String> = paths
            .iter()
            .filter(|p| !named.contains(p))
            .cloned()
            .collect();
        paths = named.into_iter().chain(rest).collect();
    }
    g["videoPaths"] = json!(paths);
    true
}

#[derive(Deserialize)]
struct Body {
    name: Option<String>,
    color: Option<String>,
}

fn bad(code: StatusCode, msg: &str) -> Response {
    (code, Json(json!({"error": msg}))).into_response()
}

async fn done(st: &AppState, body: Value) -> Response {
    match st.library_changed().await {
        Ok(()) => Json(body).into_response(),
        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn create(
    State(st): State<Arc<AppState>>,
    UrlPath(kind): UrlPath<String>,
    Json(b): Json<Body>,
) -> Response {
    let Some(name) = b.name.filter(|n| !n.trim().is_empty()) else {
        return bad(StatusCode::BAD_REQUEST, "a name is needed");
    };
    let g = create_in(
        &mut *st.library.write().await,
        &kind,
        &name,
        b.color.as_deref(),
    );
    match g {
        Some(g) => done(&st, g).await,
        None => bad(StatusCode::NOT_FOUND, "unknown kind"),
    }
}

async fn update(
    State(st): State<Arc<AppState>>,
    UrlPath((kind, id)): UrlPath<(String, String)>,
    Json(b): Json<Body>,
) -> Response {
    let found = {
        let mut lib = st.library.write().await;
        match list_mut(&mut lib, &kind).and_then(|l| l.iter_mut().find(|g| g["id"] == id.as_str()))
        {
            Some(g) => {
                if let Some(n) = b.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                    g["name"] = json!(n);
                }
                if let Some(c) = b.color.filter(|_| kind == "categories") {
                    g["color"] = json!(c);
                }
                Some(g.clone())
            }
            None => None,
        }
    };
    match found {
        Some(g) => done(&st, g).await,
        None => bad(StatusCode::NOT_FOUND, "no such list"),
    }
}

async fn remove(
    State(st): State<Arc<AppState>>,
    UrlPath((kind, id)): UrlPath<(String, String)>,
) -> Response {
    if delete_in(&mut *st.library.write().await, &kind, &id) {
        done(&st, json!({"ok": true})).await
    } else {
        bad(StatusCode::NOT_FOUND, "no such list")
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Members {
    add: Vec<String>,
    remove: Vec<String>,
    order: Option<Vec<String>>,
}

/// `{add, remove, order}` are video ids.
async fn videos(
    State(st): State<Arc<AppState>>,
    UrlPath((kind, id)): UrlPath<(String, String)>,
    Json(m): Json<Members>,
) -> Response {
    let snap = st.snapshot.read().await.clone();
    let path = |ids: &[String]| -> Vec<String> {
        ids.iter()
            .filter_map(|i| {
                snap.videos
                    .iter()
                    .find(|v| &v.id == i)
                    .map(|v| v.path.clone())
            })
            .collect()
    };
    let (add, rm) = (path(&m.add), path(&m.remove));
    let order = m.order.as_deref().map(path);
    if set_members(
        &mut *st.library.write().await,
        &kind,
        &id,
        &add,
        &rm,
        order.as_deref(),
    ) {
        done(&st, json!({"ok": true})).await
    } else {
        bad(StatusCode::NOT_FOUND, "no such list")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn playlists_add_remove_order() {
        let mut lib = Library::default();
        let id = create_in(&mut lib, "playlists", " Mine ", None).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(set_members(
            &mut lib,
            "playlists",
            &id,
            &s(&["/a", "/b", "/c", "/a"]),
            &[],
            None
        ));
        assert!(set_members(
            &mut lib,
            "playlists",
            &id,
            &[],
            &s(&["/b"]),
            Some(&s(&["/c"]))
        ));
        assert_eq!(
            lib.config["playlists"][0]["videoPaths"],
            json!(["/c", "/a"])
        );
        assert_eq!(lib.config["playlists"][0]["name"], "Mine");
        assert!(!set_members(&mut lib, "playlists", "nope", &[], &[], None));
    }

    #[test]
    fn categories_membership_and_delete() {
        let mut lib = Library::default();
        let c = create_in(&mut lib, "categories", "Fav", Some("#f00")).unwrap();
        let id = c["id"].as_str().unwrap().to_string();
        set_members(&mut lib, "categories", &id, &s(&["/a", "/b"]), &[], None);
        set_members(&mut lib, "categories", &id, &[], &s(&["/b"]), None);
        assert_eq!(lib.config["videoCategories"], json!({"/a": [id.clone()]}));
        assert!(delete_in(&mut lib, "categories", &id));
        assert_eq!(lib.config["videoCategories"], json!({}));
        assert!(create_in(&mut lib, "collections", "C", None).is_some());
        assert!(create_in(&mut lib, "bogus", "C", None).is_none());
    }
}
