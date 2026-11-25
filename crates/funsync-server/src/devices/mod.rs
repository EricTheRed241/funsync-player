//! Device output, driven by the server: the web remote streams its playback
//! state over `/api/remote/sync` (the phone side of
//! `backend/web-remote/remote-sync.js`), a 60 Hz engine plays the video's
//! scripts from it, and sends TCode (WebSocket, UDP, USB serial) and
//! Buttplug (Intiface) commands. Settings: `settings.tcode` (with its
//! device profiles), `settings.buttplug`, `settings.player.{smoothing,speedLimit}`.

mod buttplug;
mod dashboard;
mod interp;
mod profiles;
mod script;
mod tcode;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;

use crate::data::now_ms;
use crate::server::AppState;
use interp::Action;

pub use dashboard::status;

const KINDS: [&str; 2] = ["tcode", "buttplug"];
/// One engine tick, in ms (60 Hz).
const TICK: f64 = 1000.0 / 60.0;
/// What the old backend tells a phone another one replaced.
const KICK_REPLACED: &str = "Another device took over this session.";
/// FUNSR MINI settings are sent again this long after each connect.
const FUNSR_RESEND: Duration = Duration::from_millis(2500);

#[derive(Clone)]
struct Play {
    session: u64,
    video_id: String,
    name: String,
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

/// A device link, as `/api/status` and the dashboard report it.
#[derive(Default)]
struct Status {
    running: bool,
    connected: bool,
    /// Connected at least once since Connect.
    ever: bool,
    /// Connected since / down since.
    since: Option<u64>,
    attempts: u32,
    retry_ms: u64,
    next_retry: Option<u64>,
    error: Option<(String, u64)>,
    commands: u64,
    bytes: u64,
    last_send: Option<u64>,
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
    /// The phone session that drives the devices; older ones are kicked.
    active: AtomicU64,
    kick: Notify,
    /// `(ip, videoId)` of the active phone.
    phone: Mutex<Option<(String, String)>>,
    ticking: AtomicBool,
    tcode: Out<String>,
    buttplug: Out<buttplug::Frame>,
    buttplug_names: Mutex<Vec<String>>,
    telem: Mutex<dashboard::Telem>,
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

