//! Diagnostics: the old backend's `/api/status` device part (`issues`,
//! `devices`, `session`, `phone`, `backend`, `desktop`) and the device
//! dashboard (`/device`, `backend/device-dashboard/` served unchanged, and
//! `GET /api/device/telemetry`), both fed by the server's own engine. The
//! dashboard's "desktop app" is this server: it is always connected.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Json, Redirect, Response};
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Play, Status, profiles};
use crate::data::now_ms;
use crate::server::AppState;

const PAGE_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../backend/device-dashboard"
);
/// 15 minutes of 1 s samples, ~100 s of commands, 500 events (as the old backend).
const SAMPLES: usize = 900;
const COMMANDS_MS: u64 = 100_000;
const EVENTS: usize = 500;

/// ISO 8601 UTC, seconds (days → civil date, H. Hinnant's algorithm).
pub fn iso(ms: u64) -> String {
    let s = ms / 1000;
    let z = (s / 86400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let t = s % 86400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}+00:00",
        t / 3600,
        t / 60 % 60,
        t % 60
    )
}

#[derive(Default)]
pub struct Telem {
    samples: VecDeque<(u64, u64, Value)>,
    commands: VecDeque<(u64, String)>,
    events: VecDeque<Value>,
    seq: u64,
    event_seq: u64,
}

impl Telem {
    pub fn sample(&mut self, s: Value) {
        self.seq += 1;
        self.samples.push_back((self.seq, now_ms(), s));
        if self.samples.len() > SAMPLES {
            self.samples.pop_front();
        }
    }

    pub fn command(&mut self, cmd: &str) {
        let now = now_ms();
        self.commands.push_back((now, cmd.trim_end().to_string()));
        while self
            .commands
            .front()
            .is_some_and(|c| c.0 + COMMANDS_MS < now)
        {
            self.commands.pop_front();
        }
    }

    pub fn event(&mut self, kind: &str, level: &str, text: String) {
        self.event_seq += 1;
        self.events
            .push_back(json!({"seq": self.event_seq, "at": iso(now_ms()),
            "kind": kind, "level": level, "text": text}));
        if self.events.len() > EVENTS {
            self.events.pop_front();
        }
    }
}

/// Where the TCode device is: its URL, port or host.
pub fn address(tc: &Value) -> String {
    let s = |k: &str| tc[k].as_str().unwrap_or("").to_string();
    match tc["transport"].as_str().unwrap_or("") {
        "websocket" => s("wsUrl"),
        "udp" => format!("{}:{}", s("udpHost"), tc["udpPort"]),
        _ => s("port"),
    }
}

/// The device pill's name, e.g. `FUNSR MINI · TCode (/dev/cu.usbserial-1)`.
pub fn label(tc: &Value) -> String {
    let name = profiles::PROFILES
        .iter()
        .find(|p| p.0 == profiles::active(tc))
        .map_or("", |p| p.1);
    let t = format!("TCode ({})", address(tc));
    if name.is_empty() {
        t
    } else {
        format!("{name} · {t}")
    }
}

pub fn link(s: &Status, tc: &Value) -> Value {
    let state = match (s.running, s.connected, s.attempts, s.ever) {
        (false, ..) => "stopped",
        (_, true, ..) => "connected",
        (_, _, 0, _) => "connecting",
        (_, _, _, true) => "reconnecting",
        _ => "waiting",
    };
    let since = s.since.map(iso);
    json!({
        "state": state,
        "transport": tc["transport"],
        "url": address(tc),
        "connectedSince": if s.connected { since.clone() } else { None },
        "downSince": if s.connected { None } else { since },
        "attempts": s.attempts,
        "retryEveryMs": s.retry_ms,
        "nextRetryAt": s.next_retry.map(iso),
        "lastError": s.error.as_ref().map(|(t, at)| json!({"text": t, "at": iso(*at)})),
        "sent": {"commands": s.commands, "bytes": s.bytes, "errors": 0,
                 "lastSendAgoMs": s.last_send.map(|t| now_ms().saturating_sub(t))},
    })
}

/// One dashboard sample (the shape `renderer/js/device-telemetry.js` sent).
pub fn sample(st: &AppState, play: Option<&Play>, s: &Value) -> Value {
    let tc = &s["tcode"];
    let d = &st.devices;
    let t = play.map(Play::time);
    let offset = tc["defaultOffset"].as_f64().unwrap_or(0.0);
    let main = play.map(|p| &p.axes[0].1);
    let next = main.and_then(|a| {
        let t = t? + offset;
        a.iter().find(|x| x.at > t).map(|x| (x.at - t).round())
    });
    let axes: Vec<Value> = play.map_or(vec![], |p| {
        p.axes
            .iter()
            .map(|(id, acts)| {
                json!({"axis": id, "kind": "script", "actions": acts.len(),
                "enabled": tc["axes"][id]["enabled"] != false})
            })
            .collect()
    });
    let active = d.connected("tcode") || d.connected("buttplug");
    json!({
        "source": play.map(|_| "phone"),
        "script": play.map(|p| json!({"name": p.name, "actions": p.axes[0].1.len(),
            "durationMs": p.axes[0].1.last().map(|a| a.at)})),
        "sync": {
            "active": active, "running": active && play.is_some_and(|p| !p.paused),
            "paused": play.map(|p| p.paused), "mediaTimeMs": t.map(f64::round),
            "nextKeyframeInMs": next, "maxGapMs": 3_600_000, "tickHz": 60,
            "offsetMs": offset, "axes": axes,
        },
        "tcode": {"precision": tc["precision"].as_u64().unwrap_or(4),
                  "link": link(&d.status("tcode").lock().unwrap(), tc)},
    })
}

