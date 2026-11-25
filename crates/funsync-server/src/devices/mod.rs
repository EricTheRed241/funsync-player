//! Device output, driven by the server: the web remote streams its playback
//! state over `/api/remote/sync` (the phone side of
//! `backend/web-remote/remote-sync.js`), a 60 Hz engine plays the video's
//! scripts from it, and sends TCode (WebSocket, UDP, USB serial) and
//! Buttplug (Intiface) commands. Settings: `settings.tcode`,
//! `settings.buttplug`, `settings.player.{smoothing,speedLimit}`.

mod buttplug;
mod interp;
mod script;
mod tcode;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::server::AppState;
use interp::Action;

const KINDS: [&str; 2] = ["tcode", "buttplug"];
/// One engine tick, in ms (60 Hz).
const TICK: f64 = 1000.0 / 60.0;

#[derive(Clone)]
struct Play {
    session: u64,
    axes: Arc<Vec<(String, Vec<Action>)>>,
    at: f64,
    paused: bool,
    rate: f64,
    stamp: Instant,
}

impl Play {
    /// Media time now, extrapolated from the last state.
    fn time(&self) -> f64 {
        if self.paused {
            self.at
        } else {
            self.at + self.stamp.elapsed().as_secs_f64() * 1000.0 * self.rate
        }
    }

    /// Apply a phone message (`play`, `pause`, `seek`, `state`, `ended`).
    fn apply(&mut self, m: &Value) {
        self.at = m["at"].as_f64().unwrap_or_else(|| self.time());
        self.paused = match m["type"].as_str() {
            Some("play") => false,
            Some("pause" | "ended") => true,
            Some("state") => m["paused"].as_bool().unwrap_or(self.paused),
            _ => self.paused,
        };
        self.rate = m["rate"].as_f64().unwrap_or(self.rate);
        self.stamp = Instant::now();
    }
}

#[derive(Default)]
struct Status {
    connected: bool,
    error: Option<String>,
}

struct Out<T> {
    tx: watch::Sender<T>,
    task: Mutex<Option<JoinHandle<()>>>,
    status: Mutex<Status>,
}

impl<T: Default> Default for Out<T> {
    fn default() -> Self {
        Self {
            tx: watch::channel(T::default()).0,
            task: Mutex::default(),
            status: Mutex::default(),
        }
    }
}

#[derive(Default)]
pub struct Devices {
    play: Mutex<Option<Play>>,
    sessions: AtomicU64,
    ticking: AtomicBool,
    tcode: Out<String>,
    buttplug: Out<buttplug::Frame>,
    buttplug_names: Mutex<Vec<String>>,
}

impl Devices {
    fn status(&self, kind: &str) -> &Mutex<Status> {
        if kind == "tcode" {
            &self.tcode.status
        } else {
            &self.buttplug.status
        }
    }

    fn task(&self, kind: &str) -> &Mutex<Option<JoinHandle<()>>> {
        if kind == "tcode" {
            &self.tcode.task
        } else {
            &self.buttplug.task
        }
    }