    fn event(&self, kind: &str, level: &str, text: String) {
        self.telem.lock().unwrap().event(kind, level, text);
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/remote/sync", get(sync))
        .route("/api/devices", get(get_devices).post(post_devices))
        .merge(dashboard::routes())
}

/// Start the engine and reconnect the devices that were connected.
pub async fn start(st: &Arc<AppState>) {
    ensure_ticker(st);
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
        st.devices
            .event("link", "info", format!("{kind} disconnected"));
    }
    *st.devices.status(kind).lock().unwrap() = Status::default();
    st.devices.buttplug_names.lock().unwrap().clear();
}

/// Send the FUNSR MINI's `D` settings now, when it is the active profile.
fn send_funsr(st: &AppState, tc: &Value) {
    let cmds = profiles::funsr_commands(&tc["funsr"]);
    if profiles::active(tc) == "funsr-mini" && !cmds.is_empty() && st.devices.connected("tcode") {
        st.devices.tcode.tx.send_replace(cmds);
    }
}

/// (Re)connect `kind`, retrying every 5 s for a minute, then every minute.
fn connect(st: &Arc<AppState>, kind: &'static str) {
    disconnect(st, kind);
    ensure_ticker(st);
    st.devices.status(kind).lock().unwrap().running = true;
    let st2 = st.clone();
    let h = tokio::spawn(async move {
        let d = &st2.devices;
        loop {
            let cfg = settings(&st2, kind).await;
            let on = || {
                {
                    let mut s = d.status(kind).lock().unwrap();
                    *s = Status {
                        running: true,
                        connected: true,
                        ever: true,
                        since: Some(now_ms()),
                        ..Status::default()
                    };
                }
                let to = if kind == "tcode" {
                    dashboard::address(&cfg)
                } else {
                    "Intiface".into()
                };
                d.event("link", "ok", format!("{kind} connected to {to}"));
                if kind == "tcode" {
                    let (st3, cfg) = (st2.clone(), cfg.clone());
                    tokio::spawn(async move {
                        tokio::time::sleep(FUNSR_RESEND).await;
                        send_funsr(&st3, &cfg);
                    });
                }
            };
            let r = if kind == "tcode" {
                tcode::session(&cfg, &mut d.tcode.tx.subscribe(), on).await
            } else {
                let port = cfg["port"].as_u64().unwrap_or(12345);
                buttplug::session(port, &mut d.buttplug.tx.subscribe(), &d.buttplug_names, on).await
            };
            let err = r.err().unwrap_or_default();
            let wait = {
                let mut s = d.status(kind).lock().unwrap();
                let now = now_ms();
                if s.connected {
                    s.connected = false;
                    s.attempts = 0;
                    s.since = Some(now);
                    d.event("link", "bad", format!("{kind} dropped: {err}"));
                } else if s.error.as_ref().is_none_or(|e| e.0 != err) {
                    d.event("link", "warn", format!("{kind} connect failed: {err}"));
                }
                s.since.get_or_insert(now);
                s.attempts += 1;
                s.retry_ms = if s.attempts <= 12 { 5000 } else { 60_000 };
                s.next_retry = Some(now + s.retry_ms);
                s.error = Some((err, now));
                s.retry_ms
            };
            d.buttplug_names.lock().unwrap().clear();
            tokio::time::sleep(Duration::from_millis(wait)).await;
        }
    });
    *st.devices.task(kind).lock().unwrap() = Some(h);
}

/// The 60 Hz engine: one task, started with the server. Also takes the
/// dashboard's sample once a second.
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
        for n in 0u64.. {
            iv.tick().await;
            let play = d.play.lock().unwrap().clone();
            let s = device_settings(&st).await;
            if n % 60 == 0 {
                let sample = dashboard::sample(&st, play.as_ref(), &s);
                d.telem.lock().unwrap().sample(sample);
            }
            let Some(p) = play.filter(|p| !p.paused) else {
                // Vibrators stop with the video; strokers hold.
                d.buttplug
                    .tx
                    .send_if_modified(|f| std::mem::replace(&mut f.speed, 0.0) != 0.0);
                continue;
            };
            let (tc, player) = (&s["tcode"], &s["player"]);
            let t = p.time();
            if d.connected("tcode") {
                let off = tc["defaultOffset"].as_f64().unwrap_or(0.0);
                let f = tcode::frame(&p.axes, t + off, tc, player, TICK, &mut last);
                let sent = d.tcode.tx.send_if_modified(|v| {
                    !f.is_empty() && *v != f && {
                        v.clone_from(&f);
                        true
                    }
                });
                if sent {
                    let mut s = d.tcode.status.lock().unwrap();
                    s.commands += 1;
                    s.bytes += f.len() as u64;
                    s.last_send = Some(now_ms());
                    drop(s);
                    d.telem.lock().unwrap().command(&f);
                }
            }
            if d.connected("buttplug") {
                let (mode, main) = (
                    player["smoothing"].as_str().unwrap_or("linear"),
                    &p.axes[0].1,
                );
                let t = t + s["buttplug"]["defaultOffset"].as_f64().unwrap_or(0.0);
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
        let label = dashboard::label(&s["tcode"]);
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
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(q): Query<SyncQuery>,
    State(st): State<Arc<AppState>>,
) -> Response {
    ws.on_upgrade(move |sock| sync_session(st, q.video_id, addr.ip().to_string(), sock))
}

async fn send(sock: &mut WebSocket, v: Value) {
    let _ = sock.send(Message::text(v.to_string())).await;
}

/// Load the video's script (or variant) and make it what the engine plays,
/// keeping the playback state; tells the phone `script-ready` / `-missing`.
async fn load_script(
    st: &AppState,
    n: u64,
    video: Option<&crate::scan::Video>,
    variant: Option<&str>,
    sock: &mut WebSocket,
) {
    let axes = match video {
        Some(v) => {
            let (v, variant) = (v.clone(), variant.map(String::from));
            tokio::task::spawn_blocking(move || script::load(&v, variant.as_deref()))
                .await
                .ok()
                .flatten()
        }
        None => None,
    };
    let ready = axes.is_some();
    if let (Some(v), Some(a)) = (video, &axes) {
        let ids: Vec<&str> = a.iter().map(|x| x.0.as_str()).collect();
        let what = variant.map_or(String::new(), |l| format!(" ({l})"));
        st.devices.event(
            "script",
            "info",
            format!("{}{what}: axes {}", v.name, ids.join(" ")),
        );
    }
    {
        let mut play = st.devices.play.lock().unwrap();
        let prev = play.take().filter(|p| p.session == n);
        *play = axes.map(|a| {
            let mut p = prev.unwrap_or(Play {
                session: n,
                video_id: video.map_or(String::new(), |v| v.id.clone()),
                name: video.map_or(String::new(), |v| v.name.clone()),
                axes: Arc::default(),
                at: 0.0,
                paused: true,
                rate: 1.0,
                stamp: Instant::now(),
            });
            p.axes = Arc::new(a);
            p
        });
    }
    let kind = if ready {
        "script-ready"
    } else {
        "script-missing"
    };
    send(sock, json!({"type": kind})).await;
}

/// One phone playing one video; the newest phone drives the devices and
/// the older one is sent `kicked`.
async fn sync_session(st: Arc<AppState>, id: String, ip: String, mut sock: WebSocket) {
    let d = &st.devices;
    ensure_ticker(&st);
    let n = d.sessions.fetch_add(1, Ordering::SeqCst) + 1;
    d.active.store(n, Ordering::SeqCst);
    d.kick.notify_waiters();
    *d.phone.lock().unwrap() = Some((ip, id.clone()));
    let video = st.video(&id).await;
    load_script(&st, n, video.as_ref(), None, &mut sock).await;
    send(&mut sock, device_status(&st).await).await;
    loop {
        let kicked = d.kick.notified();
        tokio::pin!(kicked);
        kicked.as_mut().enable();
        if d.active.load(Ordering::SeqCst) != n {
            send(
                &mut sock,
                json!({"type": "kicked", "reason": KICK_REPLACED}),
            )
            .await;
            d.event("phone", "info", "a newer phone took over".into());
            return;
        }
        let msg = tokio::select! {
            m = sock.recv() => m,
            _ = kicked => continue,
        };
        let Some(Ok(msg)) = msg else { break };
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
            Some("switch-variant") => {
                let label = m["label"].as_str().unwrap_or("");
                // The variant that is the main script plays as the main script.
                let variant = video
                    .as_ref()
                    .and_then(|v| {
                        v.variants.iter().find(|x| {
                            x.label == label && Some(&x.path) != v.funscript_path.as_ref()
                        })
                    })
                    .map(|x| x.label.as_str());
                load_script(&st, n, video.as_ref(), variant, &mut sock).await;
                send(
                    &mut sock,
                    json!({"type": "variant-changed", "label": label}),
                )
                .await;
            }
            _ => {
                if let Some(p) = d.play.lock().unwrap().as_mut().filter(|p| p.session == n) {
                    p.apply(&m);
                }
            }
        }
    }
    let mut play = d.play.lock().unwrap();
    if play.as_ref().is_some_and(|p| p.session == n) {
        *play = None;
    }
    if d.active.load(Ordering::SeqCst) == n {
        *d.phone.lock().unwrap() = None;
    }
}

async fn devices_json(st: &AppState) -> Value {
    let mut s = device_settings(st).await;
    let status = |k: &str| {
        let s = st.devices.status(k).lock().unwrap();
        json!({"connected": s.connected, "error": s.error.as_ref().map(|e| &e.0)})
    };
    let mut bp = status("buttplug");
    bp["devices"] = json!(*st.devices.buttplug_names.lock().unwrap());
    s["status"] = json!({"tcode": status("tcode"), "buttplug": bp});
    s["profile"] = json!(profiles::active(&s["tcode"]));
    s
}

async fn get_devices(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(devices_json(&st).await)
}

/// `{kind: tcode|buttplug|player, settings?: {…merged…}, action?:
/// connect|disconnect, profile?: mirabot|funsr-mini|custom}`. Picking another
/// TCode profile disconnects, swaps the settings and connects the new one
/// (unless it was disconnected last time).
async fn post_devices(State(st): State<Arc<AppState>>, Json(b): Json<Value>) -> Response {
    let Some(kind) = ["tcode", "buttplug", "player"]
        .into_iter()
        .find(|k| b["kind"] == *k)
    else {
        return (StatusCode::BAD_REQUEST, "kind: tcode, buttplug or player").into_response();
    };
    let mut action = b["action"].as_str();
    let (switched, tc) = {
        let mut lib = st.library.write().await;
        let sec = lib.section_mut(kind);
        let switched = kind == "tcode"
            && b["profile"]
                .as_str()
                .is_some_and(|p| profiles::switch(sec, p));
        for (k, v) in b["settings"].as_object().into_iter().flatten() {
            sec.insert(k.clone(), v.clone());
        }
        if switched && action.is_none() && sec.get("autoConnect") != Some(&json!(false)) {
            action = Some("connect");
        }
        if kind != "player" && action.is_some() {
            sec.insert("autoConnect".into(), json!(action == Some("connect")));
        }
        (switched, Value::Object(sec.clone()))
    };
    if let Err(e) = st.save().await {
        return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }
    if switched {
        disconnect(&st, "tcode");
    }
    let kind = KINDS.into_iter().find(|k| *k == kind);
    match (kind, action) {
        (Some(k), Some("connect")) => connect(&st, k),
        (Some(k), Some("disconnect")) => disconnect(&st, k),
        (Some("tcode"), None) if b["settings"].get("funsr").is_some() => send_funsr(&st, &tc),
        _ => {}
    }
    Json(devices_json(&st).await).into_response()
}
