//! Device output, driven by the server. Playback comes from the web remote
//! (its playback state over `/api/remote/sync`, the phone side of
//! `backend/web-remote/remote-sync.js`) or from the PCVR companion bridge
//! (HereSphere's timestamp server); the newest source owns the session. A
//! 60 Hz engine plays the video's scripts and sends TCode (WebSocket, UDP,
//! USB serial) and Buttplug (Intiface) commands. Settings: `settings.tcode`
//! (with its device profiles), `settings.buttplug`, `settings.vr`,
//! `settings.player.{smoothing,speedLimit,orgasmConfig,orgasmSwitchMode}`.

mod buttplug;
mod dashboard;
mod interp;
mod profiles;
mod routing;
pub(crate) mod script;
mod tcode;
mod vr;

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

/// Device links: outputs, and the VR bridge (an input).
const KINDS: [&str; 3] = ["tcode", "buttplug", "vr"];
/// One engine tick, in ms (60 Hz).
const TICK: f64 = 1000.0 / 60.0;
/// What the old backend tells a phone another source replaced.
const KICK_REPLACED: &str = "Another device took over this session.";
/// FUNSR MINI settings are sent again this long after each connect.
const FUNSR_RESEND: Duration = Duration::from_millis(2500);
/// Sessions kept in `settings.sessions.history`.
const HISTORY_MAX: usize = 50;

type Axes = Arc<Vec<(String, Vec<Action>)>>;

#[derive(Clone)]
struct Play {
    session: u64,
    /// `web-remote` or `vr`.
    source: &'static str,
    video_id: String,
    path: String,
    name: String,
    axes: Axes,
    routes: Arc<Vec<routing::Route>>,
    at: f64,
    paused: bool,
    rate: f64,
    stamp: Instant,
    /// The Orgasm Switch's looping script and when it started.
    orgasm: Option<(Axes, Instant)>,
    /// Stopped by the Orgasm Switch's finish (toggle mode) until `play`.
    halted: bool,
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
            Some("play") => {
                self.halted = false;
                false
            }
            Some("pause" | "ended") => true,
            Some("state") => m["paused"].as_bool().unwrap_or(self.paused),
            _ => self.paused,
        };
        self.rate = m["rate"].as_f64().unwrap_or(self.rate);
        self.stamp = Instant::now();
    }

    /// What to play now: the orgasm loop, or the video's scripts at its
    /// time; None when nothing should move.
    fn now(&self) -> Option<(&Axes, f64)> {
        if let Some((axes, start)) = &self.orgasm {
            let len = axes[0].1.last().map_or(1.0, |a| a.at.max(1.0));
            return Some((axes, start.elapsed().as_secs_f64() * 1000.0 % len));
        }
        (!self.paused && !self.halted).then(|| (&self.axes, self.time()))
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
    /// The source session that drives the devices; older phones are kicked.
    active: AtomicU64,
    kick: Notify,
    /// The current session (`session-tracker.js` shape).
    current: Mutex<Option<Value>>,
    /// `(ip, videoId)` of the active phone.
    phone: Mutex<Option<(String, String)>>,
    ticking: AtomicBool,
    tcode: Out<String>,
    buttplug: Out<buttplug::Frame>,
    vr: Out<()>,
    vr_link: Mutex<vr::Link>,
    buttplug_names: Mutex<Vec<(u64, String)>>,
    telem: Mutex<dashboard::Telem>,
}

impl Devices {
    fn out_status(&self, kind: &str) -> (&Mutex<Status>, &Mutex<Option<JoinHandle<()>>>) {
        match kind {
            "tcode" => (&self.tcode.status, &self.tcode.task),
            "buttplug" => (&self.buttplug.status, &self.buttplug.task),
            _ => (&self.vr.status, &self.vr.task),
        }
    }

