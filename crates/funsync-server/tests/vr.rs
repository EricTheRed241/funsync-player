//! End to end: the HereSphere listing and scene detail against a temp library.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn post(port: u16, path: &str) -> Value {
    post_json(port, path, json!({}))
}

fn post_json(port: u16, path: &str, body: Value) -> Value {
    let body = body.to_string();
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        s,
        "POST {path} HTTP/1.1\r\nHost: hs.local:9\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    serde_json::from_slice(&buf[split + 4..]).unwrap()
}

#[test]
fn heresphere_listing_and_scene() {
    let bin = env!("CARGO_BIN_EXE_funsync-server");
    let lib = tempfile::tempdir().unwrap();
    let old = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let dir = lib.path().join("Studio");
    std::fs::create_dir_all(&dir).unwrap();
    let vr = dir.join("Scene_180_sbs.mp4");
    let flat = dir.join("Flat.mp4");
    std::fs::write(&vr, b"0123456789").unwrap();
    std::fs::write(dir.join("Scene_180_sbs.funscript"), br#"{"actions":[]}"#).unwrap();
    std::fs::write(&flat, b"0123456789").unwrap();
    let cfg = json!({
        "settings": {"library": {
            "sources": [{"id": "1", "name": "Lib", "path": lib.path(), "enabled": true}],
            "ratings": {flat.to_str().unwrap(): {"rating": 4, "updatedAt": 1}},
            // Score 3 (VR) + 20 (tags), computed by the server after the scan;
            // VR picks deleted by the user, so it isn't made again.
            "scriptTags": {vr.to_str().unwrap(): {"tags": ["a", "b"]}},
            "tagScores": {"a": 10, "b": 10},
            "smartPlaylists": {"created": ["vr-picks"]},
            "durationCache": {vr.to_str().unwrap(): 700},
        }},
        "playlists": [{"id": "p", "name": "Picks", "videoPaths": [vr]}],
    });
    std::fs::write(old.path().join("config.json"), cfg.to_string()).unwrap();
    assert!(
        Command::new(bin)
            .args(["import", "--from"])
            .arg(old.path())
            .arg("--data-dir")
            .arg(data.path())
            .status()
            .unwrap()
            .success()
    );
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let _srv = Kill(
        Command::new(bin)
            .args(["--port", &port.to_string(), "--data-dir"])
            .arg(data.path())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let listing = loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            let v = post(port, "/heresphere");
            if v["library"].as_array().is_some_and(|a| !a.is_empty()) {
                break v;
            }
        }
        assert!(Instant::now() < deadline, "no listing");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(listing["access"], 1);
    let sections: Vec<(&str, usize)> = listing["library"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap(),
                s["list"].as_array().unwrap().len(),
            )
        })
        .collect();
    assert_eq!(
        sections,
        [
            // the scripted VR video is auto-filled into the queue
            ("Queue", 1),
            ("Score 20+", 1),
            ("VR", 1),
            ("Flat", 1),
            ("Medium (5-15m)", 1),
            ("Recently Added", 2),
            ("Top Rated", 1),
            ("Unrated", 1),
            ("Watched", 1),
            ("Playlist: Picks", 1),
        ]
    );
    let url = listing["library"][1]["list"][0].as_str().unwrap();
    let path = url.strip_prefix("http://hs.local:9").unwrap();
    let scene = post(port, path);
    let id = path.rsplit('/').next().unwrap();
    assert_eq!(scene["title"], "Scene_180_sbs");
    assert_eq!(scene["projection"], "equirectangular");
    assert_eq!(scene["stereo"], "sbs");
    assert_eq!(scene["fov"], 180.0);
    assert_eq!(scene["duration"], 700_000);
    assert_eq!(
        scene["scripts"][0]["url"],
        format!("http://hs.local:9/api/media/script/{id}")
    );
    assert_eq!(
        scene["media"][0]["sources"][0]["url"],
        format!("http://hs.local:9/api/media/stream/{id}")
    );
    let tags: Vec<&str> = scene["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        tags,
        [
            "Studio:Lib",
            "Feature:Is scripted",
            "Format:VR",
            "Length:Medium (5-15m)",
            "Queue:Up next",
            "Playlist:Picks",
            "Category:VR",
            "Category:Single-axis",
            "Tag:a",
            "Tag:b",
            "Score:23.0",
            "5 min",
            "10 min"
        ]
    );
    assert_eq!(scene["writeRating"], true);
    assert_eq!(
        scene["eventServer"],
        format!("http://hs.local:9/heresphere/events/{id}")
    );

    // Headset edits: rating, heart, a new playlist tag; Picks kept.
    let edit = json!({"rating": 3, "isFavorite": true, "tags": [
        {"name": "Playlist:Picks"}, {"name": "Playlist:Headset"}, {"name": "Queue:Up next"}]});
    let scene = post_json(port, path, edit);
    assert_eq!(
        (scene["rating"].clone(), scene["isFavorite"].clone()),
        (json!(3.0), json!(true))
    );
    let has = |s: &Value, t: &str| s["tags"].as_array().unwrap().iter().any(|x| x["name"] == t);
    assert!(has(&scene, "Playlist:Headset") && has(&scene, "Playlist:Picks"));
    // A play event answers `{}`; a close at the end of a rated video files
    // nothing under Needs review.
    assert_eq!(
        post_json(
            port,
            &format!("/heresphere/events/{id}"),
            json!({"event": 1, "time": 0})
        ),
        json!({})
    );
    post_json(
        port,
        &format!("/heresphere/events/{id}"),
        json!({"event": 3, "time": 699_000}),
    );
    let scene = post(port, path);
    assert!(!has(&scene, "Review:Needed"));

    let missing = post(port, "/heresphere/nope");
    assert_eq!(missing["detail"], "Scene not found");
}