    fn connected(&self, kind: &str) -> bool {
        self.status(kind).lock().unwrap().connected
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/remote/sync", get(sync))
        .route("/api/devices", get(get_devices).post(post_devices))
}

/// Start the engine and reconnect the devices that were connected.
pub async fn start(st: &Arc<AppState>) {
    for kind in KINDS {
        if settings(st, kind).await["autoConnect"] == true {
            connect(st, kind);
        }
    }
}

async fn settings(st: &AppState, kind: &str) -> Value {
    st.library.read().await.config["settings"][kind].clone()
}

/// `{tcode, buttplug, player: {smoothing, speedLimit}}`, without cloning the
/// rest of `settings`.
async fn device_settings(st: &AppState) -> Value {
    let lib = st.library.read().await;
    let s = &lib.config["settings"];
    json!({
        "tcode": s["tcode"],
        "buttplug": s["buttplug"],
        "player": {"smoothing": s["player"]["smoothing"], "speedLimit": s["player"]["speedLimit"]},
    })
}

fn disconnect(st: &AppState, kind: &str) {
    if let Some(h) = st.devices.task(kind).lock().unwrap().take() {
        h.abort();
    }
    *st.devices.status(kind).lock().unwrap() = Status::default();
    st.devices.buttplug_names.lock().unwrap().clear();
}

/// (Re)connect `kind`, retrying every 5 s for a minute, then every minute.
fn connect(st: &Arc<AppState>, kind: &'static str) {
    disconnect(st, kind);
    ensure_ticker(st);
    let st2 = st.clone();
    let h = tokio::spawn(async move {
        let d = &st2.devices;
        let mut fails = 0;
        loop {
            let cfg = settings(&st2, kind).await;
            let on = || {
                *d.status(kind).lock().unwrap() = Status {
                    connected: true,
                    error: None,
                }
            };
            let r = if kind == "tcode" {
                tcode::session(&cfg, &mut d.tcode.tx.subscribe(), on).await
            } else {
                let port = cfg["port"].as_u64().unwrap_or(12345);
                buttplug::session(port, &mut d.buttplug.tx.subscribe(), &d.buttplug_names, on).await
            };
            {
                let mut s = d.status(kind).lock().unwrap();
                fails = if s.connected { 1 } else { fails + 1 };
                *s = Status {
                    connected: false,
                    error: r.err(),
                };
            }
            d.buttplug_names.lock().unwrap().clear();
            let wait = if fails <= 12 { 5 } else { 60 };
            tokio::time::sleep(Duration::from_secs(wait)).await;
        }
    });
    *st.devices.task(kind).lock().unwrap() = Some(h);
}

/// The 60 Hz engine: one task, started with the first device or phone.
fn ensure_ticker(st: &Arc<AppState>) {
    if st.devices.ticking.swap(true, Ordering::SeqCst) {
        return;
    }
    let st = st.clone();
    tokio::spawn(async move {
        let d = &st.devices;
        let mut iv = tokio::time::interval(Duration::from_secs_f64(TICK / 1000.0));
        iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last = HashMap::new();
        loop {
            iv.tick().await;
            let play = d.play.lock().unwrap().clone();
            let Some(p) = play.filter(|p| !p.paused) else {
                // Vibrators stop with the video; strokers hold.
                d.buttplug
                    .tx
                    .send_if_modified(|f| std::mem::replace(&mut f.speed, 0.0) != 0.0);
                continue;
            };
            let s = device_settings(&st).await;
            let (tc, player) = (&s["tcode"], &s["player"]);
            let bp_off = s["buttplug"]["defaultOffset"].as_f64().unwrap_or(0.0);
            let t = p.time();
            if d.connected("tcode") {
                let off = tc["defaultOffset"].as_f64().unwrap_or(0.0);
                let f = tcode::frame(&p.axes, t + off, tc, player, TICK, &mut last);
                d.tcode.tx.send_if_modified(|v| {
                    !f.is_empty() && *v != f && {
                        *v = f;
                        true
                    }
                });
            }
            if d.connected("buttplug") {
                let (mode, main) = (
                    player["smoothing"].as_str().unwrap_or("linear"),
                    &p.axes[0].1,
                );
                let t = t + bp_off;
                let f = buttplug::Frame {
                    pos: interp::position(mode, main, t).unwrap_or(0.0).round(),
                    speed: interp::speed(main, t).round(),
                    ms: TICK.round() as u32,
                };
                d.buttplug.tx.send_if_modified(|v| {
                    *v != f && {
                        *v = f;
                        true
                    }
                });
            }
        }
    });
}

async fn device_status(st: &AppState) -> Value {
    let s = device_settings(st).await;
    let off = |k: &str| s[k]["defaultOffset"].as_i64().unwrap_or(0);
    let mut devices = Vec::new();
    if st.devices.connected("tcode") {
        let label = format!("TCode ({})", s["tcode"]["transport"].as_str().unwrap_or(""));
        devices.push(json!({"kind": "tcode", "label": label, "offsetMs": off("tcode")}));
    }
    if st.devices.connected("buttplug") {
        for n in st.devices.buttplug_names.lock().unwrap().iter() {
            devices.push(json!({"kind": "buttplug", "label": n, "offsetMs": off("buttplug")}));
        }
    }
    json!({"type": "device-status", "devices": devices})
}

#[derive(Deserialize)]
struct SyncQuery {
    #[serde(rename = "videoId")]
    video_id: String,
}

async fn sync(
    ws: WebSocketUpgrade,
    Query(q): Query<SyncQuery>,
    State(st): State<Arc<AppState>>,
) -> Response {
    ws.on_upgrade(move |sock| sync_session(st, q.video_id, sock))
}

async fn send(sock: &mut WebSocket, v: Value) {
    let _ = sock.send(Message::text(v.to_string())).await;
}

/// One phone playing one video; the newest phone drives the devices.
async fn sync_session(st: Arc<AppState>, id: String, mut sock: WebSocket) {
    ensure_ticker(&st);
    let n = st.devices.sessions.fetch_add(1, Ordering::SeqCst) + 1;
    let axes = match st.video(&id).await {
        Some(v) => tokio::task::spawn_blocking(move || script::load(&v))
            .await
            .ok()
            .flatten(),
        None => None,
    };
    let kind = if axes.is_some() {
        "script-ready"
    } else {
        "script-missing"
    };
    *st.devices.play.lock().unwrap() = axes.map(|a| Play {
        session: n,
        axes: Arc::new(a),
        at: 0.0,
        paused: true,
        rate: 1.0,
        stamp: Instant::now(),
    });
    send(&mut sock, json!({"type": kind})).await;
    send(&mut sock, device_status(&st).await).await;
    while let Some(Ok(msg)) = sock.recv().await {
        let Message::Text(text) = msg else { continue };
        let Ok(m) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        match m["type"].as_str() {
            Some("bye") => break,
            Some("set-offset") => {
                let kind = m["device"].as_str().unwrap_or("");
                if KINDS.contains(&kind) {
                    let ms = m["ms"].as_f64().unwrap_or(0.0).clamp(-2000.0, 2000.0);
                    st.library
                        .write()
                        .await
                        .section_mut(kind)
                        .insert("defaultOffset".into(), json!(ms as i64));
                    let _ = st.save().await;
                }
                send(&mut sock, device_status(&st).await).await;
            }
            _ => {
                if let Some(p) = st
                    .devices
                    .play
                    .lock()
                    .unwrap()
                    .as_mut()
                    .filter(|p| p.session == n)
                {
                    p.apply(&m);
                }
            }
        }
    }
    let mut play = st.devices.play.lock().unwrap();
    if play.as_ref().is_some_and(|p| p.session == n) {
        *play = None;
    }
}

async fn devices_json(st: &AppState) -> Value {
    let mut s = device_settings(st).await;
    let status = |k: &str| {
        let s = st.devices.status(k).lock().unwrap();
        json!({"connected": s.connected, "error": s.error})
    };
    let mut bp = status("buttplug");
    bp["devices"] = json!(*st.devices.buttplug_names.lock().unwrap());
    s["status"] = json!({"tcode": status("tcode"), "buttplug": bp});
    s
}

async fn get_devices(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(devices_json(&st).await)
}

/// `{kind: tcode|buttplug|player, settings?: {…merged…}, action?: connect|disconnect}`.
async fn post_devices(State(st): State<Arc<AppState>>, Json(b): Json<Value>) -> Response {
    let Some(kind) = ["tcode", "buttplug", "player"]
        .into_iter()
        .find(|k| b["kind"] == *k)
    else {
        return (StatusCode::BAD_REQUEST, "kind: tcode, buttplug or player").into_response();
    };
    let action = b["action"].as_str();
    {
        let mut lib = st.library.write().await;
        let sec = lib.section_mut(kind);
        for (k, v) in b["settings"].as_object().into_iter().flatten() {
            sec.insert(k.clone(), v.clone());
        }
        if kind != "player" && action.is_some() {
            sec.insert("autoConnect".into(), json!(action == Some("connect")));
        }
    }
    if let Err(e) = st.save().await {
        return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }
    let kind = KINDS.into_iter().find(|k| *k == kind);
    match (kind, action) {
        (Some(k), Some("connect")) => connect(&st, k),
        (Some(k), Some("disconnect")) => disconnect(&st, k),
        _ => {}
    }
    Json(devices_json(&st).await).into_response()
}
