//! Warms the thumbnail cache: asks the running app's backend for each
//! registered video's thumbnail (`GET /api/media/thumb/<id>`), which makes it
//! generate and cache the missing ones. One request at a time: the backend's
//! generator is the bottleneck. Raw HTTP/1.1 over a TcpStream, to the given
//! backend only.

pub mod cli;

use serde_json::Value;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};
use video_tags::host::HttpBase;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// The video list.
const LIST_TIMEOUT: Duration = Duration::from_secs(60);
/// One thumbnail: the backend gives up after `THUMB_WAIT_SEC` (30 s,
/// backend/routes/media.py) and answers 503; this leaves room for that.
const THUMB_TIMEOUT: Duration = Duration::from_secs(40);

/// Why a request failed: nobody answered (the app isn't running), or the
/// answer was unusable (timeout, cut short).
#[derive(Debug, PartialEq)]
pub enum Error {
    Down(String),
    Bad(String),
}

/// One registered video, as `GET /api/remote/videos` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Video {
    /// The backend's registry id, the one `/api/media/thumb/<id>` takes.
    pub id: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct Backend {
    host: String,
    addr: SocketAddr,
}

impl Backend {
    /// From `http://host:port` (a trailing `/` is fine).
    pub fn new(url: &str) -> Result<Self, String> {
        let base = HttpBase::parse(url).map_err(|e| format!("backend {e}"))?;
        let addr = base.resolve().map_err(|e| format!("backend {url}: {e}"))?;
        Ok(Backend {
            host: base.host,
            addr,
        })
    }

    /// `GET /api/remote/videos`.
    pub fn videos(&self) -> Result<Vec<Video>, Error> {
        let path = "/api/remote/videos";
        let (status, body) = self.get(path, LIST_TIMEOUT)?;
        if status != 200 {
            return Err(Error::Bad(format!("GET {path}: HTTP {status}")));
        }
        let v: Value = serde_json::from_slice(&body)
            .map_err(|e| Error::Bad(format!("GET {path}: not JSON ({e})")))?;
        parse_videos(&v).map_err(|e| Error::Bad(format!("GET {path}: {e}")))
    }

    /// `GET /api/media/thumb/<id>`: the HTTP status (the image is read and
    /// dropped).
    pub fn thumb(&self, id: &str) -> Result<u16, Error> {
        self.get(&format!("/api/media/thumb/{}", encode(id)), THUMB_TIMEOUT)
            .map(|(status, _)| status)
    }

    fn get(&self, path: &str, timeout: Duration) -> Result<(u16, Vec<u8>), Error> {
        let addr = self.addr;
        let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
            .map_err(|e| Error::Down(format!("backend {addr}: {e}")))?;
        let bad = |e: std::io::Error| Error::Bad(format!("GET {path}: {e}"));
        stream.set_read_timeout(Some(timeout)).map_err(bad)?;
        stream.set_write_timeout(Some(timeout)).map_err(bad)?;
        let head = format!(
            "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            self.host
        );
        stream.write_all(head.as_bytes()).map_err(bad)?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).map_err(bad)?;
        parse_response(&raw).map_err(|e| Error::Bad(format!("GET {path}: {e}")))
    }
}

/// Percent-encode everything but RFC 3986 unreserved characters.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
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
/// are left out; a number id is used as its text.
pub fn parse_videos(v: &Value) -> Result<Vec<Video>, String> {
    let list = v
        .get("videos")
        .unwrap_or(v)
        .as_array()
        .ok_or("expected a list of videos")?;
    Ok(list
        .iter()
        .filter_map(|e| {
            let id = match e.get("id")? {
                Value::String(s) if !s.is_empty() => s.clone(),
                n @ Value::Number(_) => n.to_string(),
                _ => return None,
            };
            let path = e.get("path").and_then(Value::as_str).unwrap_or("");
            Some(Video {
                id,
                path: path.to_string(),
            })
        })
        .collect())
}

/// What one thumbnail request came to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    Ok,
    /// 404: the backend doesn't know the video or its file is gone.
    Missing,
    Failed,
}

