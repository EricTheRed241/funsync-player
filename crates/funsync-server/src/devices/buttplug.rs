//! Buttplug v3 client for Intiface Central (`ws://127.0.0.1:<port>`) with
//! a per-device output (ported from the old Electron app):
//! strokers follow the position (`LinearCmd`); vibrators, oscillating
//! machines, constrict / inflate devices (`ScalarCmd`) and rotators
//! (`RotateCmd`) get an intensity from the position or the speed per their
//! mode. Per device (`settings.buttplug.deviceSettings["<index>:<name>"]`):
//! `inverted`, `range`, `cutoff`, `vibeMode` / `scalarMode` /
//! `oscillateMode` / `rotateMode` (`speed`, `position`, `intensity`),
//! `maxIntensity` (default 70) and `rampUp` (2 s, default on) for the
//! devices that show those controls (scalar, oscillate).

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;

/// The stroke at one moment: position 0–100 and signed speed (units/s).
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Motion {
    pub pos: f64,
    pub vel: f64,
}

/// What the engine sends. `routed` (custom routing) gives `(device index,
/// motion)` per routed device and the others get nothing; `vib` is a
/// dedicated vibration script's value (`buttplugVib`); `stopped` idles
/// every output but the strokers'.
#[derive(Clone, Default, PartialEq)]
pub struct Frame {
    pub main: Motion,
    pub routed: Option<Vec<(u64, Motion)>>,
    pub vib: Option<f64>,
    pub stopped: bool,
    /// Ramp-up factor 0–1 since playback started.
    pub ramp: f64,
    pub ms: u32,
    pub settings: Arc<Value>,
}

impl Frame {
    /// Idle the device outputs; true when it changed.
    pub fn still(&mut self) -> bool {
        !std::mem::replace(&mut self.stopped, true)
    }
}

/// A device as Intiface lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub index: u64,
    pub name: String,
    pub linear: bool,
    /// `(actuator index, type)` of its `ScalarCmd` actuators.
    pub scalars: Vec<(usize, String)>,
    pub rotators: usize,
}

impl Device {
    fn has(&self, kind: &str) -> bool {
        self.scalars.iter().any(|s| s.1 == kind)
    }

    /// Feature names for the Devices page.
    pub fn features(&self) -> Vec<&str> {
        let mut f: Vec<&str> = self.scalars.iter().map(|s| s.1.as_str()).collect();
        if self.linear {
            f.push("Linear");
        }
        if self.rotators > 0 {
            f.push("Rotate");
        }
        f.dedup();
        f
    }
}

fn device(d: &Value) -> Device {
    let m = &d["DeviceMessages"];
    let count = |v: &Value| v.as_array().map_or(usize::from(v.is_object()), Vec::len);
    Device {
        index: d["DeviceIndex"].as_u64().unwrap_or(0),
        name: d["DeviceName"].as_str().unwrap_or("").to_string(),
        linear: m.get("LinearCmd").is_some(),
        scalars: m["ScalarCmd"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, a)| (i, a["ActuatorType"].as_str().unwrap_or("").to_string()))
            .collect(),
        rotators: count(&m["RotateCmd"]),
    }
}

/// `inverted`, then `range`, then `cutoff` (`device-transform-stack.js`).
fn transform(pos: f64, s: &Value) -> f64 {
    let p = if s["inverted"] == true {
        100.0 - pos
    } else {
        pos
    };
    let bounds = |v: &Value| {
        let (lo, hi) = (
            v["min"].as_f64()?.clamp(0.0, 100.0),
            v["max"].as_f64()?.clamp(0.0, 100.0),
        );
        (lo < hi).then_some((lo, hi))
    };
    let p = match bounds(&s["range"]) {
        Some((lo, hi)) => lo + (hi - lo) * p.clamp(0.0, 100.0) / 100.0,
        None => p,
    };
    match bounds(&s["cutoff"]) {
        Some((lo, hi)) => p.clamp(lo, hi),
        None => p,
    }
}

/// Intensity 0–100 from the position or the speed (300 units/s = full).
fn intensity(mode: &str, pos: f64, speed: f64) -> f64 {
    let by_speed = (speed / 300.0 * 100.0).min(100.0);
    match mode {
        "position" => pos.clamp(0.0, 100.0),
        "intensity" => (pos * 0.4 + by_speed * 0.6).min(100.0),
        _ => by_speed,
    }
}

