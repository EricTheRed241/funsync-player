//! HTTP: health, status, library listing, scan, media (stream, thumbnail,
//! script, subtitle), the web UI and the parity dashboard.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::Body;
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::RwLock;
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};

use crate::data::{self, Library, SCAN_FILE, now_ms};
use crate::media::Media;
use crate::scan::{self, Source, Video};

/// The UI is the web remote (`backend/web-remote/`, served unchanged), with
/// this crate's `web/remote/` files taking precedence where a page needs
/// to differ.
const WEB_OVERLAY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/web/remote");
const WEB_REMOTE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../backend/web-remote");
const LOCALES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../renderer/locales");
const PARITY_HTML: &str = include_str!("../web/parity.html");
const PARITY_MD: &str = include_str!("../../../docs/rust-app/parity.md");
const NAV_JS: &str = include_str!("../../../backend/site-nav.js");

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub scanned_at: Option<u64>,
    pub scan_ms: u64,
    pub videos: Vec<Video>,
    pub unmatched_funscripts: usize,
    pub failed_paths: Vec<String>,
}

impl Snapshot {
    pub fn ids_by_path(&self) -> std::collections::HashMap<&str, &str> {
        self.videos
            .iter()
            .map(|v| (v.path.as_str(), v.id.as_str()))
            .collect()
    }
}

pub struct AppState {
    pub data_dir: PathBuf,
    pub library: RwLock<Library>,
    pub snapshot: RwLock<Arc<Snapshot>>,
    pub media: Media,
    pub scanning: AtomicBool,
    pub started_at: u64,
    pub devices: crate::devices::Devices,
}

