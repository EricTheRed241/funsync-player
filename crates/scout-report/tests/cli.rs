//! End to end through the binary: serve a temp userData on a free loopback
//! port, fetch the page and the report over raw HTTP.

use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_scout-report");

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("scout-report-test-{name}-{}", std::process::id()));
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

/// The server process, killed when dropped.
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Start the binary on a free port; returns it with the port it announced.
fn start(user_data: &PathBuf) -> (Server, u16) {
    let mut child = Command::new(BIN)
        .args(["--port", "0", "--user-data"])
        .arg(user_data)
        .env_remove("FUNSYNC_USER_DATA")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let server = Server(child);
    let port = line
        .trim()
        .strip_prefix("Scout report: http://127.0.0.1:")
        .and_then(|rest| rest.strip_suffix('/'))
        .unwrap_or_else(|| panic!("unexpected banner {line:?}"))
        .parse()
        .unwrap();
    (server, port)
}

/// `GET path` → (status, headers, body).
fn get(port: u16, path: &str) -> (u16, String, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8(raw[..split].to_vec()).unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    (status, head, raw[split + 4..].to_vec())
}

#[test]
fn serves_the_page_and_the_report() {
    let tmp = TempDir::new("serve");
    let ud = tmp.0.join("userData");
    fs::create_dir_all(&ud).unwrap();
    let state = json!({
        "version": 1,
        "lastScanAt": "2026-11-12T10:00:00.000Z",
        "feeds": {"vam": {"backfillPage": 2, "exhausted": true, "reached": "2025-04-09T20:34:05.030Z"}},
        "topics": {
            "11": {"verdict": "no-video", "rules": 12, "at": "2026-11-10T18:05:15.337Z"},
            "22": {"verdict": "candidate", "rules": 12, "at": "2026-11-11T00:00:00Z",
                   "title": "A post", "author": "ann", "created": "2026-10-01T00:00:00Z",
                   "bumped": "2026-10-05T00:00:00Z", "image": null}
        },
        "pending": {}
    });
    let waiting = json!({
        "generatedAt": "2026-11-12T10:00:01.000Z", "listed": 1, "judged": 2, "rejected": {},
        "candidates": [{"id": 22, "title": "Listed", "url": "https://discuss.eroscripts.com/t/22",
            "author": "ann", "tags": ["pov"], "likes": 3, "vr": false, "music": false,
            "axisType": "single", "hosts": ["MEGA"], "live": false, "favourite": null,
            "artistInLibrary": null, "score": 2.5,
            "scenes": [{"base": "S", "axes": [], "video": {"name": "S.mp4", "host": "MEGA", "size": 250000000}}]}]
    });
    let rejected = json!({"version": 1, "entries": [
        {"key": "11|v.mp4", "topicId": 11, "video": "v.mp4", "title": "Turned down",
         "url": "https://discuss.eroscripts.com/t/11", "reason": "short", "at": "2026-11-01T00:00:00Z"}
    ]});
    fs::write(ud.join("eroscripts-scout-state.json"), state.to_string()).unwrap();
    fs::write(ud.join("eroscripts-scout.json"), waiting.to_string()).unwrap();
    fs::write(
        ud.join("eroscripts-scout-rejected.json"),
        rejected.to_string(),
    )
    .unwrap();
    let before: Vec<_> = fs::read_dir(&ud)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();

    let (_server, port) = start(&ud);

    let (status, head, body) = get(port, "/");
    assert_eq!(status, 200);
    assert!(head.contains("Content-Type: text/html"));
    let page = String::from_utf8(body).unwrap();
    assert!(page.contains("/api/report"));
    assert!(!page.contains("<script src"), "no external scripts");

    let (status, head, body) = get(port, "/api/report");
    assert_eq!(status, 200);
    assert!(head.contains("Content-Type: application/json"));
    let r: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(r["lastScanAt"], "2026-11-12T10:00:00.000Z");
    assert_eq!(r["feeds"]["vam"]["backfillPage"], 2);
    assert_eq!(r["verdicts"], json!({"candidate": 1, "no-video": 1}));
    assert_eq!(r["waiting"], 1);
    let posts = r["posts"].as_array().unwrap();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0]["id"], 22);
    assert_eq!(posts[0]["title"], "A post");
    assert_eq!(posts[0]["waiting"], true);
    assert_eq!(posts[0]["score"], 2.5);
    assert_eq!(posts[0]["gb"], 0.25);
    assert_eq!(posts[1]["id"], 11);
    assert_eq!(posts[1]["url"], "https://discuss.eroscripts.com/t/11");
    assert_eq!(posts[1]["title"], "Turned down");
    assert_eq!(posts[1]["waiting"], false);
    assert_eq!(posts[1]["rejectedScenes"][0]["reason"], "short");

    // Rebuilt per request: a file that disappears empties its section.
    fs::remove_file(ud.join("eroscripts-scout.json")).unwrap();
    let r: Value = serde_json::from_slice(&get(port, "/api/report").2).unwrap();
    assert_eq!(r["waiting"], 0);
    assert_eq!(r["posts"][0]["waiting"], false);

    assert_eq!(get(port, "/eroscripts-scout-state.json").0, 404);
    assert_eq!(get(port, "/api/nope").0, 404);

    // Read-only: nothing was written next to the inputs.
    let mut after: Vec<_> = fs::read_dir(&ud)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    after.push(ud.join("eroscripts-scout.json"));
    let (mut a, mut b) = (before, after);
    a.sort();
    b.sort();
    assert_eq!(a, b);
}

#[test]
fn empty_user_data_serves_an_empty_report() {
    let tmp = TempDir::new("empty");
    let (_server, port) = start(&tmp.0);
    let (status, _, body) = get(port, "/api/report");
    assert_eq!(status, 200);
    let r: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(r["posts"], json!([]));
    assert_eq!(r["feeds"], json!({}));
    assert_eq!(r["lastScanAt"], Value::Null);
}

#[test]
fn help_and_errors() {
    let out = Command::new(BIN).arg("--help").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("--port"));

    let out = Command::new(BIN).arg("--bogus").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown argument"));

    let out = Command::new(BIN)
        .args([
            "--port",
            "0",
            "--user-data",
            "/nonexistent/scout-report-test",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not found"));
}
