//! End to end through the binary: serve a temp userData on a free loopback
//! port, fetch the pages, the report and the deck, and swipe, over raw HTTP.

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
    request(port, "GET", path, None)
}

/// `POST path` with a JSON body → (status, body as JSON or Null).
fn post(port: u16, path: &str, body: &str) -> (u16, Value) {
    let (status, _, body) = request(port, "POST", path, Some(body.as_bytes()));
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

fn request(port: u16, method: &str, path: &str, body: Option<&[u8]>) -> (u16, String, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n").unwrap();
    if let Some(body) = body {
        write!(
            s,
            "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .unwrap();
        s.write_all(body).unwrap();
    } else {
        write!(s, "\r\n").unwrap();
    }
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
fn swipes_through_the_deck() {
    let tmp = TempDir::new("swipe");
    let ud = tmp.0.join("userData");
    fs::create_dir_all(&ud).unwrap();
    let state = json!({"version": 1, "topics": {
        "31": {"verdict": "candidate", "title": "Thirty-one", "image": "https://x/31.jpg",
               "images": ["https://x/31.jpg", "https://x/31b.jpg"]},
        "32": {"verdict": "candidate", "title": "Thirty-two"}
    }});
    let waiting = json!({"candidates": [
        {"id": 31, "title": "L31", "score": 9.5, "vr": true, "axisType": "multi", "creampie": true,
         "tagScores": {"pov": 3, "anal": -2}, "scenes": [{"axes": ["twist"]}]},
        {"id": 32, "title": "L32", "score": 1, "axisType": "single", "scenes": [{"axes": []}, {"axes": []}]},
        {"id": 33, "title": "L33", "score": 4}
    ]});
    fs::write(ud.join("eroscripts-scout-state.json"), state.to_string()).unwrap();
    fs::write(ud.join("eroscripts-scout.json"), waiting.to_string()).unwrap();
    let (_server, port) = start(&ud);

    let (status, head, body) = get(port, "/swipe");
    assert_eq!(status, 200);
    assert!(head.contains("Content-Type: text/html"));
    let page = String::from_utf8(body).unwrap();
    assert!(page.contains(r#"<meta name="referrer" content="no-referrer">"#));
    assert!(!page.contains("<script src"), "no external scripts");
    assert!(page.contains(r#"href="/""#), "links back to the report");
    let report_page = String::from_utf8(get(port, "/").2).unwrap();
    assert!(report_page.contains(r#"href="/swipe""#));
    // Both pages carry their tab icon inline.
    for p in [&page, &report_page] {
        assert!(p.contains(r#"<link rel="icon" href="data:image/svg+xml,"#));
    }

    // The default stack is multi-axis: only 31.
    let deck: Value = serde_json::from_slice(&get(port, "/api/deck?limit=2").2).unwrap();
    assert_eq!(
        (deck["remaining"].as_u64(), deck["wanted"].as_u64()),
        (Some(1), Some(0))
    );
    let ids = |d: &Value| -> Vec<u64> {
        d["cards"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_u64().unwrap())
            .collect()
    };
    assert_eq!(ids(&deck), [31]);
    let single: Value = serde_json::from_slice(&get(port, "/api/deck?stack=single").2).unwrap();
    assert_eq!(
        (ids(&single), single["remaining"].as_u64()),
        (vec![33, 32], Some(2))
    );
    assert_eq!(
        deck["cards"][0],
        json!({"id": 31, "url": "https://discuss.eroscripts.com/t/31", "title": "Thirty-one", "author": null,
               "images": ["https://x/31.jpg", "https://x/31b.jpg"], "score": 9.5, "vr": true,
               "axisType": "multi", "axes": 2, "videos": 1, "creampie": true,
               "topTags": [{"tag": "pov", "score": 3.0}],
               "tags": [{"tag": "pov", "score": 3.0}, {"tag": "anal", "score": -2.0}],
               "runtimeMs": null, "heat": [], "likes": null, "hosts": [], "gb": null,
               "created": null, "bumped": null,
               "scenes": [{"name": null, "host": null, "size": null, "runtimeMs": null, "axes": ["twist"]}],
               "moreScenes": 0})
    );
    // A shuffle wider than MAX_JITTER is refused.
    assert_eq!(get(port, "/api/deck?seed=1&jitter=31").0, 400);

    let (status, v) = post(port, "/api/swipe", r#"{"id":31,"dir":"right"}"#);
    assert_eq!(status, 200);
    assert_eq!(
        (v["id"].as_u64(), v["dir"].as_str()),
        (Some(31), Some("right"))
    );
    assert_eq!(post(port, "/api/swipe", r#"{"id":32,"dir":"left"}"#).0, 200);

    let deck: Value = serde_json::from_slice(&get(port, "/api/deck").2).unwrap();
    assert_eq!(
        (
            ids(&deck),
            deck["remaining"].as_u64(),
            deck["wanted"].as_u64()
        ),
        (vec![], Some(0), Some(1))
    );
    let deck: Value = serde_json::from_slice(&get(port, "/api/deck?stack=single").2).unwrap();
    assert_eq!(ids(&deck), [33]);
    assert_eq!(
        (deck["remaining"].as_u64(), deck["wanted"].as_u64()),
        (Some(1), Some(1))
    );
    let file: Value =
        serde_json::from_slice(&fs::read(ud.join("eroscripts-scout-swipes.json")).unwrap())
            .unwrap();
    assert_eq!(file["version"], 1);
    assert_eq!(file["swipes"]["31"]["dir"], "right");
    assert_eq!(file["swipes"]["32"]["dir"], "left");
    assert!(file["swipes"]["31"]["at"].as_str().unwrap().ends_with('Z'));
    let report: Value = serde_json::from_slice(&get(port, "/api/report").2).unwrap();
    let swipe_of = |id: u64| {
        report["posts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .unwrap()["swipe"]
            .clone()
    };
    assert_eq!(
        (swipe_of(31), swipe_of(32), swipe_of(33)),
        (json!("right"), json!("left"), Value::Null)
    );

    // Undo brings the card back and drops the entry.
    let (status, v) = post(port, "/api/swipe/undo", r#"{"id":32}"#);
    assert_eq!((status, v["removed"].as_bool()), (200, Some(true)));
    let deck: Value = serde_json::from_slice(&get(port, "/api/deck?stack=single").2).unwrap();
    assert_eq!(ids(&deck), [33, 32]);
    let file: Value =
        serde_json::from_slice(&fs::read(ud.join("eroscripts-scout-swipes.json")).unwrap())
            .unwrap();
    assert!(file["swipes"].get("32").is_none());

    // Bad requests change nothing.
    let before = fs::read(ud.join("eroscripts-scout-swipes.json")).unwrap();
    for (path, body) in [
        ("/api/swipe", "{"),
        ("/api/swipe", r#"{"id":33,"dir":"up"}"#),
        ("/api/swipe", r#"{"id":999,"dir":"right"}"#),
        ("/api/swipe/undo", r#"{"dir":"right"}"#),
    ] {
        assert_eq!(post(port, path, body).0, 400, "{path} {body}");
    }
    let big = format!(r#"{{"id":33,"dir":"right","pad":"{}"}}"#, "x".repeat(5000));
    assert_eq!(post(port, "/api/swipe", &big).0, 413);
    assert_eq!(get(port, "/api/deck?limit=x").0, 400);
    assert_eq!(
        fs::read(ud.join("eroscripts-scout-swipes.json")).unwrap(),
        before
    );
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