    fn status(&self, kind: &str) -> &Mutex<Status> {
        self.out_status(kind).0
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

/// Start the engine and reconnect the links that were connected.
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

/// `{tcode, buttplug, vr, player: {…}}`, without cloning the rest of
/// `settings`.
async fn device_settings(st: &AppState) -> Value {
    let lib = st.library.read().await;
    let s = &lib.config["settings"];
    let p = &s["player"];
    json!({
        "tcode": s["tcode"],
        "buttplug": s["buttplug"],
        "vr": s["vr"],
        "player": {"smoothing": p["smoothing"], "speedLimit": p["speedLimit"],
                   "orgasmConfig": p["orgasmConfig"], "orgasmSwitchMode": p["orgasmSwitchMode"]},
    })
}

fn disconnect(st: &AppState, kind: &str) {
    if let Some(h) = st.devices.out_status(kind).1.lock().unwrap().take() {
        h.abort();
        st.devices
            .event("link", "info", format!("{kind} disconnected"));
    }
    *st.devices.status(kind).lock().unwrap() = Status::default();
    if kind == "buttplug" {
        st.devices.buttplug_names.lock().unwrap().clear();
    }
}

/// Send the FUNSR MINI's `D` settings now, when it is the active profile.
fn send_funsr(st: &AppState, tc: &Value) {
    let cmds = profiles::funsr_commands(&tc["funsr"]);
    if profiles::active(tc) == "funsr-mini" && !cmds.is_empty() && st.devices.connected("tcode") {
        st.devices.tcode.tx.send_replace(cmds);
    }
}

/// (Re)connect `kind`, retrying every 5 s for a minute, then every minute;
/// a USB serial port that isn't there is looked for every 5 s (hot-plug).
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
                *d.status(kind).lock().unwrap() = Status {
                    running: true,
                    connected: true,
                    ever: true,
                    since: Some(now_ms()),
                    ..Status::default()
                };
                let to = match kind {
                    "tcode" => dashboard::address(&cfg),
                    "vr" => format!("{}:{}", cfg["lastHost"], cfg["lastPort"]),
                    _ => "Intiface".into(),
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
            let r = match kind {
                "tcode" => tcode::session(&cfg, &mut d.tcode.tx.subscribe(), on).await,
                "vr" => vr::session(&st2, &cfg, on).await,
                _ => {
                    let port = cfg["port"].as_u64().unwrap_or(12345);
                    buttplug::session(port, &mut d.buttplug.tx.subscribe(), &d.buttplug_names, on)
                        .await
                }
            };
            let err = r.err().unwrap_or_default();
            let unplugged = kind == "tcode"
                && cfg["transport"] == "serial"
                && !std::path::Path::new(cfg["port"].as_str().unwrap_or("")).exists();
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
                s.retry_ms = if s.attempts <= 12 || unplugged {
                    5000
                } else {
                    60_000
                };
                s.next_retry = Some(now + s.retry_ms);
                s.error = Some((err, now));
                s.retry_ms
            };
            if kind == "buttplug" {
                d.buttplug_names.lock().unwrap().clear();
            }
            tokio::time::sleep(Duration::from_millis(wait)).await;
        }
    });
    *st.devices.out_status(kind).1.lock().unwrap() = Some(h);
}

/// `(device index, pos, speed)` per routed Buttplug device, and the
/// `(route, index)` pairs of routes found by name under a new index.
type Routed = (Vec<(u64, f64, f64)>, Vec<(usize, u64)>);

/// Each routed Buttplug device's position and speed at `t`.
fn route_frame(p: &Play, devices: &[(u64, String)], mode: &str, t: f64) -> Routed {
    let (mut out, mut healed, mut taken) = (Vec::new(), Vec::new(), Vec::new());
    for (i, r) in p.routes.iter().enumerate() {
        let Some(idx) = routing::find(r, devices, &taken) else {
            continue;
        };
        taken.push(idx);
        if r.index != Some(idx) {
            healed.push((i, idx));
        }
        let pos = interp::position(mode, &r.actions, t).unwrap_or(0.0).round();
        out.push((idx, pos, interp::speed(&r.actions, t).round()));
    }
    (out, healed)
}

