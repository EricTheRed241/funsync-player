//! TCode: frames (`L09999I17 R05000I17`) from `settings.tcode` and the
//! transports (WebSocket client, UDP, USB serial via a raw tty).

use std::collections::HashMap;

use futures_util::SinkExt;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

use super::interp::{self, Action};

/// One axis command: value 0–100 over `precision` digits, `I<ms>` move time.
pub fn command(axis: &str, value: f64, precision: usize, ms: u32) -> String {
    let max = 10f64.powi(precision as i32) - 1.0;
    let v = (value.clamp(0.0, 100.0) / 100.0 * max).round() as u32;
    format!("{axis}{v:0precision$}I{ms}")
}

/// The frame for media time `t`: every enabled axis, smoothed, speed
/// limited against `last`, inverted and mapped into the axis' min..max.
pub fn frame(
    axes: &[(String, Vec<Action>)],
    t: f64,
    tcode: &Value,
    player: &Value,
    dt_ms: f64,
    last: &mut HashMap<String, f64>,
) -> String {
    let precision = match tcode["precision"].as_u64() {
        Some(3) => 3,
        _ => 4,
    };
    let mode = player["smoothing"].as_str().unwrap_or("linear");
    let limit = player["speedLimit"].as_f64().unwrap_or(0.0);
    let mut out = Vec::new();
    for (id, acts) in axes {
        let cfg = &tcode["axes"][id];
        if cfg["enabled"] == false {
            continue;
        }
        let Some(mut p) = interp::position(mode, acts, t) else {
            continue;
        };
        if let Some(&prev) = last.get(id) {
            p = interp::speed_limit(p, prev, dt_ms, limit);
        }
        last.insert(id.clone(), p);
        if cfg["inverted"] == true {
            p = 100.0 - p;
        }
        let (min, max) = (
            cfg["min"].as_f64().unwrap_or(0.0),
            cfg["max"].as_f64().unwrap_or(100.0),
        );
        out.push(command(
            id,
            min + p / 100.0 * (max - min),
            precision,
            dt_ms.round() as u32,
        ));
    }
    if out.is_empty() {
        String::new()
    } else {
        out.join(" ") + "\n"
    }
}

/// Connect per `settings.tcode` and write every frame until a write fails.
pub async fn session(
    cfg: &Value,
    rx: &mut watch::Receiver<String>,
    connected: impl FnOnce(),
) -> Result<(), String> {
    let s = |k: &str| cfg[k].as_str().unwrap_or("").to_string();
    let e = |e: std::io::Error| e.to_string();
    match cfg["transport"].as_str().unwrap_or("") {
        "websocket" => {
            let url = s("wsUrl");
            let (mut ws, _) = tokio_tungstenite::connect_async(url.as_str())
                .await
                .map_err(|e| e.to_string())?;
            connected();
            loop {
                rx.changed().await.map_err(|e| e.to_string())?;
                let f = rx.borrow_and_update().clone();
                ws.send(f.into()).await.map_err(|e| e.to_string())?;
            }
        }
        "udp" => {
            let sock = tokio::net::UdpSocket::bind("0.0.0.0:0").await.map_err(e)?;
            let port = cfg["udpPort"].as_u64().unwrap_or(0);
            sock.connect(format!("{}:{port}", s("udpHost")))
                .await
                .map_err(e)?;
            connected();
            loop {
                rx.changed().await.map_err(|e| e.to_string())?;
                let f = rx.borrow_and_update().clone();
                sock.send(f.as_bytes()).await.map_err(e)?;
            }
        }
        "serial" => {
            let port = s("port");
            let baud = cfg["baudRate"].as_u64().unwrap_or(115200).to_string();
            let flag = if cfg!(target_os = "macos") {
                "-f"
            } else {
                "-F"
            };
            let ok = tokio::process::Command::new("stty")
                .args([flag, &port, &baud, "raw", "-echo"])
                .status()
                .await
                .map_err(e)?;
            if !ok.success() {
                return Err(format!("stty {port} failed"));
            }
            let mut f = tokio::fs::OpenOptions::new()
                .write(true)
                .open(&port)
                .await
                .map_err(e)?;
            connected();
            loop {
                rx.changed().await.map_err(|e| e.to_string())?;
                let fr = rx.borrow_and_update().clone();
                f.write_all(fr.as_bytes()).await.map_err(e)?;
            }
        }
        t => Err(format!("unsupported transport {t:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn commands_have_fixed_digits() {
        assert_eq!(command("L0", 100.0, 4, 16), "L09999I16");
        assert_eq!(command("R0", 0.0, 4, 16), "R00000I16");
        assert_eq!(command("R1", 50.0, 4, 17), "R15000I17");
        assert_eq!(command("L0", 150.0, 3, 5), "L0999I5");
    }

    #[test]
    fn frame_maps_axes_into_min_max_and_skips_disabled() {
        let a = vec![
            Action { at: 0.0, pos: 0.0 },
            Action {
                at: 100.0,
                pos: 100.0,
            },
        ];
        let axes = vec![
            ("L0".to_string(), a.clone()),
            ("R0".to_string(), a.clone()),
            ("R1".to_string(), a),
        ];
        let tc = json!({"axes": {
            "L0": {"min": 10, "max": 90},
            "R0": {"enabled": false},
            "R1": {"inverted": true}}});
        let mut last = HashMap::new();
        let f = frame(&axes, 100.0, &tc, &json!({}), 17.0, &mut last);
        assert_eq!(f, "L08999I17 R10000I17\n");
        // Speed limit 400/s over 17 ms: at most 6.8 units from the last.
        let f = frame(
            &axes,
            0.0,
            &tc,
            &json!({"speedLimit": 400}),
            17.0,
            &mut last,
        );
        assert_eq!(f, "L08455I17 R10680I17\n");
    }
}
