//! The running app's backend (`--backend`, default `http://127.0.0.1:5123`):
//! the video list and VR flags it serves, and the remote edits (rating,
//! playlist) the desktop applies. Raw HTTP/1.1 over a TcpStream: a few local
//! requests need no HTTP crate.

use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

pub const DEFAULT_URL: &str = "http://127.0.0.1:5123";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const IO_TIMEOUT: Duration = Duration::from_secs(60);

/// Why a request failed: nobody answered (the app isn't running), or the
/// backend answered with something unusable.
#[derive(Debug, PartialEq)]
pub enum Error {
    Down(String),
    Bad(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Down(m) | Error::Bad(m) => f.write_str(m),
        }
    }
}

/// One registered video, as `GET /api/remote/videos` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Video {
    /// The backend's id, as it sent it (string or number).
    pub id: Value,
    pub path: String,
    pub favorite: bool,
    pub rating: f64,
    /// Seconds; 0 when not known yet.
    pub duration: f64,
}

impl Video {
    /// The id as text (file names, manifests, results).
    pub fn id_text(&self) -> String {
        id_text(&self.id)
    }
}

pub fn id_text(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[derive(Debug, Clone)]
pub struct Backend {
    host: String,
    addr: SocketAddr,
}

impl Backend {
    /// From `http://host:port` (a trailing `/` is fine).
    pub fn new(url: &str) -> Result<Self, String> {
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| format!("--backend {url}: only http://host:port is supported"))?;
        let host = rest.trim_end_matches('/');
        if host.is_empty() || host.contains('/') {
            return Err(format!(
                "--backend {url}: give http://host:port without a path"
            ));
        }
        let with_port = if host
            .rsplit_once(':')
            .is_some_and(|(_, p)| p.parse::<u16>().is_ok())
        {
            host.to_string()
        } else {
            format!("{host}:80")
        };
        let addr = with_port
            .to_socket_addrs()
            .map_err(|e| format!("--backend {url}: {e}"))?
            .next()
            .ok_or_else(|| format!("--backend {url}: no address"))?;
        Ok(Backend {
            host: host.to_string(),
            addr,
        })
    }

    /// `GET /api/remote/videos`: `{videos: [...]}` or a bare list.
    pub fn videos(&self) -> Result<Vec<Video>, Error> {
        let v = self.get("/api/remote/videos")?;
        parse_videos(&v).map_err(|e| Error::Bad(format!("/api/remote/videos: {e}")))
    }

    /// `GET /api/media/vr-flags`: path → is VR (videos still pending are
    /// missing, i.e. flat).
    pub fn vr_flags(&self) -> Result<HashMap<String, bool>, Error> {
        let v = self.get("/api/media/vr-flags")?;
        Ok(v.get("flags")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| Some((k.clone(), v.as_bool()?)))
            .collect())
    }

    /// `POST /api/remote/rating {videoId, rating}`.
    pub fn rate(&self, id: &Value, rating: f64) -> Result<(), Error> {
        self.post(
            "/api/remote/rating",
            &serde_json::json!({ "videoId": id, "rating": rating }),
        )
        .map(drop)
    }

    /// `POST /api/remote/playlists/add-video {videoId, name}` (the desktop
    /// creates the playlist when no playlist has that name).
    pub fn add_to_playlist(&self, id: &Value, name: &str) -> Result<(), Error> {
        self.post(
            "/api/remote/playlists/add-video",
            &serde_json::json!({ "videoId": id, "name": name }),
        )
        .map(drop)
    }

    fn get(&self, path: &str) -> Result<Value, Error> {
        self.request("GET", path, None)
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value, Error> {
        self.request("POST", path, Some(body.to_string()))
    }

    fn request(&self, method: &str, path: &str, body: Option<String>) -> Result<Value, Error> {
        let addr = self.addr;
        let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
            .map_err(|e| Error::Down(format!("backend {addr}: {e} (is FunSync running?)")))?;
        let bad = |e: std::io::Error| Error::Bad(format!("{method} {path}: {e}"));
        stream.set_read_timeout(Some(IO_TIMEOUT)).map_err(bad)?;
        stream.set_write_timeout(Some(IO_TIMEOUT)).map_err(bad)?;
        let body = body.unwrap_or_default();
        let content_type = if body.is_empty() {
            ""
        } else {
            "Content-Type: application/json\r\n"
        };
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\n{content_type}Content-Length: {}\r\nConnection: close\r\n\r\n",
            self.host,
            body.len()
        );
        stream.write_all(head.as_bytes()).map_err(bad)?;
        stream.write_all(body.as_bytes()).map_err(bad)?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).map_err(bad)?;
        let (status, payload) =
            parse_response(&raw).map_err(|e| Error::Bad(format!("{method} {path}: {e}")))?;
        if !(200..300).contains(&status) {
            let detail = String::from_utf8_lossy(&payload);
            let detail: String = detail.chars().take(200).collect();
            return Err(Error::Bad(format!(
                "{method} {path}: HTTP {status} {detail}"
            )));
        }
        if payload.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&payload)
            .map_err(|e| Error::Bad(format!("{method} {path}: not JSON ({e})")))
    }
}

