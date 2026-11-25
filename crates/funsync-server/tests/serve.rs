//! End to end: import from a fake old userData, serve, list, stream a range.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn http(port: u16, method: &str, path: &str, extra: &str) -> (String, Vec<u8>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: 0\r\n{extra}\r\n"
    )
    .unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    (
        String::from_utf8_lossy(&buf[..split]).into_owned(),
        buf[split + 4..].to_vec(),
    )
}

#[test]
fn import_then_serve() {
    let bin = env!("CARGO_BIN_EXE_funsync-server");
    let lib = tempfile::tempdir().unwrap();
    let old = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let scene = lib.path().join("Artist/Scene");
    std::fs::create_dir_all(&scene).unwrap();
    std::fs::write(scene.join("Scene.mp4"), b"0123456789").unwrap();
    std::fs::write(scene.join("Scene.funscript"), br#"{"actions":[]}"#).unwrap();
    std::fs::write(scene.join("Scene.twist.funscript"), br#"{"actions":[1]}"#).unwrap();
    let cfg = serde_json::json!({
        "settings": {"library": {"sources": [
            {"id": "1", "name": "Lib", "path": lib.path(), "enabled": true}]}}
    });
    std::fs::write(old.path().join("config.json"), cfg.to_string()).unwrap();

    let st = Command::new(bin)
        .args(["import", "--from"])
        .arg(old.path())
        .arg("--data-dir")
        .arg(data.path())
        .status()
        .unwrap();
    assert!(st.success());

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
    let videos: serde_json::Value = loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            let (_, body) = http(port, "GET", "/api/videos", "");
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            if v.as_array().is_some_and(|a| !a.is_empty()) {
                break v;
            }
        }
        assert!(Instant::now() < deadline, "server never listed the video");
        std::thread::sleep(Duration::from_millis(100));
    };
    let id = videos[0]["id"].as_str().unwrap().to_string();
    assert_eq!(videos[0]["axes"][0]["axis"], "twist");

    let (head, body) = http(
        port,
        "GET",
        &format!("/api/media/stream/{id}"),
        "Range: bytes=2-5\r\n",
    );
    assert!(head.starts_with("HTTP/1.1 206"), "{head}");
    assert_eq!(body, b"2345");

    let (_, body) = http(
        port,
        "GET",
        &format!("/api/media/script/{id}?axis=twist"),
        "",
    );
    assert_eq!(body, br#"{"actions":[1]}"#);

    let (head, _) = http(port, "GET", "/api/media/stream/nope", "");
    assert!(head.starts_with("HTTP/1.1 404"), "{head}");

    let (_, body) = http(port, "GET", "/api/status", "");
    let status: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(status["videos"], 1);
    assert_eq!(status["sources"][0]["present"], true);

    // The web remote's API and files.
    let (_, body) = http(port, "GET", "/api/remote/videos", "");
    let remote: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(remote["videos"][0]["name"], "Scene");
    assert_eq!(
        remote["videos"][0]["scriptUrl"],
        format!("/api/media/script/{id}")
    );
    let (head, body) = http(port, "GET", "/remote/", "");
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    let page = String::from_utf8_lossy(&body);
    assert!(page.contains("app.js") && page.contains("/remote/extras.js"));
    // The library rules ran after the scan; the old backend's flag endpoints.
    let (_, body) = http(port, "GET", "/api/media/axis-flags", "");
    let flags: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let path = videos[0]["path"].as_str().unwrap();
    assert_eq!(flags["flags"][path], "handmade");
    let (_, body) = http(port, "GET", "/api/media/vr-flags", "");
    let flags: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(flags["flags"][path], false);
    let (_, body) = http(port, "GET", "/api/remote/categories", "");
    assert!(String::from_utf8_lossy(&body).contains("Hand-made axes"));
    let (_, body) = http(port, "GET", "/api/parity", "");
    let parity: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(parity["counts"]["done"].as_u64().unwrap() > 0);
}