impl AppState {
    pub fn load(data_dir: PathBuf) -> Result<Arc<Self>, String> {
        let library = data::load_library(&data_dir)?;
        // A missing or unreadable snapshot only means a scan is needed.
        let snapshot: Snapshot = std::fs::read(data_dir.join(SCAN_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Ok(Arc::new(Self {
            media: Media::new(data_dir.join("thumb-cache")),
            data_dir,
            library: RwLock::new(library),
            snapshot: RwLock::new(Arc::new(snapshot)),
            scanning: AtomicBool::new(false),
            started_at: now_ms(),
            devices: Default::default(),
        }))
    }

    /// Scan the enabled sources; false when a scan is already running.
    pub async fn rescan(self: &Arc<Self>) -> Result<bool, String> {
        if self.scanning.swap(true, Ordering::SeqCst) {
            return Ok(false);
        }
        let sources: Vec<Source> = self
            .library
            .read()
            .await
            .sources()
            .into_iter()
            .filter(|s| s.enabled)
            .map(|s| Source {
                name: s.name,
                path: s.path,
            })
            .collect();
        let started = std::time::Instant::now();
        let res = tokio::task::spawn_blocking(move || scan::scan_library(&sources)).await;
        let out = match res {
            Ok(r) => {
                let snap = Snapshot {
                    scanned_at: Some(now_ms()),
                    scan_ms: started.elapsed().as_millis() as u64,
                    unmatched_funscripts: r.unmatched_funscripts.len(),
                    failed_paths: r.failed_paths,
                    videos: r.videos,
                };
                let bytes = serde_json::to_vec(&snap).map_err(|e| e.to_string());
                *self.snapshot.write().await = Arc::new(snap);
                bytes
                    .and_then(|b| {
                        data::write_atomic(&self.data_dir.join(SCAN_FILE), &b)
                            .map_err(|e| e.to_string())
                    })
                    .map(|_| true)
            }
            Err(e) => Err(e.to_string()),
        };
        self.scanning.store(false, Ordering::SeqCst);
        let s = self.clone();
        tokio::spawn(async move { s.probe_durations().await });
        out
    }

    /// Write `library.json` (after an edit).
    pub async fn save(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec(&*self.library.read().await).map_err(|e| e.to_string())?;
        data::write_atomic(&self.data_dir.join(data::LIBRARY_FILE), &bytes)
            .map_err(|e| e.to_string())
    }

    /// Every library edit ends here: saves (score recomputation joins later).
    pub async fn library_changed(&self) -> Result<(), String> {
        self.save().await
    }

    /// ffprobe every video without a cached duration (`durationCache`, as
    /// in the old app), one at a time, saving every 25.
    async fn probe_durations(self: &Arc<Self>) {
        let snap = self.snapshot.read().await.clone();
        let mut done = 0;
        for v in &snap.videos {
            if self.library.read().await.lib()["durationCache"]
                .get(&v.path)
                .is_some()
            {
                continue;
            }
            let Ok(md) = self.media.metadata(Path::new(&v.path)).await else {
                continue;
            };
            if md.duration > 0.0 {
                self.library.write().await.lib_mut("durationCache")[&v.path] = json!(md.duration);
                done += 1;
                if done % 25 == 0 {
                    let _ = self.save().await;
                }
            }
        }
        if done > 0 {
            let _ = self.save().await;
        }
    }

    pub async fn video(&self, id: &str) -> Option<Video> {
        self.snapshot
            .read()
            .await
            .videos
            .iter()
            .find(|v| v.id == id)
            .cloned()
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    router_with_pin(state, data::default_old_user_data().unwrap_or_default())
}

/// The app with the access PIN read from `pin_dir` (the old app's folder).
pub fn router_with_pin(state: Arc<AppState>, pin_dir: PathBuf) -> Router {
    let remote_files = Router::new()
        .fallback_service(ServeDir::new(WEB_OVERLAY).fallback(ServeDir::new(WEB_REMOTE)))
        .layer(axum::middleware::map_response(with_extras));
    Router::new()
        .route(
            "/",
            get(|| async { axum::response::Redirect::to("/remote/") }),
        )
        .route(
            "/remote",
            get(|| async { axum::response::Redirect::to("/remote/") }),
        )
        .route("/site-nav.js", get(nav))
        .route("/parity", get(parity_page))
        .route("/api/parity", get(parity))
        .route("/health", get(health))
        .route("/api/status", get(status))
        .route("/api/videos", get(videos))
        .route("/api/videos/{id}", get(video))
        .route("/api/scan", post(scan_now))
        .route("/api/media/stream/{id}", get(stream))
        .route("/api/media/thumb/{id}", get(thumb))
        .route("/api/media/script/{id}", get(script))
        .route("/api/media/subtitle/{id}", get(subtitle))
        .merge(crate::remote::routes())
        .merge(crate::sites::routes())
        .merge(crate::settings::routes())
        .merge(crate::heresphere::routes())
        .merge(crate::devices::routes())
        .merge(crate::edits::routes())
        .merge(crate::queue::routes())
        .nest_service("/remote/", remote_files)
        .nest_service("/locales", ServeDir::new(LOCALES))
        .with_state(state)
        .layer(axum::middleware::from_fn_with_state(
            crate::access::Access::new(pin_dir),
            crate::access::layer,
        ))
}

/// The web remote's pages get `extras.js` (web/remote/extras.js: the desktop
/// player's keyboard shortcuts, screenshot, PiP, …) before `</body>`.
async fn with_extras(resp: Response) -> Response {
    let html = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|c| c.to_str().ok())
        .is_some_and(|c| c.starts_with("text/html"));
    if !html {
        return resp;
    }
    let (mut parts, body) = resp.into_parts();
    let page = axum::body::to_bytes(body, 8 << 20)
        .await
        .unwrap_or_default();
    let page = String::from_utf8_lossy(&page).replacen(
        "</body>",
        "<script src=\"/remote/extras.js\" defer></script></body>",
        1,
    );
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(page))
}

pub async fn serve(state: Arc<AppState>, addr: SocketAddr, supervise: bool) -> Result<(), String> {
    if supervise {
        crate::sites::supervise(&data::default_old_user_data().unwrap_or_default());
    }
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind {addr}: {e}"))?;
    eprintln!(
        "funsync-server: http://{addr}/ (data {})",
        state.data_dir.display()
    );
    crate::settings::watch_drives(state.clone());
    crate::devices::start(&state).await;
    let s = state.clone();
    tokio::spawn(async move {
        if let Err(e) = s.rescan().await {
            eprintln!("funsync-server: startup scan failed: {e}");
        }
    });
    let app = router(state).into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|e| e.to_string())
}

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}

