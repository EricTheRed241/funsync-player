//! TCode browser serial relay (`backend/routes/tcode_relay.py`): a USB
//! serial TCode device on another computer is driven through the web remote
//! open there. Its tab (`backend/web-remote/serial-relay.js`, unchanged)
//! opens the device with Web Serial and attaches to
//! `/api/remote/tcode-relay?role=sink`; the `remote-serial` TCode transport
//! sends the engine's frames to it. Only from loopback / private-network
//! addresses and only pages this server served (`Origin` host = `Host`).
//! One tab at a time: a newer one replaces the older (`kicked`, close 4000).
//! No `role=desktop`: this server is the desktop.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

use super::dashboard::iso;
use crate::data::now_ms;
use crate::server::AppState;

/// Frames waiting for one tab; older ones are dropped beyond this.
const QUEUE_MAX: usize = 64;

/// The attached tab.
pub struct Sink {
    id: u64,
    ip: String,
    tx: mpsc::Sender<Out>,
    port_open: bool,
    port: Value,
    baud_rate: Value,
    since: u64,
}

enum Out {
    Text(String),
    Kick,
}

#[derive(Default)]
pub struct Relay {
    sink: Option<Sink>,
    next: u64,
    forwarded: u64,
    dropped: u64,
}

/// `/api/status` `tcodeRelay` (the old backend's `relay_state`, no desktop).
pub fn json(st: &AppState) -> Value {
    let r = st.devices.relay.lock().unwrap();
    let s = r.sink.as_ref();
    json!({
        "sink": {"attached": s.is_some(), "ip": s.map(|s| &s.ip),
                 "portOpen": s.is_some_and(|s| s.port_open), "port": s.map(|s| &s.port),
                 "baudRate": s.map(|s| &s.baud_rate), "since": s.map(|s| iso(s.since))},
        "forwarded": r.forwarded, "dropped": r.dropped,
    })
}

/// Loopback, RFC 1918, link-local or IPv6 unique-local.
fn private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_loopback() || v.is_private() || v.is_link_local(),
        IpAddr::V6(v) => match v.to_ipv4_mapped() {
            Some(v4) => private(IpAddr::V4(v4)),
            None => {
                let s = v.segments()[0];
                v.is_loopback() || s & 0xfe00 == 0xfc00 || s & 0xffc0 == 0xfe80
            }
        },
    }
}

/// A page this server served: the `Origin`'s host:port is the `Host`.
fn same_origin(h: &HeaderMap) -> bool {
    let get = |k| h.get(k).and_then(|v| v.to_str().ok());
    let (Some(origin), Some(host)) = (get(header::ORIGIN), get(header::HOST)) else {
        return false;
    };
    let netloc = origin.split_once("://").map_or("", |x| x.1);
    netloc
        .split('/')
        .next()
        .unwrap_or("")
        .eq_ignore_ascii_case(host)
}

#[derive(Deserialize)]
pub struct RoleQuery {
    role: Option<String>,
}

