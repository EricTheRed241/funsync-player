//! Minimal Buttplug v3 client for Intiface Central (`ws://127.0.0.1:<port>`):
//! handshake, scan, device list; linear devices follow the stroke
//! (`LinearCmd`), vibrators its speed (`ScalarCmd` Vibrate, 400 units/s = full).

use std::sync::Mutex;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;

/// What the engine sends: stroke position 0–100, stroke speed (units/s),
/// move time in ms.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Frame {
    pub pos: f64,
    pub speed: f64,
    pub ms: u32,
}

struct Device {
    index: u64,
    name: String,
    linear: bool,
    vibrators: usize,
}

fn device(d: &Value) -> Device {
    let m = &d["DeviceMessages"];
    Device {
        index: d["DeviceIndex"].as_u64().unwrap_or(0),
        name: d["DeviceName"].as_str().unwrap_or("").to_string(),
        linear: m.get("LinearCmd").is_some(),
        vibrators: m["ScalarCmd"].as_array().map_or(0, |a| {
            a.iter().filter(|x| x["ActuatorType"] == "Vibrate").count()
        }),
    }
}

/// Run against Intiface until the socket fails; `names` mirrors the device list.
pub async fn session(
    port: u64,
    rx: &mut watch::Receiver<Frame>,
    names: &Mutex<Vec<String>>,
    connected: impl FnOnce(),
) -> Result<(), String> {
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}"))
        .await
        .map_err(|e| e.to_string())?;
    let (mut tx, mut inc) = ws.split();
    let mut id = 0u64;
    let mut msg = |kind: &str, mut v: Value| {
        id += 1;
        v["Id"] = json!(id);
        Message::text(json!([{ kind: v }]).to_string())
    };
    for (k, v) in [
        (
            "RequestServerInfo",
            json!({"ClientName": "FunSync", "MessageVersion": 3}),
        ),
        ("StartScanning", json!({})),
        ("RequestDeviceList", json!({})),
    ] {
        tx.send(msg(k, v)).await.map_err(|e| e.to_string())?;
    }
    connected();
    let mut devices: Vec<Device> = Vec::new();
    loop {
        tokio::select! {
            r = rx.changed() => {
                r.map_err(|e| e.to_string())?;
                let f = *rx.borrow_and_update();
                for d in &devices {
                    if d.linear {
                        let v = json!({"DeviceIndex": d.index, "Vectors":
                            [{"Index": 0, "Duration": f.ms, "Position": f.pos / 100.0}]});
                        tx.send(msg("LinearCmd", v)).await.map_err(|e| e.to_string())?;
                    }
                    if d.vibrators > 0 {
                        let s = (f.speed / 400.0).clamp(0.0, 1.0);
                        let scalars: Vec<Value> = (0..d.vibrators)
                            .map(|i| json!({"Index": i, "Scalar": s, "ActuatorType": "Vibrate"}))
                            .collect();
                        let v = json!({"DeviceIndex": d.index, "Scalars": scalars});
                        tx.send(msg("ScalarCmd", v)).await.map_err(|e| e.to_string())?;
                    }
                }
            }
            m = inc.next() => {
                let text = match m {
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(_)) => continue,
                    _ => return Err("Intiface closed the connection".into()),
                };
                let Ok(Value::Array(list)) = serde_json::from_str::<Value>(&text) else { continue };
                for m in list {
                    if let Some(l) = m["DeviceList"]["Devices"].as_array() {
                        devices = l.iter().map(device).collect();
                    } else if m.get("DeviceAdded").is_some() {
                        devices.push(device(&m["DeviceAdded"]));
                    } else if let Some(i) = m["DeviceRemoved"]["DeviceIndex"].as_u64() {
                        devices.retain(|d| d.index != i);
                    }
                }
                *names.lock().unwrap() = devices.iter().map(|d| d.name.clone()).collect();
            }
        }
    }
}
