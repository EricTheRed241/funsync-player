//! End to end: import from a fake old userData, serve, list, stream a range.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

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
    let lib = tempfile::tempdir().unwrap();
    let scene = lib.path().join("Artist/Scene");
    std::fs::create_dir_all(&scene).unwrap();
    std::fs::write(scene.join("Scene.mp4"), b"0123456789").unwrap();
    std::fs::write(scene.join("Scene.funscript"), br#"{"actions":[]}"#).unwrap();
    std::fs::write(scene.join("Scene.twist.funscript"), br#"{"actions":[1]}"#).unwrap();
    let data = common::import(&serde_json::json!({
        "settings": {"library": {"sources": [
            {"id": "1", "name": "Lib", "path": lib.path(), "enabled": true}]}}
    }));
    let (_srv, port) = common::start(data.path(), &[]);
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
    // Pin a frame as the thumbnail (T in the player), then take the pin away.
    let pin = |body: &str| {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            s,
            "POST /api/media/pin-thumb/{id} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut buf = String::new();
        s.read_to_string(&mut buf).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(buf.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        v["thumbUrl"].as_str().unwrap().to_string()
    };
    let before = remote["videos"][0]["thumbUrl"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(
        pin(r#"{"pct": 0.42}"#),
        before,
        "a pin is a new thumbnail version"
    );
    assert_eq!(pin(r#"{"pct": null}"#), before);
}

fn ffmpeg_bin(name: &str) -> Option<String> {
    [
        format!("/opt/homebrew/bin/{name}"),
        format!("/usr/local/bin/{name}"),
    ]
    .into_iter()
    .find(|p| std::path::Path::new(p).exists())
}

/// The web remote's Quality setting: `?quality=720&t=1` streams a live,
/// scaled-down transcode starting 1 s in; an unknown quality is refused.
#[test]
fn stream_quality_transcodes_from_a_time() {
    let (Some(ffmpeg), Some(ffprobe)) = (ffmpeg_bin("ffmpeg"), ffmpeg_bin("ffprobe")) else {
        eprintln!("ffmpeg missing: transcode not checked");
        return;
    };
    let lib = tempfile::tempdir().unwrap();
    let scene = lib.path().join("Artist/Clip");
    std::fs::create_dir_all(&scene).unwrap();
    let clip = scene.join("Clip.mp4");
    let made = std::process::Command::new(&ffmpeg)
        .args(["-nostdin", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg("testsrc=size=1920x1080:rate=30:duration=4")
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(&clip)
        .status()
        .unwrap();
    assert!(made.success());
    std::fs::write(scene.join("Clip.funscript"), br#"{"actions":[]}"#).unwrap();
    let data = common::import(&serde_json::json!({
        "settings": {"library": {"sources": [
            {"id": "1", "name": "Lib", "path": lib.path(), "enabled": true}]}}
    }));
    let (_srv, port) = common::start(data.path(), &[]);
    let deadline = Instant::now() + Duration::from_secs(10);
    let id = loop {
        let (_, body) = http(port, "GET", "/api/videos", "");
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        if let Some(id) = v[0]["id"].as_str() {
            break id.to_string();
        }
        assert!(Instant::now() < deadline, "server never listed the video");
        std::thread::sleep(Duration::from_millis(100));
    };

    // HTTP/1.0: the streamed body comes close-delimited, not chunked.
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        s,
        "GET /api/media/stream/{id}?quality=720&t=1 HTTP/1.0\r\nHost: x\r\n\r\n"
    )
    .unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&buf[..split]).to_lowercase();
    assert!(
        head.starts_with("http/1.0 200") || head.starts_with("http/1.1 200"),
        "{head}"
    );
    assert!(head.contains("content-type: video/mp4"), "{head}");
    let out = data.path().join("out.mp4");
    std::fs::write(&out, &buf[split + 4..]).unwrap();
    let probe = std::process::Command::new(&ffprobe)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .args(["stream=height:format=duration", "-of", "default=nw=1"])
        .arg(&out)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&probe.stdout);
    assert!(text.contains("height=720"), "{text}");
    let secs: f64 = text
        .lines()
        .find_map(|l| l.strip_prefix("duration="))
        .and_then(|d| d.parse().ok())
        .unwrap_or(0.0);
    assert!(
        (2.5..=3.5).contains(&secs),
        "starts 1 s in: {secs} s of a 4 s clip"
    );

    let (head, _) = http(
        port,
        "GET",
        &format!("/api/media/stream/{id}?quality=4320"),
        "",
    );
    assert!(head.starts_with("HTTP/1.1 400"), "{head}");
}
