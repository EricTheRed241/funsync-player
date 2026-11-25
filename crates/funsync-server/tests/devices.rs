//! End to end: a fake TCode WebSocket device receives the script's strokes
//! while a fake phone plays the video over `/api/remote/sync`.

mod common;

use std::time::Duration;

use common::Kill;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;

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

    let lib = tempfile::tempdir().unwrap();
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
    let (_srv, port, _data) = serve(serde_json::json!({"settings": {
        "library": {"sources": [{"id": "1", "name": "Lib", "path": lib.path(), "enabled": true}]},
        "tcode": {"transport": "websocket", "wsUrl": format!("ws://127.0.0.1:{dev_port}"),
                  "axes": {"L0": {"min": 0, "max": 100}}},
    }}));

    // The server auto-connects to the device.
    let (sock, _) = tokio::time::timeout(Duration::from_secs(30), dev.accept())
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
    while got.len() < 3 {
        if let Some(Ok(Message::Text(t))) = phone.next().await {
            got.push(serde_json::from_str::<serde_json::Value>(&t).unwrap());
        }
    }
    assert_eq!(got[0]["type"], "script-ready");
    assert_eq!(got[1]["type"], "device-status");
    assert_eq!(got[1]["devices"][0]["kind"], "tcode");
    assert_eq!(got[2]["type"], "orgasm-state");
    assert_eq!(got[2]["configured"], false);

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

/// Import `cfg` and serve it with TCode auto-connect on again (an import
/// never carries it over; the Devices page's Connect sets it).
fn serve(cfg: serde_json::Value) -> (Kill, u16, tempfile::TempDir) {
    let data = common::import(&cfg);
    let lib_file = data.path().join("library.json");
    let mut saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&lib_file).unwrap()).unwrap();
    saved["config"]["settings"]["tcode"]["autoConnect"] = true.into();
    std::fs::write(&lib_file, saved.to_string()).unwrap();
    let (srv, port) = common::start(data.path(), &[]);
    (srv, port, data)
}

/// The next TCode frame the fake device gets that starts with `prefix`
/// (frames already queued that don't match are skipped; a generous budget,
/// the suite runs many servers at once).
async fn frame_starting(
    device: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    prefix: &str,
) -> String {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(20), device.next())
            .await
            .unwrap_or_else(|_| panic!("no frame starting with {prefix}"))
            .unwrap()
            .unwrap();
        let f = m.into_text().unwrap().to_string();
        if f.starts_with(prefix) {
            return f;
        }
    }
}