async fn nav() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        format!(
            "window.FUNSYNC_NAV={};\n{NAV_JS}",
            crate::sites::nav_links()
        ),
    )
}

async fn parity_page() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        PARITY_HTML,
    )
}

/// `docs/rust-app/parity.md` as `{counts, sections: [{name, rows}]}`.
pub fn parse_parity(md: &str) -> serde_json::Value {
    let mut sections: Vec<serde_json::Value> = Vec::new();
    let mut counts = serde_json::Map::new();
    for line in md.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            sections.push(json!({"name": h, "rows": []}));
            continue;
        }
        let cells: Vec<&str> = line
            .trim()
            .trim_matches('|')
            .split(" | ")
            .map(str::trim)
            .collect();
        let [feature, status, notes] = cells[..] else {
            continue;
        };
        if !["done", "partial", "not started", "not planned"].contains(&status) {
            continue;
        }
        if let Some(s) = sections.last_mut() {
            s["rows"]
                .as_array_mut()
                .unwrap()
                .push(json!({"feature": feature, "status": status, "notes": notes}));
            let n = counts.get(status).and_then(|c| c.as_u64()).unwrap_or(0);
            counts.insert(status.to_string(), json!(n + 1));
        }
    }
    sections.retain(|s| !s["rows"].as_array().unwrap().is_empty());
    json!({"counts": counts, "sections": sections})
}

async fn parity() -> impl IntoResponse {
    Json(parse_parity(PARITY_MD))
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok", "app": "funsync-server" }))
}

/// The server's own status, plus the old backend's device part (`issues`,
/// `devices`, `session`, `phone`, `backend`, `desktop`).
async fn status(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let mut out = crate::devices::status(&st).await;
    let lib = st.library.read().await;
    let snap = st.snapshot.read().await.clone();
    let sources: Vec<_> = lib
        .sources()
        .into_iter()
        .map(|s| {
            json!({
                "name": s.name, "path": s.path, "enabled": s.enabled,
                "present": Path::new(&s.path).is_dir(),
                "videos": snap.videos.iter().filter(|v| v.path.starts_with(&s.path)).count(),
            })
        })
        .collect();
    let own = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "dataDir": st.data_dir,
        "startedAt": st.started_at,
        "importedFrom": lib.imported_from,
        "importedAt": lib.imported_at,
        "sources": sources,
        "scanning": st.scanning.load(Ordering::SeqCst),
        "scannedAt": snap.scanned_at,
        "scanMs": snap.scan_ms,
        "videos": snap.videos.len(),
        "withScript": snap.videos.iter().filter(|v| v.funscript_path.is_some()).count(),
        "unmatchedFunscripts": snap.unmatched_funscripts,
        "failedPaths": snap.failed_paths,
    });
    for (k, v) in own.as_object().into_iter().flatten() {
        out[k] = v.clone();
    }
    Json(out)
}

async fn videos(State(st): State<Arc<AppState>>) -> Response {
    let snap = st.snapshot.read().await.clone();
    Json(&snap.videos).into_response()
}

async fn video(State(st): State<Arc<AppState>>, UrlPath(id): UrlPath<String>) -> Response {
    match st.video(&id).await {
        Some(v) => Json(v).into_response(),
        None => err(StatusCode::NOT_FOUND, "no such video"),
    }
}

