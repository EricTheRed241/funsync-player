//! End to end through the binary: serve a temp userData on a free loopback
//! port, fetch the deck page, the deck and the drive, and swipe, over raw HTTP.

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
fn the_root_goes_to_the_deck_and_the_report_is_gone() {
    let tmp = TempDir::new("serve");
    let (_server, port) = start(&tmp.0);
    let (status, head, _) = get(port, "/");
    assert_eq!(status, 303);
    assert!(head.contains("\r\nLocation: /swipe\r\n"), "{head}");
    assert_eq!(get(port, "/api/report").0, 404);
    assert_eq!(get(port, "/eroscripts-scout-state.json").0, 404);
    assert_eq!(get(port, "/api/nope").0, 404);
}

#[test]
fn the_drive_bar_measures_the_library_sources() {
    let tmp = TempDir::new("drive");
    let ud = tmp.0.join("userData");
    let lib = tmp.0.join("Library");
    fs::create_dir_all(&ud).unwrap();
    fs::create_dir_all(&lib).unwrap();
    let config = json!({"settings": {"library": {"sources": [{"path": lib.to_str().unwrap()}]}}});
    fs::write(ud.join("config.json"), config.to_string()).unwrap();
    let (_server, port) = start(&ud);
    let (status, head, body) = get(port, "/api/drive");
    assert_eq!(status, 200);
    assert!(head.contains("Content-Type: application/json"));
    let v: Value = serde_json::from_slice(&body).unwrap();
    let (free, total) = (
        v["drive"]["free"].as_u64().unwrap(),
        v["drive"]["total"].as_u64().unwrap(),
    );
    assert!(total > 0 && free <= total, "{v}");
    assert!(v["minFreeBytes"].as_u64().unwrap() > 0);
    assert_eq!(v["full"], free < v["minFreeBytes"].as_u64().unwrap());
}

