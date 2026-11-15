//! End to end through the binary against a fake backend (a TcpListener
//! thread serving the video list and thumbnails with fixed statuses).

use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "library-thumbs-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

type Seen = Arc<Mutex<Vec<String>>>;

/// Serve `videos`; `/api/media/thumb/<id>` answers `statuses[id]` (200 when
/// absent) with a small body. Returns the URL and the paths requested.
fn fake_backend(videos: Value, statuses: Value) -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { continue };
            let mut reader = BufReader::new(conn.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            let path = first.split_whitespace().nth(1).unwrap().to_string();
            log.lock().unwrap().push(path.clone());
            let (status, body) = if path == "/api/remote/videos" {
                (200, json!({ "videos": videos }).to_string())
            } else if let Some(id) = path.strip_prefix("/api/media/thumb/") {
                let s = statuses.get(id).and_then(Value::as_u64).unwrap_or(200);
                (s, "JPEGDATA".repeat(100))
            } else {
                (404, String::new())
            };
            let _ = write!(
                conn,
                "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (url, seen)
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_library-thumbs"))
        .env_remove("FUNSYNC_USER_DATA")
        .args(args)
        .output()
        .unwrap()
}

fn videos() -> Value {
    json!([
        { "id": "a1", "path": "/V/a.mp4" },
        { "id": "b2", "path": "/V/b.mp4" },
        { "id": "c3", "path": "/V/c.mp4" },
        { "id": "d4", "path": "/V/d.mp4" },
    ])
}

#[test]
fn counts_each_outcome_and_exits_2_on_a_failure() {
    let (url, seen) = fake_backend(videos(), json!({ "b2": 503, "c3": 404 }));
    let out = run(&["--backend", &url]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stdout}{stderr}");
    assert!(
        stdout.contains("2 ok, 1 failed, 1 missing (of 4)"),
        "{stdout}"
    );
    assert!(stderr.contains("/V/b.mp4: HTTP 503"), "{stderr}");
    assert!(stderr.contains("/V/c.mp4: HTTP 404"), "{stderr}");
    assert_eq!(seen.lock().unwrap().len(), 5);
}

#[test]
fn missing_files_alone_exit_0() {
    let (url, _) = fake_backend(videos(), json!({ "d4": 404 }));
    let out = run(&["--backend", &url]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("3 ok, 0 failed, 1 missing (of 4)"));
}

#[test]
fn limit_caps_the_videos_asked_for() {
    let (url, seen) = fake_backend(videos(), json!({}));
    let out = run(&["--backend", &url, "--limit", "2"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("2 ok, 0 failed, 0 missing (of 2)"));
    assert_eq!(
        *seen.lock().unwrap(),
        [
            "/api/remote/videos",
            "/api/media/thumb/a1",
            "/api/media/thumb/b2"
        ]
    );
}

#[test]
fn default_backend_uses_the_configured_port() {
    let (url, seen) = fake_backend(videos(), json!({}));
    let port = url.rsplit(':').next().unwrap();
    let ud = TempDir::new();
    fs::write(
        ud.0.join("config.json"),
        json!({ "settings": {
            "backend": { "port": port.parse::<u16>().unwrap() },
            "library": { "sources": [{ "path": "/V" }] },
        } })
        .to_string(),
    )
    .unwrap();
    let out = run(&["--user-data", ud.0.to_str().unwrap(), "--limit=1"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[test]
fn an_unreachable_backend_exits_1() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}");
    let out = run(&["--backend", &url]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains(&format!(
        "The FunSync app isn't running (no backend at {url})"
    )));
}

#[test]
fn help_prints_usage() {
    let out = run(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("npm run library:thumbs"));
}
