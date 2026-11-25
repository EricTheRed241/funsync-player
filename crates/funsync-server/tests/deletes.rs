//! End to end: triage and duplicate deletes happen only when the server owns
//! the library (`FUNSYNC_OWNS_LIBRARY=1`); temp dirs only.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, Instant};

use common::Kill;
use serde_json::{Value, json};

fn call(port: u16, method: &str, path: &str, body: Value) -> (u16, Value) {
    let body = body.to_string();
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
    (
        head[9..12].parse().unwrap(),
        serde_json::from_str(rest).unwrap_or(Value::Null),
    )
}

/// A library of three scenes under `Artist/`, served; the ids by file stem.
fn serve(
    lib: &Path,
    data: &Path,
    user_data: &Path,
    owns: bool,
) -> (Kill, u16, Vec<(String, String)>) {
    for name in ["A", "B", "C"] {
        let dir = lib.join("Artist").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{name}.mp4")), b"0").unwrap();
        std::fs::write(dir.join(format!("{name}.funscript")), b"{}").unwrap();
    }
    let library = json!({"version": 1, "config": {"settings": {"library": {"sources": [
        {"id": "1", "name": "Lib", "path": lib, "enabled": true}]}}}});
    std::fs::write(data.join("library.json"), library.to_string()).unwrap();
    let owns = if owns { "1" } else { "0" };
    let user_data = user_data.to_str().unwrap();
    let (srv, port) = common::start(
        data,
        &[
            ("FUNSYNC_OWNS_LIBRARY", owns),
            ("FUNSYNC_USER_DATA", user_data),
        ],
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            let v = call(port, "GET", "/api/remote/videos", json!({})).1;
            if let Some(a) = v["videos"].as_array().filter(|a| a.len() == 3) {
                let ids = a
                    .iter()
                    .map(|v| {
                        let stem = v["name"].as_str().unwrap().to_string();
                        (stem, v["id"].as_str().unwrap().to_string())
                    })
                    .collect();
                return (srv, port, ids);
            }
        }
        assert!(Instant::now() < deadline, "server never listed the videos");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn id<'a>(ids: &'a [(String, String)], stem: &str) -> &'a str {
    &ids.iter().find(|(n, _)| n.starts_with(stem)).unwrap().1
}

#[test]
fn deletes_only_when_owning_the_library() {
    let (lib, data, ud) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let (srv, port, ids) = serve(lib.path(), data.path(), ud.path(), false);
    let a = lib.path().join("Artist/A/A.mp4");
    call(
        port,
        "POST",
        "/api/remote/rating",
        json!({"videoId": id(&ids, "A"), "rating": 1}),
    );
    assert!(a.exists(), "not owning: a low rating deletes nothing");
    let (code, v) = call(
        port,
        "POST",
        "/api/remote/delete-duplicate",
        json!({"videoId": id(&ids, "B"), "keepId": id(&ids, "C")}),
    );
    assert_eq!(
        (code, v["detail"].as_str()),
        (409, Some("the Electron app owns the library"))
    );
    drop(srv);

    let (lib, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (_srv, port, ids) = serve(lib.path(), data.path(), ud.path(), true);
    let artist = lib.path().join("Artist");
    // A favourite is kept; a plain 2★ goes.
    call(
        port,
        "POST",
        "/api/remote/rating",
        json!({"videoId": id(&ids, "A"), "rating": 2, "favorite": true}),
    );
    assert!(artist.join("A/A.mp4").exists());
    call(
        port,
        "POST",
        "/api/remote/rating",
        json!({"videoId": id(&ids, "A"), "favorite": false}),
    );
    assert!(!artist.join("A").exists(), "scene folder deleted");
    let (code, _) = call(
        port,
        "POST",
        "/api/remote/delete-duplicate",
        json!({"videoId": id(&ids, "B"), "keepId": id(&ids, "C")}),
    );
    assert_eq!(code, 200);
    assert!(!artist.join("B").exists() && artist.join("C/C.mp4").exists());
    let v = call(port, "GET", "/api/remote/videos", json!({})).1;
    assert_eq!(v["videos"].as_array().unwrap().len(), 1);
    let saved: Value =
        serde_json::from_slice(&std::fs::read(data.path().join("library.json")).unwrap()).unwrap();
    assert_eq!(saved["config"]["settings"]["library"]["ratings"], json!({}));
}