/// The device part of `GET /api/status`.
pub async fn status(st: &AppState) -> Value {
    let s = super::device_settings(st).await;
    let tc = &s["tcode"];
    let d = &st.devices;
    let link = link(&d.status("tcode").lock().unwrap(), tc);
    let mut issues = Vec::new();
    if let Some(doing) = match link["state"].as_str() {
        Some("waiting") => Some("not answering yet"),
        Some("reconnecting") => Some("connection lost, reconnecting"),
        Some("connecting") => Some("connecting"),
        _ => None,
    } {
        let mut t = format!(
            "TCode device {}: {doing}",
            link["url"].as_str().unwrap_or("")
        );
        if let Some(since) = link["downSince"].as_str() {
            t += &format!(" since {since}");
        }
        t += &format!(" ({} attempts)", link["attempts"]);
        if let Some(e) = link["lastError"]["text"].as_str() {
            t += &format!(". Last failure: {e}");
        }
        issues.push(t + ".");
    }
    if !d.connected("tcode") && !d.connected("buttplug") {
        issues.push("No device is connected.".to_string());
    }
    let play = d.play.lock().unwrap().clone();
    let phone = d.phone.lock().unwrap().clone();
    let now = now_ms();
    json!({
        "issues": issues,
        "devices": {
            "tcode": {"connected": d.connected("tcode"), "profile": profiles::active(tc),
                      "label": label(tc), "link": link},
            "buttplug": {"connected": d.connected("buttplug"),
                         "devices": *d.buttplug_names.lock().unwrap()},
        },
        "session": play.as_ref().map(|p| json!({"source": "web remote",
            "state": if p.paused { "paused" } else { "playing" },
            "videoId": p.video_id, "video": p.name, "positionMs": p.time().round()})),
        "phone": {
            "connected": phone.is_some(),
            "ip": phone.as_ref().map(|p| &p.0), "videoId": phone.as_ref().map(|p| &p.1),
            "video": play.as_ref().map(|p| &p.name),
            "lastState": play.as_ref().map(|p| json!({"at": p.at.round(), "paused": p.paused})),
        },
        "backend": {"pid": std::process::id(), "startedAt": iso(st.started_at),
                    "uptimeS": (now - st.started_at) / 1000,
                    "videos": st.snapshot.read().await.videos.len()},
        "desktop": {"connected": true, "connectedSince": iso(st.started_at),
                    "statusAt": iso(now), "statusAgeS": 0},
    })
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/device", get(|| async { Redirect::to("/device/") }))
        .route("/device/", get(|c| page(c, Path(String::new()))))
        .route("/device/{name}", get(page))
        .route("/api/device/telemetry", get(telemetry))
}

fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        "The device dashboard is only available on this computer.",
    )
        .into_response()
}

async fn page(ConnectInfo(a): ConnectInfo<SocketAddr>, Path(name): Path<String>) -> Response {
    if !a.ip().is_loopback() {
        return forbidden();
    }
    let (file, mime) = match name.as_str() {
        "" => ("index.html", "text/html; charset=utf-8"),
        "dashboard.js" => ("dashboard.js", "text/javascript"),
        "dashboard.css" => ("dashboard.css", "text/css"),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    match tokio::fs::read(format!("{PAGE_DIR}/{file}")).await {
        Ok(b) => (
            [
                (header::CONTENT_TYPE, mime),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            b,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cursors {
    #[serde(default)]
    since: u64,
    #[serde(default)]
    cmd_since: u64,
    #[serde(default)]
    event_since: u64,
}

async fn telemetry(
    ConnectInfo(a): ConnectInfo<SocketAddr>,
    Query(c): Query<Cursors>,
    State(st): State<Arc<AppState>>,
) -> Response {
    if !a.ip().is_loopback() {
        return forbidden();
    }
    let now = now_ms();
    let mut body = {
        let t = st.devices.telem.lock().unwrap();
        let latest = t.samples.back();
        let cmd_since = if c.cmd_since > 0 {
            c.cmd_since
        } else {
            now - 30_000
        };
        json!({
            "now": now,
            "streaming": latest.is_some_and(|l| now - l.1 <= 3000),
            "latest": latest.map(|l| &l.2),
            "latestAgeS": latest.map(|l| (now - l.1) as f64 / 1000.0),
            "seq": latest.map_or(0, |l| l.0),
            "points": t.samples.iter().filter(|s| s.0 > c.since).map(|(seq, at, s)| json!({
                "seq": seq, "t": at, "linkState": s["tcode"]["link"]["state"],
                "sent": s["tcode"]["link"]["sent"]["commands"],
                "mediaMs": s["sync"]["mediaTimeMs"], "paused": s["sync"]["paused"],
            })).collect::<Vec<_>>(),
            "commands": t.commands.iter().filter(|x| x.0 > cmd_since)
                .map(|(t, cmd)| json!({"t": t, "cmd": cmd})).collect::<Vec<_>>(),
            "events": t.events.iter().filter(|e| e["seq"].as_u64() > Some(c.event_since))
                .collect::<Vec<_>>(),
        })
    };
    body["status"] = status(&st).await;
    Json(body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates() {
        assert_eq!(iso(0), "1970-01-01T00:00:00+00:00");
        assert_eq!(iso(1_795_000_000_000), "2026-11-18T11:06:40+00:00");
        assert_eq!(iso(951_782_400_000), "2000-02-29T00:00:00+00:00");
    }

    #[test]
    fn labels_name_the_profile() {
        let tc = json!({"transport": "websocket", "wsUrl": "ws://mirabot.local:8888"});
        assert_eq!(label(&tc), "MiraBot · TCode (ws://mirabot.local:8888)");
        let tc = json!({"transport": "udp", "udpHost": "h", "udpPort": 9});
        assert_eq!(label(&tc), "TCode (h:9)");
    }
}