/// Store routes' new Buttplug indexes in the video's association.
async fn heal_routes(st: &AppState, path: &str, healed: &[(usize, u64)]) {
    {
        let mut lib = st.library.write().await;
        let routes = &mut lib.lib_mut("associations")[path]["custom"]["routes"];
        for (i, idx) in healed {
            if routes[*i].is_object() {
                routes[*i]["buttplugIndex"] = json!(idx);
            }
        }
    }
    let _ = st.save().await;
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
            let Some((p, (axes, t))) = play.as_ref().and_then(|p| Some((p, p.now()?))) else {
                // Vibrators stop with the video; strokers hold.
                d.buttplug.tx.send_if_modified(buttplug::Frame::still);
                continue;
            };
            let (tc, player) = (&s["tcode"], &s["player"]);
            let mode = player["smoothing"].as_str().unwrap_or("linear");
            if d.connected("tcode") {
                let off = tc["defaultOffset"].as_f64().unwrap_or(0.0);
                let f = tcode::frame(axes, t + off, tc, player, TICK, &mut last);
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
                let main = &axes[0].1;
                let t = t + s["buttplug"]["defaultOffset"].as_f64().unwrap_or(0.0);
                let routed = (p.orgasm.is_none() && !p.routes.is_empty()).then(|| {
                    let devices = d.buttplug_names.lock().unwrap().clone();
                    let (out, healed) = route_frame(p, &devices, mode, t);
                    if !healed.is_empty() {
                        if let Some(cur) = d
                            .play
                            .lock()
                            .unwrap()
                            .as_mut()
                            .filter(|c| c.session == p.session)
                        {
                            let r = Arc::make_mut(&mut cur.routes);
                            for (i, idx) in &healed {
                                r[*i].index = Some(*idx);
                            }
                        }
                        let (st, path) = (st.clone(), p.path.clone());
                        tokio::spawn(async move { heal_routes(&st, &path, &healed).await });
                    }
                    out
                });
                let f = buttplug::Frame {
                    pos: interp::position(mode, main, t).unwrap_or(0.0).round(),
                    speed: interp::speed(main, t).round(),
                    ms: TICK.round() as u32,
                    routed,
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

/// Start a session for `source`; the current one (another source, or
/// another phone) ends and is kept in the history, last wins: an evicted
/// phone is kicked, an evicted VR bridge is disconnected.
async fn session_begin(st: &Arc<AppState>, n: u64, source: &str, identifier: String) {
    let d = &st.devices;
    let old = d.current.lock().unwrap().replace(json!({
        "id": n, "source": source, "identifier": identifier,
        "startedAt": now_ms(), "endedAt": null, "videoName": null, "videoId": null, "videos": [],
    }));
    d.active.store(n, Ordering::SeqCst);
    d.kick.notify_waiters();
    if source == "vr" {
        *d.phone.lock().unwrap() = None;
    }
    if let Some(old) = old {
        if old["source"] == "vr" && source != "vr" {
            disconnect(st, "vr");
            d.event(
                "session",
                "info",
                "the web remote took over from the VR bridge".into(),
            );
        } else if old["source"] == "web-remote" && source == "vr" {
            d.event(
                "session",
                "info",
                "the VR bridge took over from the web remote".into(),
            );
        }
        finish(st, old).await;
    }
}

/// End session `n` if it is still the current one.
async fn session_end(st: &AppState, n: u64) {
    let old = {
        let mut cur = st.devices.current.lock().unwrap();
        let mine = cur.as_ref().is_some_and(|c| c["id"] == n);
        if mine { cur.take() } else { None }
    };
    if let Some(old) = old {
        finish(st, old).await;
    }
}

/// Close a session and keep it in `settings.sessions.history` (newest last,
/// 50 at most).
async fn finish(st: &AppState, mut s: Value) {
    let now = now_ms();
    s["endedAt"] = json!(now);
    if let Some(v) = s["videos"].as_array_mut().and_then(|v| v.last_mut()) {
        v["endedAt"] = json!(now);
    }
    {
        let mut lib = st.library.write().await;
        let sec = lib.section_mut("sessions");
        let h = sec.entry("history").or_insert_with(|| json!([]));
        if !h.is_array() {
            *h = json!([]);
        }
        let h = h.as_array_mut().unwrap();
        h.push(s);
        let extra = h.len().saturating_sub(HISTORY_MAX);
        h.drain(..extra);
    }
    let _ = st.save().await;
}

/// Load the video's script (or variant) for session `n` and make it what
/// the engine plays, keeping the playback state. True when it has one.
async fn load_play(
    st: &AppState,
    n: u64,
    source: &'static str,
    video: Option<&crate::scan::Video>,
    variant: Option<&str>,
) -> bool {
    let d = &st.devices;
    let entry = match video {
        Some(v) => st.library.read().await.lib()["associations"][&v.path].clone(),
        None => Value::Null,
    };
    let loaded = match video {
        Some(v) => {
            let (v, variant) = (v.clone(), variant.map(String::from));
            tokio::task::spawn_blocking(move || {
                let axes = script::load(&v, variant.as_deref())?;
                let routes = routing::load(&entry, &axes[0].1);
                Some((axes, routes))
            })
            .await
            .ok()
            .flatten()
        }
        None => None,
    };
    if let Some(v) = video {
        let mut cur = d.current.lock().unwrap();
        if let Some(c) = cur.as_mut().filter(|c| c["id"] == n) {
            if let Some(prev) = c["videos"].as_array_mut().and_then(|x| x.last_mut()) {
                prev["endedAt"] = json!(now_ms());
            }
            c["videoName"] = json!(v.name);
            c["videoId"] = json!(v.id);
            c["videos"]
                .as_array_mut()
                .unwrap()
                .push(json!({"name": v.name, "startedAt": now_ms(), "endedAt": null}));
        }
    }
    if let (Some(v), Some((a, r))) = (video, &loaded) {
        let ids: Vec<&str> = a.iter().map(|x| x.0.as_str()).collect();
        let what = variant.map_or(String::new(), |l| format!(" ({l})"));
        let routed = if r.is_empty() {
            String::new()
        } else {
            format!(", {} routes", r.len())
        };
        d.event(
            "script",
            "info",
            format!("{}{what}: axes {}{routed}", v.name, ids.join(" ")),
        );
    }
    let ready = loaded.is_some();
    let mut play = d.play.lock().unwrap();
    let prev = play.take().filter(|p| p.session == n);
    *play = loaded.map(|(a, r)| {
        let mut p = prev.unwrap_or(Play {
            session: n,
            source,
            video_id: video.map_or(String::new(), |v| v.id.clone()),
            path: video.map_or(String::new(), |v| v.path.clone()),
            name: video.map_or(String::new(), |v| v.name.clone()),
            axes: Arc::default(),
            routes: Arc::default(),
            at: 0.0,
            paused: true,
            rate: 1.0,
            stamp: Instant::now(),
            orgasm: None,
            halted: false,
        });
        if let Some(v) = video {
            (p.video_id, p.path, p.name) = (v.id.clone(), v.path.clone(), v.name.clone());
        }
        p.axes = Arc::new(a);
        p.routes = Arc::new(r);
        p
    });
    ready
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
        for (_, n) in st.devices.buttplug_names.lock().unwrap().iter() {
            devices.push(json!({"kind": "buttplug", "label": n, "offsetMs": off("buttplug")}));
        }
    }
    json!({"type": "device-status", "devices": devices})
}

/// The Orgasm Switch's state for the phone's button.
async fn orgasm_state(st: &AppState, n: u64) -> Value {
    let p = &device_settings(st).await["player"];
    let active = st
        .devices
        .play
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|x| x.session == n && x.orgasm.is_some());
    json!({"type": "orgasm-state", "configured": script_configured(&p["orgasmConfig"]),
           "active": active, "mode": if p["orgasmSwitchMode"] == "toggle" { "toggle" } else { "hold" }})
}

fn script_configured(cfg: &Value) -> bool {
    cfg["active"].is_string()
}

/// `orgasm-hold` from the phone: hold mode plays the looping orgasm script
/// while held; toggle mode starts it on one press and on the next stops the
/// devices until playback starts again.
async fn orgasm_hold(st: &AppState, n: u64, active: bool) {
    let p = device_settings(st).await["player"].clone();
    let toggle = p["orgasmSwitchMode"] == "toggle";
    if toggle && !active {
        return;
    }
    let running = st
        .devices
        .play
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|x| x.orgasm.is_some());
    let start = if toggle { !running } else { active };
    let plan = if start {
        let cfg = p["orgasmConfig"].clone();
        tokio::task::spawn_blocking(move || script::orgasm(&cfg))
            .await
            .ok()
            .flatten()
    } else {
        None
    };
    if let Some(x) = st
        .devices
        .play
        .lock()
        .unwrap()
        .as_mut()
        .filter(|x| x.session == n)
    {
        x.halted = toggle && running;
        x.orgasm = plan.map(|a| (Arc::new(a), Instant::now()));
    }
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

async fn send_script(sock: &mut WebSocket, ready: bool) {
    let kind = if ready {
        "script-ready"
    } else {
        "script-missing"
    };
    send(sock, json!({"type": kind})).await;
}

/// One phone playing one video; the newest source drives the devices and
/// an older phone is sent `kicked`.
async fn sync_session(st: Arc<AppState>, id: String, ip: String, mut sock: WebSocket) {
    let d = &st.devices;
    ensure_ticker(&st);
    let n = d.sessions.fetch_add(1, Ordering::SeqCst) + 1;
    session_begin(&st, n, "web-remote", ip.clone()).await;
    *d.phone.lock().unwrap() = Some((ip, id.clone()));
    let video = st.video(&id).await;
    let ready = load_play(&st, n, "web-remote", video.as_ref(), None).await;
    send_script(&mut sock, ready).await;
    send(&mut sock, device_status(&st).await).await;
    send(&mut sock, orgasm_state(&st, n).await).await;
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
            d.event("phone", "info", "another source took over".into());
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
                if ["tcode", "buttplug"].contains(&kind) {
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
                let ready = load_play(&st, n, "web-remote", video.as_ref(), variant).await;
                send_script(&mut sock, ready).await;
                send(
                    &mut sock,
                    json!({"type": "variant-changed", "label": label}),
                )
                .await;
            }
            Some("orgasm-hold") => {
                orgasm_hold(&st, n, m["active"] == true).await;
                send(&mut sock, orgasm_state(&st, n).await).await;
            }
            _ => {
                if let Some(p) = d.play.lock().unwrap().as_mut().filter(|p| p.session == n) {
                    p.apply(&m);
                }
            }
        }
    }
    {
        let mut play = d.play.lock().unwrap();
        if play.as_ref().is_some_and(|p| p.session == n) {
            *play = None;
        }
    }
    if d.active.load(Ordering::SeqCst) == n {
        *d.phone.lock().unwrap() = None;
    }
    session_end(&st, n).await;
}

