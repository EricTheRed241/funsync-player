//! End to end: a fake TCode WebSocket device receives the script's strokes
//! while a fake phone plays the video over `/api/remote/sync`.

use std::process::{Child, Command};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn get(port: u16, path: &str) -> serde_json::Value {
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).await.unwrap();
    serde_json::from_str(buf.split("\r\n\r\n").nth(1).unwrap_or("")).unwrap_or_default()
}

async fn raw(port: u16, path: &str) -> String {
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).await.unwrap();
    buf
}

async fn post(port: u16, path: &str, body: &str) -> serde_json::Value {
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).await.unwrap();
    serde_json::from_str(buf.split("\r\n\r\n").nth(1).unwrap_or("")).unwrap()
}

#[tokio::test]
async fn phone_playback_drives_a_tcode_websocket_device() {
    // The fake device.
    let dev = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dev_port = dev.local_addr().unwrap().port();

    let bin = env!("CARGO_BIN_EXE_funsync-server");
    let (lib, old, data) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    std::fs::write(lib.path().join("Scene.mp4"), b"0").unwrap();
    std::fs::write(
        lib.path().join("Scene.funscript"),
        br#"{"actions":[{"at":0,"pos":0},{"at":500,"pos":100},{"at":1000,"pos":0},{"at":60000,"pos":100}]}"#,
    )
    .unwrap();
    // A variant with its own hand-made twist and a generated roll.
    let flat = br#"{"actions":[{"at":0,"pos":100},{"at":60000,"pos":100}]}"#;
    for f in [
        "Scene (Soft).funscript",
        "Scene (Soft).twist.funscript",
        "Scene (Soft).generated.roll.funscript",
    ] {
        std::fs::write(lib.path().join(f), flat).unwrap();
    }
    let cfg = serde_json::json!({"settings": {
        "library": {"sources": [{"id": "1", "name": "Lib", "path": lib.path(), "enabled": true}]},
        "tcode": {"transport": "websocket", "wsUrl": format!("ws://127.0.0.1:{dev_port}"),
                  "autoConnect": true, "axes": {"L0": {"min": 0, "max": 100}}},
    }});
    std::fs::write(old.path().join("config.json"), cfg.to_string()).unwrap();
    let ok = Command::new(bin)
        .args(["import", "--from"])
        .arg(old.path())
        .arg("--data-dir")
        .arg(data.path())
        .status()
        .unwrap();
    assert!(ok.success());
    // An import never carries auto-connect over (the desktop app holds its
    // devices); this server was told to connect, as its Devices page does.
    let lib_file = data.path().join("library.json");
    let mut saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&lib_file).unwrap()).unwrap();
    saved["config"]["settings"]["tcode"]["autoConnect"] = true.into();
    std::fs::write(&lib_file, saved.to_string()).unwrap();
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

    // The server auto-connects to the device.
    let (sock, _) = tokio::time::timeout(Duration::from_secs(10), dev.accept())
        .await
        .expect("server never connected to the device")
        .unwrap();
    let mut device = tokio_tungstenite::accept_async(sock).await.unwrap();

    let id = loop {
        let v = get(port, "/api/videos").await;
        if let Some(id) = v[0]["id"].as_str() {
            break id.to_string();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let status = get(port, "/api/devices").await;
    assert_eq!(status["status"]["tcode"]["connected"], true, "{status}");

    let url = format!("ws://127.0.0.1:{port}/api/remote/sync?videoId={id}");
    let (mut phone, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let mut got = Vec::new();
    while got.len() < 2 {
        if let Some(Ok(Message::Text(t))) = phone.next().await {
            got.push(serde_json::from_str::<serde_json::Value>(&t).unwrap());
        }
    }
    assert_eq!(got[0]["type"], "script-ready");
    assert_eq!(got[1]["type"], "device-status");
    assert_eq!(got[1]["devices"][0]["kind"], "tcode");

    for m in [
        r#"{"type":"hello","duration":60}"#,
        r#"{"type":"state","at":0,"paused":false,"rate":1,"duration":60}"#,
    ] {
        phone.send(Message::text(m)).await.unwrap();
    }
    let mut frames = Vec::new();
    while frames.len() < 10 {
        let m = tokio::time::timeout(Duration::from_secs(5), device.next())
            .await
            .expect("no TCode frames")
            .unwrap()
            .unwrap();
        frames.push(m.into_text().unwrap().to_string());
    }
    for f in &frames {
        let cmd = f.trim_end();
        assert!(f.ends_with('\n'), "{f:?}");
        assert!(cmd.starts_with("L0") && cmd[2..6].bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(&cmd[6..], "I17", "{f:?}");
    }
    // Rising toward 100 during the first half second.
    let pos = |f: &str| f[2..6].parse::<u32>().unwrap();
    assert!(pos(&frames[9]) > pos(&frames[0]), "{frames:?}");

    // Paused: the device gets nothing more.
    phone
        .send(Message::text(r#"{"type":"pause"}"#))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), device.next()).await {}
    assert!(
        tokio::time::timeout(Duration::from_millis(300), device.next())
            .await
            .is_err()
    );

    // Switch to the variant: its own hand-made twist plays, not its
    // generated roll.
    phone
        .send(Message::text(r#"{"type":"switch-variant","label":"Soft"}"#))
        .await
        .unwrap();
    let mut types = Vec::new();
    while !types.contains(&"variant-changed".to_string()) {
        if let Some(Ok(Message::Text(t))) = phone.next().await {
            let m: serde_json::Value = serde_json::from_str(&t).unwrap();
            types.push(m["type"].as_str().unwrap().to_string());
        }
    }
    assert_eq!(types, ["script-ready", "variant-changed"]);
    phone
        .send(Message::text(r#"{"type":"play"}"#))
        .await
        .unwrap();
    let f = loop {
        let m = tokio::time::timeout(Duration::from_secs(5), device.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let f = m.into_text().unwrap().to_string();
        if f.starts_with("L09999") {
            break f;
        }
    };
    assert_eq!(f, "L09999I17 R09999I17\n");

    // The status API and the dashboard (this computer only).
    let status = get(port, "/api/status").await;
    assert_eq!(status["devices"]["tcode"]["link"]["state"], "connected");
    assert_eq!(status["session"]["state"], "playing");
    assert_eq!(status["phone"]["connected"], true);
    assert_eq!(status["issues"], serde_json::json!([]));
    assert_eq!(status["videos"], 1, "the server's own status stays");
    let tele = get(port, "/api/device/telemetry").await;
    assert!(
        tele["commands"][0]["cmd"]
            .as_str()
            .unwrap()
            .starts_with("L0")
    );
    assert_eq!(tele["status"]["desktop"]["connected"], true);
    let page = raw(port, "/device/").await;
    assert!(page.starts_with("HTTP/1.1 200") && page.contains("dashboard.js"));

    // A newer phone takes over; the older one is told so.
    let url = format!("ws://127.0.0.1:{port}/api/remote/sync?videoId={id}");
    let (_phone2, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let kicked = loop {
        let m = tokio::time::timeout(Duration::from_secs(5), phone.next())
            .await
            .unwrap();
        if let Some(Ok(Message::Text(t))) = m {
            let m: serde_json::Value = serde_json::from_str(&t).unwrap();
            if m["type"] == "kicked" {
                break m;
            }
        }
    };
    assert_eq!(kicked["reason"], "Another device took over this session.");

    // Picking the FUNSR MINI profile swaps the TCode settings.
    let d = post(
        port,
        "/api/devices",
        r#"{"kind":"tcode","profile":"funsr-mini"}"#,
    )
    .await;
    assert_eq!(d["profile"], "funsr-mini");
    assert_eq!(d["tcode"]["transport"], "serial");
    assert_eq!(d["tcode"]["axes"]["L0"]["max"], 95);
    let d = post(
        port,
        "/api/devices",
        r#"{"kind":"tcode","profile":"custom","action":"disconnect"}"#,
    )
    .await;
    assert_eq!(d["tcode"]["wsUrl"], format!("ws://127.0.0.1:{dev_port}"));
}
