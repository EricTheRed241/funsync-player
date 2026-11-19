//! End to end through the binary: serve a temp userData with a sample creators.json and
//! pictures on a free loopback port; fetch both tabs, the decks, a picture; swipe and undo,
//! over raw HTTP.

use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_creator-report");
/// The smallest bytes that pass for a JPEG here (the server doesn't look inside).
const JPEG: &[u8] = b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\xff\xd9";

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("creator-report-test-{name}-{}", std::process::id()));
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
fn start(user_data: &Path) -> (Server, u16) {
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
        .strip_prefix("Creators: http://127.0.0.1:")
        .and_then(|rest| rest.strip_suffix('/'))
        .unwrap_or_else(|| panic!("unexpected banner {line:?}"))
        .parse()
        .unwrap();
    (server, port)
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

fn get_json(port: u16, path: &str) -> Value {
    let (status, _, body) = request(port, "GET", path, None);
    assert_eq!(status, 200, "{path}: {}", String::from_utf8_lossy(&body));
    serde_json::from_slice(&body).unwrap()
}

fn post(port: u16, path: &str, body: Value) -> (u16, Value) {
    let (status, _, body) = request(port, "POST", path, Some(body.to_string().as_bytes()));
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

fn sample() -> Value {
    json!({
        "version": 1,
        "updatedAt": "2026-11-18T01:40:00Z",
        "eroscriptsChecked": {"newestTopicId": 348600},
        "creators": {
            "sweetiefox": {
                "name": "Sweetie Fox", "aliases": ["SweetieFox"], "firstSeen": "2026-11-17T21:00:00Z", "score": 6.1,
                "instagram": {"handle": "swfx_real", "followers": "5M", "posts": "225", "lastPostAt": null, "match": "high"},
                "twitch": {"handle": "sweetiefoxxx", "followers": 9225, "lastLive": "2020-09-11T18:17:44Z", "match": "medium"},
                "pictures": [
                    {"file": "es-41415-0.jpg", "source": "eroscripts", "url": "https://discuss.eroscripts.com/t/41415"},
                    {"file": "ig-3456789.jpg", "source": "instagram", "url": "https://www.instagram.com/p/abc/"}
                ],
                "posts": [
                    {"kind": "eroscripts", "id": "41415", "title": "Fox POV", "url": "https://discuss.eroscripts.com/t/41415",
                     "free": true, "hosts": ["pornhub"], "downloadable": true, "creampie": true, "multiAxis": false,
                     "score": 7.0, "likes": 108, "tags": ["pov"], "image": "es-41415-0.jpg", "durationS": 675, "inLibrary": false},
                    {"kind": "eroscripts", "id": "41416", "title": "Paid one", "free": false, "downloadable": false, "score": 8.0},
                    {"kind": "faptap", "id": "2122637633308856321", "title": "Tap", "url": "https://faptap.net/v/2122637633308856321",
                     "free": true, "hosts": ["faptap"], "downloadable": true, "score": 3.0, "durationS": 300, "inLibrary": false}
                ]
            },
            "annie": {"name": "Annie", "score": 4.0, "instagram": null, "twitch": null, "pictures": [], "posts": []}
        }
    })
}

#[test]
fn serves_the_tabs_decks_pictures_and_records_swipes() {
    let tmp = TempDir::new("serve");
    let ud = tmp.0.join("userData");
    let pics = ud.join("creator-pictures").join("sweetiefox");
    fs::create_dir_all(&pics).unwrap();
    fs::write(pics.join("es-41415-0.jpg"), JPEG).unwrap();
    fs::write(pics.join("ig-3456789.jpg"), JPEG).unwrap();
    fs::write(ud.join("creators.json"), sample().to_string()).unwrap();

    let (_server, port) = start(&ud);

    for page in ["/creators", "/posts", "/swiped", "/collection"] {
        let (status, head, body) = request(port, "GET", page, None);
        assert_eq!(status, 200, "{page}");
        assert!(head.contains("Content-Type: text/html; charset=utf-8"));
        assert!(head.contains("Cache-Control: no-store"));
        assert!(String::from_utf8_lossy(&body).contains("data-tab=\"collection\""));
    }
    let (status, head, _) = request(port, "GET", "/", None);
    assert_eq!(status, 303);
    assert!(head.contains("Location: /creators\r\n"), "{head}");

    let deck = get_json(port, "/api/creators/deck?seed=1&offset=0");
    let slugs: Vec<_> = deck["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs, ["sweetiefox", "annie"]);
    assert_eq!(deck["remaining"], 2);
    let fox = &deck["cards"][0];
    // Instagram's picture first.
    assert_eq!(
        fox["pictures"][0]["src"],
        "/pictures/sweetiefox/ig-3456789.jpg"
    );
    assert_eq!(fox["instagram"]["lastPostAt"], Value::Null);
    assert_eq!(fox["twitch"]["followers"], 9225);
    assert_eq!(
        fox["counts"],
        json!({"eroscriptsFree": 1, "eroscriptsPaid": 1, "faptap": 1})
    );
    assert_eq!(fox["topPosts"][0]["title"], "Paid one");

    let (status, head, body) = request(port, "GET", "/pictures/sweetiefox/ig-3456789.jpg", None);
    assert_eq!(status, 200);
    assert!(head.contains("Content-Type: image/jpeg"));
    assert!(head.contains("Cache-Control: max-age="));
    assert_eq!(body, JPEG);
    let (status, _, _) = request(
        port,
        "GET",
        "/pictures/sweetiefox/../../creators.json",
        None,
    );
    assert_eq!(status, 404);

    // Nobody said yes yet: no posts.
    assert_eq!(get_json(port, "/api/posts/deck")["remaining"], 0);
    let (status, v) = post(
        port,
        "/api/creators/swipe",
        json!({"slug": "sweetiefox", "choice": "yes"}),
    );
    assert_eq!(status, 200, "{v}");
    let (status, _) = post(
        port,
        "/api/creators/swipe",
        json!({"slug": "annie", "choice": "no"}),
    );
    assert_eq!(status, 200);
    assert_eq!(get_json(port, "/api/creators/deck")["remaining"], 0);

    let posts = get_json(port, "/api/posts/deck?offset=0");
    let keys: Vec<_> = posts["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["eroscripts:41415", "faptap:2122637633308856321"]);
    assert_eq!(
        posts["cards"][0]["pictures"][0]["src"],
        "/pictures/sweetiefox/es-41415-0.jpg"
    );
    assert_eq!(posts["cards"][0]["creatorName"], "Sweetie Fox");

    let (status, v) = post(
        port,
        "/api/posts/swipe",
        json!({"key": "eroscripts:41415", "choice": "want", "creator": "sweetiefox"}),
    );
    assert_eq!(status, 200, "{v}");
    let (status, _) = post(
        port,
        "/api/posts/swipe",
        json!({"key": "faptap:2122637633308856321", "choice": "hide", "creator": "sweetiefox"}),
    );
    assert_eq!(status, 200);
    let sum = get_json(port, "/api/summary");
    assert_eq!(
        sum["creators"],
        json!({"total": 2, "yes": 1, "no": 1, "unswiped": 0})
    );
    assert_eq!(sum["posts"]["wanted"], 1);
    assert_eq!(sum["posts"]["pendingDownload"], 1);
    assert_eq!(sum["posts"]["hidden"], 1);
    let collection = get_json(port, "/api/creators/collection");
    assert_eq!(collection["cards"][0]["slug"], "sweetiefox");
    assert_eq!(collection["cards"][0]["instagram"]["handle"], "swfx_real");
    let swiped = get_json(port, "/api/creators/swiped");
    let got: Vec<_> = swiped["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["slug"].as_str().unwrap(), c["choice"].as_str().unwrap()))
        .collect();
    // Newest swipe first: Annie was swiped after Sweetie Fox.
    assert_eq!(got, [("annie", "no"), ("sweetiefox", "yes")]);
    assert_eq!(
        swiped["cards"][1]["pictures"][0]["src"],
        "/pictures/sweetiefox/ig-3456789.jpg"
    );

    // The file creator-scout reads, in the contract's shape.
    let on_disk: Value =
        serde_json::from_slice(&fs::read(ud.join("creator-swipes.json")).unwrap()).unwrap();
    assert_eq!(on_disk["version"], 1);
    assert_eq!(on_disk["creators"]["sweetiefox"]["choice"], "yes");
    assert_eq!(on_disk["posts"]["eroscripts:41415"]["choice"], "want");
    assert_eq!(
        on_disk["posts"]["eroscripts:41415"]["creator"],
        "sweetiefox"
    );

    // Undo puts the post back in the deck.
    let (status, v) = post(
        port,
        "/api/posts/swipe/undo",
        json!({"key": "faptap:2122637633308856321"}),
    );
    assert_eq!((status, &v["removed"]), (200, &json!(true)));
    assert_eq!(get_json(port, "/api/posts/deck")["remaining"], 1);
    let (status, v) = post(port, "/api/creators/swipe/undo", json!({"slug": "annie"}));
    assert_eq!((status, &v["removed"]), (200, &json!(true)));
    assert_eq!(get_json(port, "/api/creators/deck")["remaining"], 1);

    // A new creators.json (another daily run) is read on the next request.
    let mut next = sample();
    next["creators"]["newbie"] = json!({"name": "Newbie", "score": 9.0});
    fs::write(ud.join("creators.json"), next.to_string()).unwrap();
    let deck = get_json(port, "/api/creators/deck");
    assert_eq!(deck["cards"][0]["slug"], "newbie");
}

#[test]
fn says_when_creators_json_is_missing() {
    let tmp = TempDir::new("missing");
    let (_server, port) = start(&tmp.0);
    let deck = get_json(port, "/api/creators/deck");
    assert_eq!(deck["missing"], true);
    assert_eq!(deck["cards"], json!([]));
    assert_eq!(get_json(port, "/api/posts/deck")["missing"], true);
    let (status, _, body) = request(port, "GET", "/creators", None);
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&body).contains("npm run creators:scout -- --daily"));
    assert!(!tmp.0.join("creator-swipes.json").exists());
}

#[test]
fn help_and_bad_arguments() {
    let out = Command::new(BIN).arg("--help").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("default 5132"));
    let out = Command::new(BIN).arg("--bogus").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("creator-report: unknown argument"));
    let out = Command::new(BIN)
        .args(["--user-data", "/nonexistent/creator-report-test"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("userData folder not found"));
}