pub fn outcome(status: u16) -> Outcome {
    match status {
        200..=299 => Outcome::Ok,
        404 => Outcome::Missing,
        _ => Outcome::Failed,
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Counts {
    pub ok: usize,
    pub failed: usize,
    pub missing: usize,
}

impl Counts {
    pub fn add(&mut self, o: Outcome) {
        match o {
            Outcome::Ok => self.ok += 1,
            Outcome::Missing => self.missing += 1,
            Outcome::Failed => self.failed += 1,
        }
    }

    /// 0 when nothing failed (missing files are skipped), else 2.
    pub fn exit_code(&self) -> i32 {
        if self.failed == 0 { 0 } else { 2 }
    }

    pub fn summary(&self, elapsed: Duration) -> String {
        format!(
            "Thumbnails: {} ok, {} failed, {} missing (of {}) in {:.1}s",
            self.ok,
            self.failed,
            self.missing,
            self.ok + self.failed + self.missing,
            elapsed.as_secs_f64()
        )
    }
}

pub fn run(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let opts = match cli::parse(args) {
        Ok(o) => o,
        Err(e) => {
            let _ = writeln!(err, "{e}");
            return 1;
        }
    };
    if opts.help {
        let _ = write!(out, "{}", cli::USAGE);
        return 0;
    }
    match warm(&opts, out, err) {
        Ok(code) => code,
        Err(e) => {
            let _ = writeln!(err, "{e}");
            1
        }
    }
}

fn warm(opts: &cli::Options, out: &mut dyn Write, err: &mut dyn Write) -> Result<i32, String> {
    let url = match &opts.backend {
        Some(u) => u.trim_end_matches('/').to_string(),
        None => {
            let user_data = match &opts.user_data {
                Some(d) => d.clone(),
                None => video_tags::host::default_user_data()
                    .ok_or("no userData folder (give --user-data)")?,
            };
            let port = video_tags::config::read(&user_data)?.backend_port;
            format!("http://127.0.0.1:{port}")
        }
    };
    let backend = Backend::new(&url)?;
    let down = || format!("The FunSync app isn't running (no backend at {url})");
    let started = Instant::now();
    let mut videos = match backend.videos() {
        Ok(v) => v,
        Err(Error::Down(_)) => return Err(down()),
        Err(Error::Bad(e)) => return Err(e),
    };
    if let Some(n) = opts.limit {
        videos.truncate(n);
    }
    let mut counts = Counts::default();
    for v in &videos {
        let name = if v.path.is_empty() { &v.id } else { &v.path };
        let o = match backend.thumb(&v.id) {
            Ok(status) => {
                let o = outcome(status);
                if o != Outcome::Ok {
                    let _ = writeln!(err, "{name}: HTTP {status}");
                }
                o
            }
            // The app quit mid-run: the rest would fail the same way.
            Err(Error::Down(_)) => return Err(down()),
            Err(Error::Bad(e)) => {
                let _ = writeln!(err, "{name}: {e}");
                Outcome::Failed
            }
        };
        counts.add(o);
    }
    let _ = writeln!(out, "{}", counts.summary(started.elapsed()));
    Ok(counts.exit_code())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_wrapped_and_bare_video_lists() {
        let want = vec![
            Video {
                id: "ab12".into(),
                path: "/V/a.mp4".into(),
            },
            Video {
                id: "7".into(),
                path: String::new(),
            },
        ];
        let list = json!([{ "id": "ab12", "path": "/V/a.mp4" }, { "id": 7 }, { "path": "/p" }, { "id": "" }]);
        assert_eq!(parse_videos(&json!({ "videos": list })).unwrap(), want);
        assert_eq!(parse_videos(&list).unwrap(), want);
        assert!(parse_videos(&json!({ "nope": 1 })).is_err());
    }

    #[test]
    fn parses_plain_and_chunked_responses() {
        let plain = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}extra";
        assert_eq!(parse_response(plain).unwrap(), (200, b"{}".to_vec()));
        let chunked = b"HTTP/1.1 503 Service Unavailable\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;x=y\r\nde\r\n0\r\n\r\n";
        assert_eq!(parse_response(chunked).unwrap(), (503, b"abcde".to_vec()));
        assert!(parse_response(b"HTTP/1.1 200 OK\r\ncontent-length: 9\r\n\r\n{}").is_err());
    }

    #[test]
    fn counts_and_exit_code() {
        let mut c = Counts::default();
        c.add(outcome(200));
        c.add(outcome(404));
        assert_eq!(c.exit_code(), 0);
        c.add(outcome(503));
        c.add(outcome(500));
        assert_eq!(
            c,
            Counts {
                ok: 1,
                failed: 2,
                missing: 1
            }
        );
        assert_eq!(c.exit_code(), 2);
        assert_eq!(
            c.summary(Duration::from_millis(1300)),
            "Thumbnails: 1 ok, 2 failed, 1 missing (of 4) in 1.3s"
        );
    }

    #[test]
    fn encodes_ids_for_the_path() {
        assert_eq!(encode("ab-12_x.y~"), "ab-12_x.y~");
        assert_eq!(encode("a b/c"), "a%20b%2Fc");
    }
}