/// Poll `/api/status` until `ok` holds (20 s at most).
async fn status_until(port: u16, ok: impl Fn(&serde_json::Value) -> bool) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let s = get(port, "/api/status").await;
        if ok(&s) {
            return s;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "status never got there: {s}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn headset_bridge_then_phone_take_over_and_orgasm_switch() {
    let dev = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dev_port = dev.local_addr().unwrap().port();
    let hs = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hs_port = hs.local_addr().unwrap().port();
    let lib = tempfile::tempdir().unwrap();
    std::fs::write(lib.path().join("Scene.mp4"), b"0").unwrap();
    std::fs::write(
        lib.path().join("Scene.funscript"),
        br#"{"actions":[{"at":0,"pos":0},{"at":60000,"pos":100}]}"#,
    )
    .unwrap();
    let finisher = lib.path().join("finisher.json");
    std::fs::write(
        &finisher,
        br#"{"actions":[{"at":0,"pos":20},{"at":1000,"pos":20}]}"#,
    )
    .unwrap();
    let (_srv, port, _data) = serve(serde_json::json!({"settings": {
        "library": {"sources": [{"id": "1", "name": "Lib", "path": lib.path(), "enabled": true}]},
        "tcode": {"transport": "websocket", "wsUrl": format!("ws://127.0.0.1:{dev_port}"),
                  "axes": {"L0": {"min": 0, "max": 100}}},
        "vr": {"lastHost": "127.0.0.1", "lastPort": hs_port},
        "player": {"orgasmConfig": {"active": "single", "single": finisher}},
    }}));
    let (sock, _) = tokio::time::timeout(Duration::from_secs(30), dev.accept())
        .await
        .unwrap()
        .unwrap();
    let mut device = tokio_tungstenite::accept_async(sock).await.unwrap();
    let id = loop {
        if let Some(id) = get(port, "/api/videos").await[0]["id"].as_str() {
            break id.to_string();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };

    // The headset plays Scene.mp4 at 30 s: the bridge connects and drives.
    post(port, "/api/devices", r#"{"kind":"vr","action":"connect"}"#).await;
    let (mut headset, _) = tokio::time::timeout(Duration::from_secs(30), hs.accept())
        .await
        .unwrap()
        .unwrap();
    tokio::spawn(async move {
        for i in 0.. {
            let p = serde_json::json!({"path": "C:\\VR\\Scene.mp4",
                "currentTime": 30.0 + i as f64 * 0.1, "playbackSpeed": 1, "playerState": 0})
            .to_string();
            let mut buf = (p.len() as u32).to_le_bytes().to_vec();
            buf.extend(p.as_bytes());
            if headset.write_all(&buf).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    // Wait on the server's state (one status reads its parts one after the
    // other, so wait until they all agree), then on the frames it makes.
    let s = status_until(port, |s| {
        s["vrBridge"]["linkState"] == "receiving"
            && s["sessions"]["current"]["state"] == "playing"
            && s["session"]["source"] == "VR"
    })
    .await;
    assert_eq!(s["vrBridge"]["video"], "Scene.mp4");
    assert_eq!(s["sessions"]["current"]["source"], "vr");
    assert_eq!(s["session"]["source"], "VR");
    let f = frame_starting(&mut device, "L0").await;
    let pos: u32 = f[2..6].parse().unwrap();
    assert!((4500..5600).contains(&pos), "{f}");

    // A phone opens the video: last wins, the bridge is dropped.
    let url = format!("ws://127.0.0.1:{port}/api/remote/sync?videoId={id}");
    let (mut phone, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let mut configured = None;
    while configured.is_none() {
        if let Some(Ok(Message::Text(t))) = phone.next().await {
            let m: serde_json::Value = serde_json::from_str(&t).unwrap();
            if m["type"] == "orgasm-state" {
                configured = Some(m["configured"].clone());
            }
        }
    }
    assert_eq!(configured, Some(true.into()));
    let s = get(port, "/api/status").await;
    assert_eq!(s["vrBridge"]["connected"], false, "{s}");
    assert_eq!(s["sessions"]["current"]["source"], "web-remote");
    assert_eq!(s["sessions"]["history"][0]["source"], "vr");
    assert_eq!(
        s["sessions"]["history"][0]["videos"][0]["name"],
        "Scene.mp4"
    );

    // Hold the Orgasm Switch: the finisher loop plays though the video is
    // paused (once the server says it's on; older VR frames may still be
    // queued on the device socket).
    phone
        .send(Message::text(r#"{"type":"orgasm-hold","active":true}"#))
        .await
        .unwrap();
    loop {
        if let Some(Ok(Message::Text(t))) = phone.next().await {
            let m: serde_json::Value = serde_json::from_str(&t).unwrap();
            if m["type"] == "orgasm-state" && m["active"] == true {
                break;
            }
        }
    }
    while frame_starting(&mut device, "L0").await != "L02000I17\n" {}
}

#[tokio::test]
async fn routing_editor_sends_tcode_another_script_and_edge_holds_it() {
    let dev = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dev_port = dev.local_addr().unwrap().port();
    let lib = tempfile::tempdir().unwrap();
    std::fs::write(lib.path().join("Scene.mp4"), b"0").unwrap();
    std::fs::write(
        lib.path().join("Scene.funscript"),
        br#"{"actions":[{"at":0,"pos":0},{"at":60000,"pos":100}]}"#,
    )
    .unwrap();
    let alt = lib.path().join("Alt.funscript");
    std::fs::write(
        &alt,
        br#"{"actions":[{"at":0,"pos":30},{"at":60000,"pos":30}]}"#,
    )
    .unwrap();
    let (_srv, port, data) = serve(serde_json::json!({"settings": {
        "library": {"sources": [{"id": "1", "name": "Lib", "path": lib.path(), "enabled": true}]},
        "tcode": {"transport": "websocket", "wsUrl": format!("ws://127.0.0.1:{dev_port}"),
                  "axes": {"L0": {"min": 0, "max": 100}}},
    }}));
    let (sock, _) = tokio::time::timeout(Duration::from_secs(30), dev.accept())
        .await
        .unwrap()
        .unwrap();
    let mut device = tokio_tungstenite::accept_async(sock).await.unwrap();
    let id = loop {
        if let Some(id) = get(port, "/api/videos").await[0]["id"].as_str() {
            break id.to_string();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };

    // The editor lists the folder's scripts and the devices.
    let r = get(port, &format!("/api/devices/routing?videoId={id}")).await;
    assert_eq!(r["scripts"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!(r["devices"][0]["deviceId"], "tcode");
    let body = serde_json::json!({"videoId": id,
        "routes": [{"deviceId": "tcode", "scriptPath": alt}]})
    .to_string();
    let r = post(port, "/api/devices/routing", &body).await;
    assert_eq!(r["active"], "custom");
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(data.path().join("library.json")).unwrap()).unwrap();
    let entry = &saved["config"]["settings"]["library"]["associations"]
        [lib.path().join("Scene.mp4").to_str().unwrap()];
    assert_eq!(entry["mode"], "custom");
    assert_eq!(entry["routes"][0]["deviceId"], "tcode");
    assert!(entry.get("single").is_some() && entry.get("multi").is_some());

    let url = format!("ws://127.0.0.1:{port}/api/remote/sync?videoId={id}");
    let (mut phone, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    phone
        .send(Message::text(
            r#"{"type":"state","at":1000,"paused":false,"rate":1}"#,
        ))
        .await
        .unwrap();
    assert_eq!(frame_starting(&mut device, "L0").await, "L03000I17\n");

    // Edge: nothing is sent while it is held.
    let e = post(port, "/api/remote/edge", r#"{"active":true}"#).await;
    assert_eq!(e["active"], true);
    tokio::time::sleep(Duration::from_millis(100)).await;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), device.next()).await {}
    assert!(
        tokio::time::timeout(Duration::from_millis(300), device.next())
            .await
            .is_err()
    );
    post(port, "/api/remote/edge", r#"{"active":false}"#).await;
    // The phone's next state resumes the stream (a frame differing from the last).
    phone
        .send(Message::text(r#"{"type":"seek","at":2000}"#))
        .await
        .unwrap();
    let r = post(
        port,
        "/api/devices/routing",
        &serde_json::json!({"videoId": id, "routes": []}).to_string(),
    )
    .await;
    assert_eq!(r["active"], serde_json::Value::Null);
    let f = frame_starting(&mut device, "L0").await;
    assert_ne!(f, "L03000I17\n", "back on the main script");
}

/// A fake Intiface Central: answers the handshake with a rotator, an
/// oscillating machine, a vibrator and a stroker, and forwards every
/// command it gets.
async fn fake_intiface(
    listener: tokio::net::TcpListener,
    out: tokio::sync::mpsc::UnboundedSender<serde_json::Value>,
) {
    let (sock, _) = listener.accept().await.unwrap();
    let mut ws = tokio_tungstenite::accept_async(sock).await.unwrap();
    let devices = serde_json::json!([
        {"DeviceIndex": 0, "DeviceName": "Rotator", "DeviceMessages": {"RotateCmd": [{"StepCount": 20}]}},
        {"DeviceIndex": 1, "DeviceName": "Machine", "DeviceMessages": {"ScalarCmd": [{"ActuatorType": "Oscillate"}]}},
        {"DeviceIndex": 2, "DeviceName": "Vibe", "DeviceMessages": {"ScalarCmd": [{"ActuatorType": "Vibrate"}]}},
        {"DeviceIndex": 3, "DeviceName": "Stroker", "DeviceMessages": {"LinearCmd": [{"StepCount": 100}]}},
    ]);
    while let Some(Ok(Message::Text(t))) = ws.next().await {
        let list: serde_json::Value = serde_json::from_str(&t).unwrap();
        for m in list.as_array().unwrap() {
            let (kind, body) = m.as_object().unwrap().iter().next().unwrap();
            let id = &body["Id"];
            let reply = match kind.as_str() {
                "RequestServerInfo" => serde_json::json!([{"ServerInfo": {"Id": id,
                    "ServerName": "fake", "MessageVersion": 3, "MaxPingTime": 0}}]),
                "RequestDeviceList" => {
                    serde_json::json!([{"DeviceList": {"Id": id, "Devices": devices}}])
                }
                _ => {
                    let _ = out.send(serde_json::json!({ kind.as_str(): body }));
                    serde_json::json!([{"Ok": {"Id": id}}])
                }
            };
            ws.send(Message::text(reply.to_string())).await.unwrap();
        }
    }
}

#[tokio::test]
async fn buttplug_devices_get_their_own_outputs() {
    let intiface = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bp_port = intiface.local_addr().unwrap().port();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(fake_intiface(intiface, tx));
    let lib = tempfile::tempdir().unwrap();
    std::fs::write(lib.path().join("Scene.mp4"), b"0").unwrap();
    std::fs::write(
        lib.path().join("Scene.funscript"),
        br#"{"actions":[{"at":0,"pos":0},{"at":60000,"pos":100}]}"#,
    )
    .unwrap();
    let (_srv, port, data) = serve(serde_json::json!({"settings": {
        "library": {"sources": [{"id": "1", "name": "Lib", "path": lib.path(), "enabled": true}]},
        "buttplug": {"port": bp_port, "autoConnect": true,
                     "deviceSettings": {"3:Stroker": {"inverted": true, "range": {"min": 0, "max": 50}},
                                        "0:Rotator": {"rotateMode": "position"}}},
    }}));
    let id = loop {
        if let Some(id) = get(port, "/api/videos").await[0]["id"].as_str() {
            break id.to_string();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let devices = loop {
        let d = get(port, "/api/devices").await;
        if d["status"]["buttplug"]["deviceList"]
            .as_array()
            .is_some_and(|l| l.len() == 4)
        {
            break d;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let stroker = &devices["status"]["buttplug"]["deviceList"][3];
    assert_eq!(stroker["key"], "3:Stroker");
    assert_eq!(stroker["features"], serde_json::json!(["Linear"]));

    // Play at 6 s: position 10, rising 100 units per minute.
    let url = format!("ws://127.0.0.1:{port}/api/remote/sync?videoId={id}");
    let (mut phone, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    phone
        .send(Message::text(
            r#"{"type":"state","at":6000,"paused":false,"rate":1}"#,
        ))
        .await
        .unwrap();
    let mut got = std::collections::HashMap::new();
    while got.len() < 4 {
        let m = tokio::time::timeout(Duration::from_secs(20), rx.recv())
            .await
            .expect("not every device got a command")
            .unwrap();
        let (kind, body) = m.as_object().unwrap().iter().next().unwrap();
        // StartScanning and the like name no device.
        let Some(index) = body["DeviceIndex"].as_u64() else {
            continue;
        };
        // Outputs idle (0) until playback starts: keep the first live one.
        let live = body["Rotations"][0]["Speed"].as_f64().unwrap_or(1.0) > 0.0
            && body["Scalars"][0]["Scalar"].as_f64().unwrap_or(1.0) > 0.0;
        if live {
            got.entry(index).or_insert((kind.clone(), body.clone()));
        }
    }
    assert_eq!(got[&0].0, "RotateCmd");
    // Position mode: |10 - 50| * 2 = 80 % clockwise.
    let speed = got[&0].1["Rotations"][0]["Speed"].as_f64().unwrap();
    assert!((speed - 0.8).abs() < 0.02, "{speed}");
    assert_eq!(got[&0].1["Rotations"][0]["Clockwise"], true);
    assert_eq!(got[&1].0, "ScalarCmd");
    assert_eq!(got[&1].1["Scalars"][0]["ActuatorType"], "Oscillate");
    assert_eq!(got[&2].1["Scalars"][0]["ActuatorType"], "Vibrate");
    assert_eq!(got[&3].0, "LinearCmd");
    // Inverted 90, into 0..50 = 45.
    let p = got[&3].1["Vectors"][0]["Position"].as_f64().unwrap();
    assert!((p - 0.45).abs() < 0.01, "{p}");

    // Each device is remembered as the renderer does.
    let lib_file = data.path().join("library.json");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&lib_file).unwrap()).unwrap();
        let known = &saved["config"]["settings"]["knownDevices"];
        if known.as_array().is_some_and(|k| k.len() == 4) {
            assert_eq!(known[3]["id"], "buttplug:Stroker#3");
            assert_eq!(known[3]["buttplugIndex"], 3);
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "{known}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A browser tab attaching to the serial relay, as `serial-relay.js` does.
async fn sink(
    port: u16,
    origin: Option<&str>,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio_tungstenite::tungstenite::Error,
> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut req = format!("ws://127.0.0.1:{port}/api/remote/tcode-relay?role=sink")
        .into_client_request()
        .unwrap();
    if let Some(o) = origin {
        req.headers_mut().insert("Origin", o.parse().unwrap());
    }
    tokio_tungstenite::connect_async(req).await.map(|x| x.0)
}

#[tokio::test]
async fn browser_serial_relay_carries_tcode_to_the_tab() {
    let lib = tempfile::tempdir().unwrap();
    std::fs::write(lib.path().join("Scene.mp4"), b"0").unwrap();
    std::fs::write(
        lib.path().join("Scene.funscript"),
        br#"{"actions":[{"at":0,"pos":0},{"at":60000,"pos":100}]}"#,
    )
    .unwrap();
    let (_srv, port, _data) = serve(serde_json::json!({"settings": {
        "library": {"sources": [{"id": "1", "name": "Lib", "path": lib.path(), "enabled": true}]},
        "tcode": {"transport": "remote-serial", "remoteBaudRate": 250000,
                  "axes": {"L0": {"min": 0, "max": 100}}},
    }}));
    // Pages from elsewhere may not attach.
    assert!(sink(port, None).await.is_err());
    assert!(sink(port, Some("http://evil.example")).await.is_err());

    let origin = format!("http://127.0.0.1:{port}");
    let mut tab = sink(port, Some(&origin)).await.unwrap();
    let Some(Ok(Message::Text(cfg))) = tab.next().await else {
        panic!("no config")
    };
    assert_eq!(cfg.as_str(), r#"{"baudRate":250000,"type":"config"}"#);
    let port_msg = r#"{"type":"port","open":true,"info":{"usbVendorId":6790},"baudRate":250000}"#;
    tab.send(Message::text(port_msg)).await.unwrap();
    status_until(port, |s| {
        s["tcodeRelay"]["sink"]["portOpen"] == true
            && s["devices"]["tcode"]["link"]["state"] == "connected"
    })
    .await;

    let id = loop {
        if let Some(id) = get(port, "/api/videos").await[0]["id"].as_str() {
            break id.to_string();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let url = format!("ws://127.0.0.1:{port}/api/remote/sync?videoId={id}");
    let (mut phone, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    phone
        .send(Message::text(
            r#"{"type":"state","at":30000,"paused":false,"rate":1}"#,
        ))
        .await
        .unwrap();
    let frame = loop {
        let m = tokio::time::timeout(Duration::from_secs(20), tab.next())
            .await
            .expect("no TCode at the tab")
            .unwrap()
            .unwrap();
        let t = m.into_text().unwrap().to_string();
        if t.starts_with("L0") {
            break t;
        }
    };
    assert!(frame.ends_with("I17\n"), "{frame}");

    // A newer tab replaces it.
    let _newer = sink(port, Some(&origin)).await.unwrap();
    let kicked = loop {
        match tokio::time::timeout(Duration::from_secs(20), tab.next())
            .await
            .unwrap()
        {
            Some(Ok(Message::Text(t))) if t.contains("kicked") => break true,
            Some(Ok(_)) => continue,
            _ => break false,
        }
    };
    assert!(kicked);
    let s = status_until(port, |s| s["tcodeRelay"]["sink"]["portOpen"] == false).await;
    assert_eq!(s["tcodeRelay"]["sink"]["attached"], true);
    assert!(s["tcodeRelay"]["forwarded"].as_u64().unwrap() > 0);
}