pub async fn endpoint(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(q): Query<RoleQuery>,
    headers: HeaderMap,
    State(st): State<Arc<AppState>>,
) -> Response {
    if q.role.as_deref() != Some("sink") || !private(addr.ip()) || !same_origin(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.on_upgrade(move |sock| sink_session(st, addr.ip().to_string(), sock))
}

/// The baud rate the tab opens the device with.
fn config(tc: &Value) -> String {
    let baud = tc["remoteBaudRate"]
        .as_u64()
        .or(tc["baudRate"].as_u64())
        .unwrap_or(115200);
    json!({"type": "config", "baudRate": baud}).to_string()
}

async fn sink_session(st: Arc<AppState>, ip: String, mut sock: WebSocket) {
    let (tx, mut rx) = mpsc::channel(QUEUE_MAX);
    let id = {
        let mut r = st.devices.relay.lock().unwrap();
        r.next += 1;
        if let Some(old) = r.sink.take() {
            let _ = old.tx.try_send(Out::Kick);
        }
        r.sink = Some(Sink {
            id: r.next,
            ip: ip.clone(),
            tx,
            port_open: false,
            port: Value::Null,
            baud_rate: Value::Null,
            since: now_ms(),
        });
        r.next
    };
    st.devices.event(
        "relay",
        "ok",
        format!("browser serial relay attached from {ip}"),
    );
    let tc = super::settings(&st, "tcode").await;
    let _ = sock.send(Message::text(config(&tc))).await;
    loop {
        tokio::select! {
            out = rx.recv() => match out {
                Some(Out::Text(t)) => {
                    if sock.send(Message::text(t)).await.is_err() {
                        break;
                    }
                }
                Some(Out::Kick) | None => {
                    let _ = sock.send(Message::text(json!({"type": "kicked"}).to_string())).await;
                    let _ = sock
                        .send(Message::Close(Some(CloseFrame { code: 4000, reason: "replaced".into() })))
                        .await;
                    return;
                }
            },
            m = sock.recv() => {
                let Some(Ok(m)) = m else { break };
                let Message::Text(t) = m else { continue };
                let Ok(msg) = serde_json::from_str::<Value>(&t) else { continue };
                if msg["type"] == "port" {
                    let mut r = st.devices.relay.lock().unwrap();
                    if let Some(s) = r.sink.as_mut().filter(|s| s.id == id) {
                        s.port_open = msg["open"] == true;
                        s.port = msg["info"].clone();
                        s.baud_rate = msg["baudRate"].clone();
                        s.since = now_ms();
                    }
                }
            }
        }
    }
    let mut r = st.devices.relay.lock().unwrap();
    if r.sink.as_ref().is_some_and(|s| s.id == id) {
        r.sink = None;
        drop(r);
        st.devices.event(
            "relay",
            "warn",
            format!("browser serial relay from {ip} detached"),
        );
    }
}

/// The `remote-serial` TCode transport: waits for a tab with the device
/// open, then hands it every frame until it goes away.
pub async fn session(
    st: &AppState,
    rx: &mut watch::Receiver<String>,
    connected: impl FnOnce(),
) -> Result<(), String> {
    let id = {
        let r = st.devices.relay.lock().unwrap();
        match r.sink.as_ref() {
            None => return Err("no browser is attached (open the web remote on the computer with the device and connect it there)".into()),
            Some(s) if !s.port_open => return Err(format!("the browser at {} hasn't opened the device's serial port", s.ip)),
            Some(s) => s.id,
        }
    };
    connected();
    let mut check = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        tokio::select! {
            r = rx.changed() => {
                r.map_err(|e| e.to_string())?;
                let frame = rx.borrow_and_update().clone();
                let mut r = st.devices.relay.lock().unwrap();
                let Some(s) = r.sink.as_ref().filter(|s| s.id == id && s.port_open) else {
                    return Err("the browser serial relay went away".into());
                };
                if s.tx.try_send(Out::Text(frame)).is_ok() {
                    r.forwarded += 1;
                } else {
                    r.dropped += 1;
                }
            }
            _ = check.tick() => {
                let r = st.devices.relay.lock().unwrap();
                if !r.sink.as_ref().is_some_and(|s| s.id == id && s.port_open) {
                    return Err("the browser serial relay went away".into());
                }
            }
        }
    }
}

/// Tell the attached tab the baud rate again (after a settings change).
pub fn reconfigure(st: &AppState, tc: &Value) {
    if let Some(s) = st.devices.relay.lock().unwrap().sink.as_ref() {
        let _ = s.tx.try_send(Out::Text(config(tc)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sinks_only_from_private_networks_and_own_pages() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.2.9",
            "172.20.0.1",
            "fd00::1",
            "::ffff:192.168.1.2",
        ] {
            assert!(private(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "172.32.0.1", "2001:db8::1"] {
            assert!(!private(ip.parse().unwrap()), "{ip}");
        }
        let h = |origin: &str| {
            let mut h = HeaderMap::new();
            h.insert(header::HOST, "192.168.2.9:5300".parse().unwrap());
            h.insert(header::ORIGIN, origin.parse().unwrap());
            h
        };
        assert!(same_origin(&h("http://192.168.2.9:5300")));
        assert!(!same_origin(&h("http://evil.example")));
        assert!(!same_origin(&HeaderMap::new()));
    }
}
