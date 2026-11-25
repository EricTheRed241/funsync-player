//! PCVR companion bridge (`electron/main.js` `vr-connect` +
//! `renderer/js/vr-bridge.js`): a TCP client to HereSphere / DeoVR's
//! timestamp server (port 23554). Packets are a 4-byte little-endian length
//! and JSON (`path`, `currentTime` s, `playbackSpeed`, `playerState` 0 =
//! playing, 1 = paused); four zero bytes every second keep it open. The
//! headset's video is looked up in the library and played on the engine.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::data::now_ms;
use crate::scan::Video;
use crate::server::AppState;

/// No packet for this long drops the link (the old app's liveness timeout).
const LIVENESS: Duration = Duration::from_secs(8);
const MEDIA_EXT: [&str; 18] = [
    "mp4", "mkv", "webm", "avi", "mov", "wmv", "flv", "m4v", "mp3", "wav", "ogg", "flac", "aac",
    "m4a", "3gp", "ts", "mts", "m2ts",
];

#[derive(Default)]
pub struct Link {
    pub host: String,
    pub video: Option<String>,
    pub last_packet: Option<u64>,
    deltas: VecDeque<u64>,
}

impl Link {
    /// Spread of the packet arrival intervals (≈ network jitter), ms.
    fn jitter(&self) -> Option<u64> {
        if self.deltas.len() < 4 {
            return None;
        }
        let n = self.deltas.len() as f64;
        let mean = self.deltas.iter().sum::<u64>() as f64 / n;
        let var = self
            .deltas
            .iter()
            .map(|d| (*d as f64 - mean).powi(2))
            .sum::<f64>()
            / n;
        Some(var.sqrt().round() as u64)
    }
}

/// `vrBridge` of `/api/status` and the dashboard's `vr`.
pub fn json(st: &AppState) -> Value {
    let d = &st.devices;
    let (connected, error) = {
        let s = d.status("vr").lock().unwrap();
        (s.connected, s.error.as_ref().map(|e| e.0.clone()))
    };
    let l = d.vr_link.lock().unwrap();
    let age = l
        .last_packet
        .filter(|_| connected)
        .map(|t| now_ms().saturating_sub(t));
    json!({
        "connected": connected,
        "linkState": match age {
            None if !connected => "disconnected",
            Some(a) if a < 5000 => "receiving",
            _ => "waiting",
        },
        "player": "heresphere",
        "host": l.host,
        "lastPacketAgeMs": age,
        "jitterMs": l.jitter(),
        "video": l.video,
        "lastError": error,
    })
}

/// The headset's path or URL as a comparable name (`vr-bridge.js`
/// `_normalizePath`): file name, URL-decoded, without media extension,
/// lower case, `_ . -` as spaces.
fn normalize(raw: &str) -> String {
    let path = match raw.split_once("://") {
        Some((_, rest)) => rest.split_once('/').map_or("", |x| x.1),
        None => raw,
    };
    let path = path.split(['?', '#']).next().unwrap_or("");
    let name = path.rsplit(['/', '\\']).next().unwrap_or("");
    let (b, mut bytes, mut i) = (name.as_bytes(), Vec::new(), 0);
    while i < b.len() {
        let hex = b
            .get(i + 1..i + 3)
            .and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok());
        match (b[i], hex) {
            (b'%', Some(x)) => {
                bytes.push(x);
                i += 3;
            }
            (c, _) => {
                bytes.push(c);
                i += 1;
            }
        }
    }
    let name = String::from_utf8_lossy(&bytes).to_lowercase();
    let name = match name.rsplit_once('.') {
        Some((stem, ext)) if MEDIA_EXT.contains(&ext) => stem.to_string(),
        _ => name,
    };
    name.replace(['_', '.', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The library video the headset plays: this server's stream URL, else
/// the same normalized file name.
pub fn find<'a>(videos: &'a [Video], raw: &str) -> Option<&'a Video> {
    if let Some((_, rest)) = raw.split_once("/api/media/stream/") {
        let id = rest.split(['?', '/', '#']).next().unwrap_or("");
        return videos.iter().find(|v| v.id == id);
    }
    let want = normalize(raw);
    videos.iter().find(|v| normalize(&v.name) == want)
}