#[test]
fn with_a_pin_this_mac_still_gets_in_and_logins_set_the_cookie() {
    let tmp = TempDir::new("pin");
    fs::write(
        tmp.0.join("access-pin.json"),
        format!(
            r#"{{"version":1,"pin":"2468","secret":"{}","changedAt":"2026-11-16T10:00:00.000Z"}}"#,
            "ab".repeat(32)
        ),
    )
    .unwrap();
    let (_server, port) = start(&tmp.0);
    assert_eq!(get(port, "/api/deck").0, 200);
    assert_eq!(get(port, "/swipe").0, 200);
    let (status, head, _) = request(port, "POST", "/login", Some(b"pin=2468&next=%2Fswipe"));
    assert_eq!(status, 303);
    assert!(head.starts_with("HTTP/1.1 303 See Other\r\n"), "{head}");
    assert!(head.contains("\r\nLocation: /swipe\r\n"), "{head}");
    let cookie = head
        .lines()
        .find_map(|l| l.strip_prefix("Set-Cookie: funsync_access=v1."))
        .unwrap_or_else(|| panic!("no cookie in {head}"));
    assert!(cookie.ends_with("; Path=/; Max-Age=2592000; HttpOnly; SameSite=Lax"));
    let (status, head, body) = request(port, "POST", "/login", Some(b"pin=1111&next=%2F"));
    assert_eq!(status, 401);
    assert!(head.starts_with("HTTP/1.1 401 Unauthorized\r\n"), "{head}");
    assert!(String::from_utf8_lossy(&body).contains("Wrong PIN"));
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
    assert!(
        !page.contains(r#"href="/""#),
        "no link to the removed report"
    );
    assert!(page.contains("/api/drive"));
    assert!(page.contains(r#"<link rel="icon" href="data:image/svg+xml,"#));

    // The default tab is Multi: only 31 (multi-axis, VR).
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
    let flat: Value = serde_json::from_slice(&get(port, "/api/deck?stack=flat").2).unwrap();
    assert_eq!(
        (ids(&flat), flat["remaining"].as_u64()),
        (vec![33, 32], Some(2))
    );
    let vr: Value = serde_json::from_slice(&get(port, "/api/deck?stack=vr").2).unwrap();
    assert_eq!(vr["remaining"], 0);
    assert_eq!(
        deck["cards"][0],
        json!({"source": "eroscripts", "id": 31, "url": "https://discuss.eroscripts.com/t/31", "title": "Thirty-one", "author": null,
               "images": ["https://x/31.jpg", "https://x/31b.jpg"], "score": 9.5, "vr": true,
               "axisType": "multi", "axes": 2, "videos": 1, "creampie": true,
               "topTags": [{"tag": "pov", "score": 3.0}],
               "tags": [{"tag": "pov", "score": 3.0}, {"tag": "anal", "score": -2.0}],
               "runtimeMs": null, "heat": [], "likes": null, "hosts": [], "gb": null,
               "created": null, "bumped": null,
               "scenes": [{"name": null, "host": null, "url": null, "size": null, "runtimeMs": null, "axes": ["twist"]}],
               "moreScenes": 0})
    );
    // A shuffle wider than MAX_JITTER is refused, and so is the old single-axis tab.
    assert_eq!(get(port, "/api/deck?seed=1&jitter=31").0, 400);
    assert_eq!(get(port, "/api/deck?stack=single").0, 400);

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
    // Each tab counts its own right swipes: 31's is Multi's.
    let deck: Value = serde_json::from_slice(&get(port, "/api/deck?stack=flat").2).unwrap();
    assert_eq!(ids(&deck), [33]);
    assert_eq!(
        (deck["remaining"].as_u64(), deck["wanted"].as_u64()),
        (Some(1), Some(0))
    );
    let file: Value =
        serde_json::from_slice(&fs::read(ud.join("eroscripts-scout-swipes.json")).unwrap())
            .unwrap();
    assert_eq!(file["version"], 1);
    assert_eq!(file["swipes"]["31"]["dir"], "right");
    assert_eq!(file["swipes"]["32"]["dir"], "left");
    assert!(file["swipes"]["31"]["at"].as_str().unwrap().ends_with('Z'));

    // Undo brings the card back and drops the entry.
    let (status, v) = post(port, "/api/swipe/undo", r#"{"id":32}"#);
    assert_eq!((status, v["removed"].as_bool()), (200, Some(true)));
    let deck: Value = serde_json::from_slice(&get(port, "/api/deck?stack=flat").2).unwrap();
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
fn every_source_deals_into_the_tabs_and_swipes_into_its_own_file() {
    let tmp = TempDir::new("sources");
    let ud = tmp.0.join("userData");
    fs::create_dir_all(&ud).unwrap();
    let waiting = json!({"candidates": [
        {"id": 31, "title": "MEGA post", "score": 9.5, "axisType": "multi", "scenes": [{"video": {"host": "MEGA"}}]},
        {"id": 34, "title": "SLR post", "score": 4, "axisType": "single", "vr": true, "tagScores": {"pov": 2},
         "scenes": [{"axes": [], "video": {"name": "A - B (C)_180_LR.mp4", "host": "slr", "slrSceneId": 66249,
             "slrLabel": "b-66249", "url": "https://www.sexlikereal.com/scenes/b-66249", "size": null, "height": 2880}}]},
        {"id": 35, "title": "VRBangers post", "score": 3, "axisType": "single", "vr": true,
         "scenes": [{"axes": [], "video": {"host": "vrbangers", "slug": "same-scene",
             "url": "https://vrbangers.com/video/same-scene/"}}]}
    ]});
    let slr = json!({"generatedAt": "2026-11-13T18:00:00.000Z",
    "quota": {"videos": {"remaining": 30, "total": 35}},
    "scenes": [
        {"id": 66249, "title": "Same video as post 34", "score": 8, "vr": true},
        {"id": 51207, "title": "Scene", "score": 6, "vr": true, "tags": ["milf"], "tagScores": {"milf": 2},
         "scriptSource": "slr-handcrafted", "scoreParts": {"vr": 2, "tag:milf": 2}},
        {"id": 70001, "title": "Downloaded", "score": 9, "vr": true, "fetchedAt": "2026-11-13T19:00:00.000Z"}
    ]});
    let vrb = json!({"version": 1, "generatedAt": "2026-11-17T18:00:00.000Z", "scenes": [
        {"id": "6accfc2cff316f0c39086f52", "site": "vrbangers", "url": "https://vrbangers.com/video/same-scene/",
         "title": "Same as post 35", "score": 50, "vr": true, "script": {"axes": []}},
        {"id": "7b", "site": "vrconk", "url": "https://vrconk.com/video/hand-made/", "title": "Hand-made axes",
         "studio": "VRConk", "score": 7, "vr": true, "script": {"url": "https://mov/x.funscript", "axes": ["twist", "roll"]},
         "thumbnail": "https://content/7b.jpg", "images": ["https://content/7b-2.jpg"]},
        {"id": "8c", "site": "vrbangers", "url": "https://vrbangers.com/video/in-library/", "score": 70, "vr": true,
         "inLibrary": true}
    ]});
    let pmvhaven = json!({"version": 1, "scenes": [
        {"id": "pmv1", "site": "pmvhaven", "url": "https://pmvhaven.com/video/x_pmv1", "title": "Flat PMV",
         "score": 2, "vr": false, "pmv": true, "script": {"axes": []}}
    ]});
    let faptap = json!({"generatedAt": "2026-11-15T00:00:00.000Z", "candidates": [
        {"id": "1957860091331874817", "title": "FapTap flat", "url": "https://faptap.net/v/1957860091331874817",
         "author": "scripter", "score": 3, "vr": false, "duration": 300, "projection": "MONO_FLAT", "quality": 1080}
    ]});
    fs::write(ud.join("eroscripts-scout.json"), waiting.to_string()).unwrap();
    for (name, v) in [
        ("slr-scout.json", &slr),
        ("vrb-scout.json", &vrb),
        ("pmvhaven-scout.json", &pmvhaven),
        ("faptap-scout.json", &faptap),
    ] {
        fs::write(ud.join(name), v.to_string()).unwrap();
    }
    // An SLR swipes file from before ids were strings keeps working.
    fs::write(
        ud.join("slr-scout-swipes.json"),
        r#"{"version":1,"swipes":{"70001":{"dir":"right","at":"2026-11-13T19:00:00.000Z"}}}"#,
    )
    .unwrap();
    let (_server, port) = start(&ud);
    let deck = |path: &str| -> Value { serde_json::from_slice(&get(port, path).2).unwrap() };
    let cards = |d: &Value| -> Vec<String> {
        d["cards"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| match &c["id"] {
                Value::String(s) => format!("{}:{s}", c["source"].as_str().unwrap()),
                n => format!("{}:{n}", c["source"].as_str().unwrap()),
            })
            .collect()
    };

    // Multi: the multi-axis post and the scene with hand-made axes.
    let multi = deck("/api/deck?stack=multi");
    assert_eq!(cards(&multi), ["eroscripts:31", "vrb:7b"]);
    assert_eq!(
        multi["cards"][1]["images"],
        json!(["https://content/7b.jpg", "https://content/7b-2.jpg"])
    );
    assert_eq!(multi["cards"][1]["axes"], json!(["twist", "roll"]));
    // VR: VRBangers same-scene loses to its post; 8c is in the library; nothing from SLR.
    let vr = deck("/api/deck?stack=vr");
    assert_eq!(cards(&vr), ["eroscripts:35"]);
    assert_eq!(
        (vr["remaining"].as_u64(), vr["wanted"].as_u64()),
        (Some(1), Some(0))
    );
    assert_eq!(vr["generatedAt"]["vrb"], "2026-11-17T18:00:00.000Z");
    // SLR: everything that downloads from SexLikeReal; post 34 wins over SLR 66249, 70001 is
    // downloaded.
    let slr_tab = deck("/api/deck?stack=slr");
    assert_eq!(cards(&slr_tab), ["slr:51207", "eroscripts:34"]);
    assert_eq!(slr_tab["quota"]["videos"]["remaining"], 30);
    assert_eq!(slr_tab["cards"][0]["scriptSource"], "slr-handcrafted");
    assert_eq!(
        slr_tab["cards"][1]["slrVideo"]["url"],
        "https://www.sexlikereal.com/scenes/b-66249"
    );
    // Flat: FapTap and PMVHaven.
    let flat = deck("/api/deck?stack=flat");
    assert_eq!(
        cards(&flat),
        ["faptap:1957860091331874817", "pmvhaven:pmv1"]
    );
    assert_eq!(flat["cards"][0]["runtimeMs"], 300_000);

    // One right swipe per source, each into its own file.
    for (path, body) in [
        ("/api/swipe", r#"{"id":34,"dir":"right"}"#),
        ("/api/scenes/slr/swipe", r#"{"id":51207,"dir":"right"}"#),
        ("/api/scenes/vrb/swipe", r#"{"id":"7b","dir":"right"}"#),
        (
            "/api/scenes/pmvhaven/swipe",
            r#"{"id":"pmv1","dir":"right"}"#,
        ),
        (
            "/api/scenes/faptap/swipe",
            r#"{"id":"1957860091331874817","dir":"right"}"#,
        ),
    ] {
        assert_eq!(post(port, path, body).0, 200, "{path}");
    }
    let read = |name: &str| -> Value {
        serde_json::from_slice(&fs::read(ud.join(name)).unwrap()).unwrap()
    };
    assert_eq!(
        read("eroscripts-scout-swipes.json")["swipes"]["34"]["dir"],
        "right"
    );
    let slr_swipes = read("slr-scout-swipes.json");
    assert_eq!(slr_swipes["swipes"]["51207"]["dir"], "right");
    assert_eq!(slr_swipes["swipes"]["70001"]["dir"], "right", "kept");
    assert_eq!(
        read("vrb-scout-swipes.json")["swipes"]["7b"]["dir"],
        "right"
    );
    assert_eq!(
        read("pmvhaven-scout-swipes.json")["swipes"]["pmv1"]["dir"],
        "right"
    );
    assert_eq!(
        read("faptap-scout-swipes.json")["swipes"]["1957860091331874817"]["dir"],
        "right"
    );
    for (tab, left, wanted) in [("multi", 1, 1), ("vr", 1, 0), ("slr", 0, 2), ("flat", 0, 2)] {
        let d = deck(&format!("/api/deck?stack={tab}"));
        assert_eq!(
            (d["remaining"].as_u64(), d["wanted"].as_u64()),
            (Some(left), Some(wanted)),
            "{tab}"
        );
    }

    // Undo, each through its own endpoint.
    assert_eq!(
        post(port, "/api/scenes/vrb/swipe/undo", r#"{"id":"7b"}"#).1["removed"],
        true
    );
    assert_eq!(
        post(port, "/api/swipe/undo", r#"{"id":34}"#).1["removed"],
        true
    );
    assert!(read("vrb-scout-swipes.json")["swipes"].get("7b").is_none());
    assert_eq!(cards(&deck("/api/deck?stack=multi")).len(), 2);
    assert_eq!(cards(&deck("/api/deck?stack=slr")).len(), 1);
    // The old SLR tab's routes are gone.
    assert_eq!(get(port, "/api/slr/deck").0, 404);
    assert_eq!(
        post(port, "/api/slr/swipe", r#"{"id":51207,"dir":"left"}"#).0,
        404
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

#[test]
fn a_slow_picture_download_does_not_hold_up_other_requests() {
    let tmp = TempDir::new("slow");
    let dir = tmp.0.join("creator-pictures/ann");
    fs::create_dir_all(&dir).unwrap();
    // Far more than the socket buffers hold, like the forum GIFs cards show.
    let big = vec![7u8; 16 << 20];
    fs::write(dir.join("es-1-0.jpg"), &big).unwrap();
    let (_server, port) = start(&tmp.0);

    // A client that asks for the picture and doesn't read it yet (a phone on slow Wi-Fi).
    let mut slow = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        slow,
        "GET /pictures/ann/es-1-0.jpg HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Everything else is still answered meanwhile.
    let started = std::time::Instant::now();
    assert_eq!(get(port, "/api/drive").0, 200);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "waited {:?}",
        started.elapsed()
    );

    // And the slow client still gets the whole picture, with a day's caching.
    let mut raw = Vec::new();
    slow.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..split]);
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
    assert!(
        head.contains("\r\nCache-Control: max-age=86400, immutable\r\n"),
        "{head}"
    );
    assert_eq!(raw.len() - split - 4, big.len());
}