/// The device's commands for one frame, `(message type, body)`.
pub fn commands(d: &Device, f: &Frame) -> Vec<(&'static str, Value)> {
    let m = match &f.routed {
        None => Some(f.main),
        Some(r) => r.iter().find(|x| x.0 == d.index).map(|x| x.1),
    };
    let Some(m) = m else { return Vec::new() };
    let s = &f.settings[format!("{}:{}", d.index, d.name)];
    let pos = transform(m.pos, s);
    let scale = (transform(100.0, s) - transform(0.0, s)).abs() / 100.0;
    let speed = m.vel.abs() * scale;
    let mode = |k: &str, default: &str| s[k].as_str().unwrap_or(default).to_string();
    let controls = d.has("Oscillate") || d.has("Constrict") || d.has("Inflate");
    let safe = |v: f64, always: bool| {
        if !(always || controls) {
            return v;
        }
        let cap = s["maxIntensity"].as_f64().unwrap_or(70.0);
        let ramp = if s["rampUp"] == false { 1.0 } else { f.ramp };
        (v.min(cap) * ramp).clamp(0.0, 100.0)
    };
    let mut out = Vec::new();
    if d.linear && !f.stopped {
        out.push((
            "LinearCmd",
            json!({"DeviceIndex": d.index, "Vectors":
            [{"Index": 0, "Duration": f.ms, "Position": pos / 100.0}]}),
        ));
    }
    let scalars: Vec<Value> = d
        .scalars
        .iter()
        .filter_map(|(i, kind)| {
            let v = match kind.as_str() {
                // Oscillate wins on hardware with both: one motor.
                "Vibrate" if d.has("Oscillate") => return None,
                "Vibrate" => match f.vib {
                    Some(v) => v,
                    None => safe(intensity(&mode("vibeMode", "speed"), pos, speed), false),
                },
                "Oscillate" => safe(intensity(&mode("oscillateMode", "speed"), pos, speed), true),
                "Constrict" | "Inflate" => {
                    safe(intensity(&mode("scalarMode", "position"), pos, speed), true)
                }
                _ => return None,
            };
            let v = if f.stopped { 0.0 } else { v };
            Some(json!({"Index": i, "Scalar": (v / 100.0 * 1000.0).round() / 1000.0, "ActuatorType": kind}))
        })
        .collect();
    if !scalars.is_empty() {
        out.push((
            "ScalarCmd",
            json!({"DeviceIndex": d.index, "Scalars": scalars}),
        ));
    }
    if d.rotators > 0 {
        let (v, clockwise) = if mode("rotateMode", "speed") == "position" {
            (safe((pos - 50.0).abs() * 2.0, false), pos < 50.0)
        } else {
            (
                safe(intensity(&mode("rotateMode", "speed"), pos, speed), false),
                m.vel >= 0.0,
            )
        };
        let v = if f.stopped {
            0.0
        } else {
            (v / 100.0 * 1000.0).round() / 1000.0
        };
        let rotations: Vec<Value> = (0..d.rotators)
            .map(|i| json!({"Index": i, "Speed": v, "Clockwise": clockwise}))
            .collect();
        out.push((
            "RotateCmd",
            json!({"DeviceIndex": d.index, "Rotations": rotations}),
        ));
    }
    out
}

/// Run against Intiface until the socket fails; `seen` gets the device
/// list whenever it changes.
pub async fn session(
    port: u64,
    rx: &mut watch::Receiver<Frame>,
    seen: impl Fn(&[Device]),
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
    // The last body sent per device and message type: only changes go out.
    let mut last: HashMap<(u64, &str), Value> = HashMap::new();
    loop {
        tokio::select! {
            r = rx.changed() => {
                r.map_err(|e| e.to_string())?;
                let f = rx.borrow_and_update().clone();
                for d in &devices {
                    for (kind, body) in commands(d, &f) {
                        if last.get(&(d.index, kind)) == Some(&body) {
                            continue;
                        }
                        last.insert((d.index, kind), body.clone());
                        tx.send(msg(kind, body)).await.map_err(|e| e.to_string())?;
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
                    } else {
                        continue;
                    }
                    seen(&devices);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(v: Value) -> Device {
        device(&json!({"DeviceIndex": 1, "DeviceName": "D", "DeviceMessages": v}))
    }

    fn frame(pos: f64, vel: f64, settings: Value) -> Frame {
        Frame {
            main: Motion { pos, vel },
            ramp: 1.0,
            ms: 17,
            settings: Arc::new(json!({ "1:D": settings })),
            ..Frame::default()
        }
    }

    #[test]
    fn modes_ranges_and_safety() {
        let stroker = dev(json!({"LinearCmd": [{}]}));
        let f = frame(
            25.0,
            0.0,
            json!({"inverted": true, "range": {"min": 20, "max": 60}}),
        );
        // 100 - 25 = 75, into 20..60 = 50.
        assert_eq!(commands(&stroker, &f)[0].1["Vectors"][0]["Position"], 0.5);

        let vibe = dev(json!({"ScalarCmd": [{"ActuatorType": "Vibrate"}]}));
        let c = commands(&vibe, &frame(80.0, 150.0, json!({})));
        assert_eq!(c[0].1["Scalars"][0]["Scalar"], 0.5, "speed mode: 150/300");
        let c = commands(&vibe, &frame(80.0, 150.0, json!({"vibeMode": "position"})));
        assert_eq!(c[0].1["Scalars"][0]["Scalar"], 0.8);

        // Oscillate: capped at 70 % by default, ramped; it silences Vibrate.
        let machine =
            dev(json!({"ScalarCmd": [{"ActuatorType": "Vibrate"}, {"ActuatorType": "Oscillate"}]}));
        let mut f = frame(0.0, 600.0, json!({}));
        f.ramp = 0.5;
        let c = commands(&machine, &f);
        assert_eq!(
            c[0].1["Scalars"],
            json!([{"Index": 1, "Scalar": 0.35, "ActuatorType": "Oscillate"}])
        );

        let rot = dev(json!({"RotateCmd": [{"StepCount": 20}]}));
        let c = commands(
            &rot,
            &frame(10.0, -300.0, json!({"rotateMode": "position"})),
        );
        assert_eq!(
            c[0].1["Rotations"][0],
            json!({"Index": 0, "Speed": 0.8, "Clockwise": true})
        );
        let c = commands(&rot, &frame(10.0, -300.0, json!({})));
        assert_eq!(
            c[0].1["Rotations"][0],
            json!({"Index": 0, "Speed": 1.0, "Clockwise": false})
        );

        // Stopped: strokers hold, everything else goes to 0.
        let mut f = frame(10.0, 300.0, json!({}));
        f.stopped = true;
        assert!(commands(&stroker, &f).is_empty());
        assert_eq!(commands(&vibe, &f)[0].1["Scalars"][0]["Scalar"], 0.0);
    }
}
