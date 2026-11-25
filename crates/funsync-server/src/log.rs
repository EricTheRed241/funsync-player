//! The log file (`~/Library/Logs/funsync-rust/server.log`, `$FUNSYNC_RUST_LOG`):
//! every line also goes to stderr; at 1 GB the file moves to `server.old.log`
//! (as the old app's main.log). Requests are logged too, except quiet
//! loopback polling (a successful `/health` from this computer).

use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

use axum::extract::{ConnectInfo, Request};
use axum::middleware::Next;
use axum::response::Response;

const MAX_BYTES: u64 = 1 << 30;

static FILE: LazyLock<Mutex<Option<PathBuf>>> = LazyLock::new(|| {
    let path = std::env::var_os("FUNSYNC_RUST_LOG")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join("Library/Logs/funsync-rust/server.log"))
        });
    // Unit tests run the router in-process: they never write the user's log.
    Mutex::new(path.filter(|_| !cfg!(test)))
});

/// Write one line (timestamped) to stderr and the log file.
pub fn line(msg: impl AsRef<str>) {
    let text = format!("{} {}\n", video_tags::timefmt::now_iso(), msg.as_ref());
    eprint!("{text}");
    let guard = FILE.lock().unwrap();
    let Some(path) = guard.as_ref() else { return };
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(path, path.with_extension("old.log"));
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(text.as_bytes());
    }
}

pub async fn requests(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let (method, path) = (req.method().clone(), req.uri().path().to_string());
    let started = Instant::now();
    let resp = next.run(req).await;
    let quiet = addr.ip().is_loopback() && path == "/health" && resp.status().is_success();
    if !quiet {
        line(format!(
            "{addr} \"{method} {path}\" {} {} ms",
            resp.status().as_u16(),
            started.elapsed().as_millis()
        ));
    }
    resp
}