/// Split a whole HTTP/1.1 response into status and body (de-chunked).
pub fn parse_response(raw: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("no answer")?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let body = &raw[split + 4..];
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or("no status line")?;
    let mut chunked = false;
    let mut length = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            if k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked") {
                chunked = true;
            } else if k == "content-length" {
                length = v.parse::<usize>().ok();
            }
        }
    }
    if chunked {
        return dechunk(body).map(|b| (status, b));
    }
    let body = match length {
        Some(n) if n <= body.len() => body[..n].to_vec(),
        Some(_) => return Err("answer cut short".into()),
        None => body.to_vec(),
    };
    Ok((status, body))
}

fn dechunk(mut body: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(body.len());
    loop {
        let eol = body
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("bad chunk")?;
        let size_text = String::from_utf8_lossy(&body[..eol]);
        let size_text = size_text.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| "bad chunk size")?;
        body = &body[eol + 2..];
        if size == 0 {
            return Ok(out);
        }
        if body.len() < size {
            return Err("answer cut short".into());
        }
        out.extend_from_slice(&body[..size]);
        body = body.get(size + 2..).ok_or("answer cut short")?;
    }
}

/// The video list: `{videos: [...]}` or a bare list. Entries without an id
/// or a path are left out.
pub fn parse_videos(v: &Value) -> Result<Vec<Video>, String> {
    let list = v
        .get("videos")
        .unwrap_or(v)
        .as_array()
        .ok_or("expected a list of videos")?;
    Ok(list
        .iter()
        .filter_map(|e| {
            let id = e.get("id").filter(|i| i.is_string() || i.is_number())?;
            let path = e.get("path")?.as_str().filter(|p| !p.is_empty())?;
            Some(Video {
                id: id.clone(),
                path: path.to_string(),
                favorite: e.get("favorite").and_then(Value::as_bool).unwrap_or(false),
                rating: e.get("rating").and_then(Value::as_f64).unwrap_or(0.0),
                duration: e.get("duration").and_then(Value::as_f64).unwrap_or(0.0),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_wrapped_and_bare_video_lists() {
        let entry = json!({ "id": "ab12", "path": "/V/a.mp4", "favorite": true, "rating": 1.5, "duration": 61.2 });
        let want = vec![Video {
            id: json!("ab12"),
            path: "/V/a.mp4".into(),
            favorite: true,
            rating: 1.5,
            duration: 61.2,
        }];
        assert_eq!(parse_videos(&json!({ "videos": [entry] })).unwrap(), want);
        assert_eq!(
            parse_videos(&json!([entry, { "id": "x" }, { "path": "/p" }])).unwrap(),
            want
        );
        assert!(parse_videos(&json!({ "nope": 1 })).is_err());
    }

    #[test]
    fn parses_plain_and_chunked_responses() {
        let plain = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}extra";
        assert_eq!(parse_response(plain).unwrap(), (200, b"{}".to_vec()));
        let chunked = b"HTTP/1.1 404 Not Found\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;x=y\r\nde\r\n0\r\n\r\n";
        assert_eq!(parse_response(chunked).unwrap(), (404, b"abcde".to_vec()));
        assert!(parse_response(b"HTTP/1.1 200 OK\r\ncontent-length: 9\r\n\r\n{}").is_err());
    }

    #[test]
    fn a_closed_port_is_down() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let b = Backend::new(&format!("http://127.0.0.1:{port}/")).unwrap();
        assert!(matches!(b.videos(), Err(Error::Down(_))));
        assert!(Backend::new("https://x:1").is_err());
        assert!(Backend::new("http://127.0.0.1:5123/api").is_err());
    }
}