async fn packet(r: &mut tokio::net::tcp::OwnedReadHalf) -> Result<Value, String> {
    let len = r.read_u32_le().await.map_err(|e| e.to_string())?;
    let mut buf = vec![0; len as usize];
    r.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
    Ok(serde_json::from_slice(&buf).unwrap_or_default())
}

/// Connect to `settings.vr` (`lastHost`, `lastPort`) and follow the headset
/// until the link drops; the bridge owns the engine from its first packet.
pub async fn session(
    st: &Arc<AppState>,
    cfg: &Value,
    connected: impl FnOnce(),
) -> Result<(), String> {
    let host = cfg["lastHost"].as_str().unwrap_or("127.0.0.1").to_string();
    let port = cfg["lastPort"].as_u64().unwrap_or(23554) as u16;
    let sock = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await
    .map_err(|_| "connect timed out".to_string())?
    .map_err(|e| e.to_string())?;
    let (mut r, mut w) = sock.into_split();
    // Stops with this function, also when the bridge task is aborted.
    struct Abort(tokio::task::JoinHandle<()>);
    impl Drop for Abort {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _keepalive = Abort(tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_secs(1));
        loop {
            iv.tick().await;
            if w.write_all(&[0; 4]).await.is_err() {
                return;
            }
        }
    }));
    *st.devices.vr_link.lock().unwrap() = Link {
        host: format!("{host}:{port}"),
        ..Link::default()
    };
    connected();
    let d = &st.devices;
    let n = d.sessions.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let offset = cfg["offset"].as_f64().unwrap_or(0.0);
    let (mut begun, mut path) = (false, String::new());
    let res = loop {
        let v = match tokio::time::timeout(LIVENESS, packet(&mut r)).await {
            Err(_) => break Err("no packets for 8 s".to_string()),
            Ok(Err(e)) => break Err(e),
            Ok(Ok(v)) => v,
        };
        let now = now_ms();
        {
            let mut l = d.vr_link.lock().unwrap();
            if let Some(prev) = l.last_packet.replace(now)
                && (1..500).contains(&(now - prev))
            {
                l.deltas.push_back(now - prev);
                if l.deltas.len() > 20 {
                    l.deltas.pop_front();
                }
            }
        }
        if !begun {
            begun = true;
            super::session_begin(st, n, "vr", host.clone()).await;
        }
        let raw = v["path"].as_str().unwrap_or("");
        if !raw.is_empty() && raw != path {
            path = raw.to_string();
            let video = find(&st.snapshot.read().await.videos, raw).cloned();
            d.vr_link.lock().unwrap().video =
                Some(video.as_ref().map_or(raw.to_string(), |v| v.name.clone()));
            super::load_play(st, n, "vr", video.as_ref(), None).await;
        }
        if let Some(p) = d.play.lock().unwrap().as_mut().filter(|p| p.session == n) {
            p.at = v["currentTime"].as_f64().unwrap_or(0.0) * 1000.0 + offset;
            p.paused = v["playerState"].as_i64().unwrap_or(1) == 1;
            p.rate = v["playbackSpeed"].as_f64().unwrap_or(1.0);
            p.stamp = Instant::now();
        }
    };
    super::session_end(st, n).await;
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headset_paths_match_names() {
        assert_eq!(normalize("C:\\VR\\My_Scene-4K.mp4"), "my scene 4k");
        assert_eq!(
            normalize("http://192.168.1.2:5300/files/My%20Scene.4K.MP4?x=1"),
            "my scene 4k"
        );
        assert_eq!(normalize("/a/b/Scene.funscript"), "scene funscript");
    }
}