/// The current session (with its live state) and the history.
async fn sessions_json(st: &AppState) -> Value {
    let mut cur = st.devices.current.lock().unwrap().clone();
    if let Some(c) = cur.as_mut() {
        let play = st.devices.play.lock().unwrap().clone();
        let p = play.filter(|p| c["id"] == p.session);
        c["state"] = json!(match &p {
            None if c["videoId"].is_null() => "idle",
            None => "no-script",
            Some(p) if p.paused => "paused",
            Some(_) => "playing",
        });
        c["currentTime"] = json!(p.as_ref().map(|p| (p.time() / 1000.0).max(0.0)));
        c["actionCount"] = json!(p.as_ref().map(|p| p.axes[0].1.len()));
    }
    let history = st.library.read().await.config["settings"]["sessions"]["history"].clone();
    json!({"current": cur, "history": if history.is_array() { history } else { json!([]) }})
}

async fn devices_json(st: &AppState) -> Value {
    let mut s = device_settings(st).await;
    let status = |k: &str| {
        let s = st.devices.status(k).lock().unwrap();
        json!({"connected": s.connected, "error": s.error.as_ref().map(|e| &e.0)})
    };
    let mut bp = status("buttplug");
    bp["devices"] = json!(
        st.devices
            .buttplug_names
            .lock()
            .unwrap()
            .iter()
            .map(|d| &d.1)
            .collect::<Vec<_>>()
    );
    s["status"] = json!({"tcode": status("tcode"), "buttplug": bp, "vr": vr::json(st)});
    s["profile"] = json!(profiles::active(&s["tcode"]));
    s["sessions"] = sessions_json(st).await;
    s
}

async fn get_devices(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(devices_json(&st).await)
}

/// `{kind: tcode|buttplug|vr|player, settings?: {…merged…}, action?:
/// connect|disconnect, profile?: mirabot|funsr-mini|custom}`. Picking another
/// TCode profile disconnects, swaps the settings and connects the new one
/// (unless it was disconnected last time).
async fn post_devices(State(st): State<Arc<AppState>>, Json(b): Json<Value>) -> Response {
    let Some(kind) = ["tcode", "buttplug", "vr", "player"]
        .into_iter()
        .find(|k| b["kind"] == *k)
    else {
        return (
            StatusCode::BAD_REQUEST,
            "kind: tcode, buttplug, vr or player",
        )
            .into_response();
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