async fn scan_now(State(st): State<Arc<AppState>>) -> Response {
    match st.rescan().await {
        Ok(true) => status(State(st)).await.into_response(),
        Ok(false) => err(StatusCode::CONFLICT, "a scan is already running"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn send_file(path: &str, req: Request, content_type: Option<&'static str>) -> Response {
    if !Path::new(path).is_file() {
        return err(StatusCode::NOT_FOUND, "file missing");
    }
    match ServeFile::new(path).oneshot(req).await {
        Ok(r) => {
            let mut r = r.map(Body::new);
            if let Some(ct) = content_type {
                r.headers_mut()
                    .insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
            }
            r
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn stream(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    req: Request,
) -> Response {
    let Some(v) = st.video(&id).await else {
        return err(StatusCode::NOT_FOUND, "no such video");
    };
    send_file(&v.path, req, None).await
}

async fn thumb(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    req: Request,
) -> Response {
    let Some(v) = st.video(&id).await else {
        return err(StatusCode::NOT_FOUND, "no such video");
    };
    // `?v=` names one version for good (crate::remote::thumb_url).
    let versioned = req
        .uri()
        .query()
        .is_some_and(|q| q.split('&').any(|p| p.starts_with("v=")));
    let poster = crate::remote::poster(&*st.library.read().await, &v.path);
    let (file, ct, cache) = if let Some(p) = poster {
        let ct = match p
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("png") => "image/png",
            Some("webp") => "image/webp",
            _ => "image/jpeg",
        };
        (p, ct, "public, max-age=3600")
    } else {
        // The VR crop needs the frame size; the thumbnail probes it anyway.
        let dims = st
            .media
            .metadata(Path::new(&v.path))
            .await
            .ok()
            .map(|m| (m.width, m.height));
        let crop = crate::vr::thumb_crop(crate::vr::detect(&*st.library.read().await, &v, dims));
        match st.media.thumbnail(&v.id, Path::new(&v.path), crop).await {
            Ok(p) => (p, "image/jpeg", "public, max-age=86400"),
            Err(e) => return err(StatusCode::UNPROCESSABLE_ENTITY, e),
        }
    };
    let cache = if versioned {
        "public, max-age=31536000, immutable"
    } else {
        cache
    };
    let mut r = send_file(&file.to_string_lossy(), req, Some(ct)).await;
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    r
}

#[derive(Deserialize)]
struct ScriptQuery {
    variant: Option<String>,
    axis: Option<String>,
}

/// The video's main script, a variant by label, or an axis companion.
async fn script(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<ScriptQuery>,
    req: Request,
) -> Response {
    let Some(v) = st.video(&id).await else {
        return err(StatusCode::NOT_FOUND, "no such video");
    };
    let path = if let Some(axis) = &q.axis {
        // Hand-made beats generated.
        v.axes
            .iter()
            .filter(|a| &a.axis == axis)
            .min_by_key(|a| a.generated)
            .map(|a| a.path.clone())
    } else if let Some(label) = &q.variant {
        v.variants
            .iter()
            .find(|x| &x.label == label)
            .map(|x| x.path.clone())
    } else {
        v.funscript_path.clone()
    };
    match path {
        Some(p) => send_file(&p, req, Some("application/json")).await,
        None => err(StatusCode::NOT_FOUND, "no such script"),
    }
}

/// SRT → WebVTT: a header, and `,` → `.` in the cue timings.
pub fn srt_to_vtt(srt: &str) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for line in srt
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .lines()
    {
        if line.contains("-->") {
            out.push_str(&line.replace(',', "."));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

async fn subtitle(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    req: Request,
) -> Response {
    let Some(p) = st.video(&id).await.and_then(|v| v.subtitle_path) else {
        return err(StatusCode::NOT_FOUND, "no subtitle");
    };
    if p.to_lowercase().ends_with(".srt") {
        return match tokio::fs::read(&p).await {
            Ok(b) => (
                [(header::CONTENT_TYPE, "text/vtt; charset=utf-8")],
                srt_to_vtt(&String::from_utf8_lossy(&b)),
            )
                .into_response(),
            Err(e) => err(StatusCode::NOT_FOUND, e.to_string()),
        };
    }
    send_file(&p, req, Some("text/vtt; charset=utf-8")).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_conversion() {
        let v = srt_to_vtt("\u{feff}1\r\n00:00:01,500 --> 00:00:02,000\r\nHi, there\r\n");
        assert_eq!(v, "WEBVTT\n\n1\n00:00:01.500 --> 00:00:02.000\nHi, there\n");
    }
}
