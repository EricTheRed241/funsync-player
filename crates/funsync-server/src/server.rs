//! HTTP: health, status, library listing, scan, media (stream, thumbnail,
//! script, subtitle) and the page.

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
use tower_http::services::ServeFile;

use crate::data::{self, Library, SCAN_FILE, now_ms};
use crate::media::Media;
use crate::scan::{self, Source, Video};

const INDEX_HTML: &str = include_str!("../web/index.html");

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub scanned_at: Option<u64>,
    pub scan_ms: u64,
    pub videos: Vec<Video>,
    pub unmatched_funscripts: usize,
    pub failed_paths: Vec<String>,
}

pub struct AppState {
    pub data_dir: PathBuf,
    pub library: RwLock<Library>,
    pub snapshot: RwLock<Arc<Snapshot>>,
    pub media: Media,
    pub scanning: AtomicBool,
    pub started_at: u64,
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
            .sources
            .iter()
            .filter(|s| s.enabled)
            .map(|s| Source {
                name: s.name.clone(),
                path: s.path.clone(),
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
        out
    }

    async fn video(&self, id: &str) -> Option<Video> {
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
    Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/api/status", get(status))
        .route("/api/videos", get(videos))
        .route("/api/videos/{id}", get(video))
        .route("/api/scan", post(scan_now))
        .route("/api/media/stream/{id}", get(stream))
        .route("/api/media/thumb/{id}", get(thumb))
        .route("/api/media/script/{id}", get(script))
        .route("/api/media/subtitle/{id}", get(subtitle))
        .with_state(state)
}

pub async fn serve(state: Arc<AppState>, addr: SocketAddr) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind {addr}: {e}"))?;
    eprintln!(
        "funsync-server: http://{addr}/ (data {})",
        state.data_dir.display()
    );
    let s = state.clone();
    tokio::spawn(async move {
        if let Err(e) = s.rescan().await {
            eprintln!("funsync-server: startup scan failed: {e}");
        }
    });
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|e| e.to_string())
}

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}

async fn index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        INDEX_HTML,
    )
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok", "app": "funsync-server" }))
}

async fn status(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let lib = st.library.read().await;
    let snap = st.snapshot.read().await.clone();
    let sources: Vec<_> = lib
        .sources
        .iter()
        .map(|s| {
            json!({
                "name": s.name, "path": s.path, "enabled": s.enabled,
                "present": Path::new(&s.path).is_dir(),
                "videos": snap.videos.iter().filter(|v| v.path.starts_with(&s.path)).count(),
            })
        })
        .collect();
    Json(json!({
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
    }))
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
    match st.media.thumbnail(&v.id, Path::new(&v.path)).await {
        Ok(p) => {
            let mut r = send_file(&p.to_string_lossy(), req, Some("image/jpeg")).await;
            r.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=86400"),
            );
            r
        }
        Err(e) => err(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
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
