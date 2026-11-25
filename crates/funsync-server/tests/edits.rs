//! End to end: the web remote's write API and play queue over HTTP, saved
//! to library.json; nothing in the library folder changes.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

fn call(port: u16, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).unwrap();
    let (head, rest) = buf.split_once("\r\n\r\n").unwrap();
    let code = head[9..12].parse().unwrap();
    (code, serde_json::from_str(rest).unwrap_or(Value::Null))
}

fn post(port: u16, path: &str, body: Value) -> Value {
    let (code, v) = call(port, "POST", path, Some(body));
    assert_eq!(code, 200, "{path}: {v}");
    v
}

#[test]
fn edits_over_http() {
    let lib = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    for scene in ["Mira/One", "Mira/Two"] {
        let dir = lib.path().join(scene);
        std::fs::create_dir_all(&dir).unwrap();
        let name = scene.rsplit('/').next().unwrap();
        std::fs::write(dir.join(format!("{name}.mp4")), b"0").unwrap();
        std::fs::write(dir.join(format!("{name}.funscript")), b"{}").unwrap();
    }
    let library = json!({"version": 1, "config": {"settings": {"library": {"sources": [
        {"id": "1", "name": "Lib", "path": lib.path(), "enabled": true}]}}}});
    std::fs::write(data.path().join("library.json"), library.to_string()).unwrap();

    let (_srv, port) = common::start(data.path(), &[]);
    let deadline = Instant::now() + Duration::from_secs(10);
    let videos = loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            let v = call(port, "GET", "/api/remote/videos", None).1;
            if v["videos"].as_array().is_some_and(|a| a.len() == 2) {
                break v["videos"].clone();
            }
        }
        assert!(Instant::now() < deadline, "server never listed the videos");
        std::thread::sleep(Duration::from_millis(100));
    };
    let id = videos[0]["id"].as_str().unwrap().to_string();
    let other = videos[1]["id"].as_str().unwrap().to_string();

    // Rating: a low one is kept (no delete), then a heart.
    let r = post(
        port,
        "/api/remote/rating",
        json!({"videoId": id, "rating": 1}),
    );
    assert_eq!(
        (r["ok"].clone(), r["rating"].clone()),
        (json!(true), json!(1.0))
    );
    let r = post(
        port,
        "/api/remote/rating",
        json!({"videoId": id, "favorite": true}),
    );
    assert_eq!(
        (r["rating"].clone(), r["favorite"].clone()),
        (json!(1.0), json!(true))
    );
    assert_eq!(
        call(
            port,
            "POST",
            "/api/remote/rating",
            Some(json!({"videoId": id, "rating": 4.2}))
        )
        .0,
        400
    );
    assert_eq!(
        call(
            port,
            "POST",
            "/api/remote/rating",
            Some(json!({"videoId": "nope", "rating": 4}))
        )
        .0,
        404
    );
    assert!(
        lib.path().join("Mira/One/One.mp4").exists()
            && lib.path().join("Mira/Two/Two.mp4").exists()
    );
    let v = call(port, "GET", "/api/remote/videos", None).1;
    let me = v["videos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == id.as_str())
        .unwrap()
        .clone();
    assert_eq!(
        (me["rating"].clone(), me["favorite"].clone()),
        (json!(1.0), json!(true))
    );

    // Playlists: create by name, add by id, remove.
    post(
        port,
        "/api/remote/playlists/add-video",
        json!({"videoId": id, "name": "Needs review"}),
    );
    let pl = named(
        &call(port, "GET", "/api/remote/playlists", None).1["playlists"],
        "Needs review",
    );
    assert_eq!(
        (pl["name"].clone(), pl["videoIds"].clone()),
        (json!("Needs review"), json!([id]))
    );
    post(
        port,
        "/api/remote/playlists/add-video",
        json!({"videoId": other, "playlistId": pl["id"]}),
    );
    post(
        port,
        "/api/remote/playlists/remove-video",
        json!({"videoId": id, "playlistId": pl["id"]}),
    );
    let pl = named(
        &call(port, "GET", "/api/remote/playlists", None).1["playlists"],
        "Needs review",
    );
    assert_eq!(pl["videoIds"], json!([other]));

    // Queue: Needs review isn't auto-filled; add puts it first.
    let q = call(port, "GET", "/api/remote/queue", None).1;
    assert_eq!(
        q,
        json!({"items": [{"videoId": id, "auto": true}], "autoFill": true})
    );
    let r = post(
        port,
        "/api/remote/queue/add",
        json!({"videoId": other, "position": "next"}),
    );
    assert_eq!(
        r["queue"]["items"][0],
        json!({"videoId": other, "auto": false})
    );
    // Played leaves the user's queue (and `other` is under review: no auto-fill).
    let r = post(port, "/api/remote/queue/played", json!({"videoId": other}));
    assert_eq!(r["queue"]["items"], json!([{"videoId": id, "auto": true}]));
    let r = post(
        port,
        "/api/remote/queue/auto-fill",
        json!({"enabled": false}),
    );
    assert_eq!(r["queue"]["autoFill"], false);
    post(port, "/api/remote/queue/clear", json!({}));

    // Tag scores, artists.
    let r = post(
        port,
        "/api/remote/tag-score",
        json!({"tag": " Solo ", "score": 42}),
    );
    assert_eq!(
        (r["tag"].clone(), r["score"].clone()),
        (json!("solo"), json!(10.0))
    );
    let t = call(port, "GET", "/api/remote/tag-scores", None).1;
    assert_eq!(
        (t["scores"]["solo"].clone(), t["recalculating"].clone()),
        (json!(10.0), json!(false))
    );
    post(port, "/api/remote/recalculate-scores", json!({}));
    post(
        port,
        "/api/remote/favourite-artist",
        json!({"artist": "mira", "favourite": true}),
    );
    post(port, "/api/remote/favourite-score", json!({"score": 7}));
    let a = call(port, "GET", "/api/remote/artists", None).1;
    let mira = a["artists"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "mira")
        .unwrap()
        .clone();
    assert_eq!(
        (mira["favourite"].clone(), mira["videoCount"].clone()),
        (json!(true), json!(2))
    );
    assert_eq!(a["score"], 7.0);

    let saved: Value =
        serde_json::from_slice(&std::fs::read(data.path().join("library.json")).unwrap()).unwrap();
    let c = &saved["config"];
    assert_eq!(c["settings"]["library"]["tagScores"]["solo"], 10.0);
    assert_eq!(c["settings"]["player"]["userQueue"], json!([]));
    assert_eq!(c["settings"]["player"]["queueAutoFill"], false);
    // The library rules ran after the edits: the heart made Favorites, the
    // favourite artist "Favourite artists".
    let names: Vec<&str> = c["playlists"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Favorites", "Needs review", "Favourite artists"]);
}

/// The playlist called `name` in a list.
fn named(list: &Value, name: &str) -> Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap()
        .clone()
}
