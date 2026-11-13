//! The HTTP side: one request at a time. `GET /` (the report page),
//! `GET /api/report` (the report, rebuilt from the files on every request),
//! `GET /swipe` (the deck page), `GET /api/deck?limit=N&seed=N&jitter=N&exclude=ids&stack=…`
//! (the next cards),
//! `POST /api/swipe {id, dir}` and `POST /api/swipe/undo {id}` (the only
//! writes: eroscripts-scout-swipes.json).

use crate::swipes::{self, Dir};
use crate::{deck, report};
use serde::Serialize;
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

/// The report page: HTML, CSS and JS in one file, no external scripts.
pub const PAGE: &str = include_str!("page.html");
/// The swipe deck page, likewise self-contained.
pub const SWIPE_PAGE: &str = include_str!("swipe.html");

/// Longest request head read before giving up on a request.
const MAX_HEAD: usize = 16 * 1024;
/// Longest request body accepted (a swipe is a few dozen bytes).
const MAX_BODY: usize = 4 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

fn text(status: u16, body: &str) -> Response {
    Response {
        status,
        content_type: "text/plain; charset=utf-8",
        body: body.as_bytes().to_vec(),
    }
}

fn html(page: &str) -> Response {
    Response {
        status: 200,
        content_type: "text/html; charset=utf-8",
        body: page.as_bytes().to_vec(),
    }
}

fn json_ok<T: Serialize>(value: &T) -> Response {
    match serde_json::to_vec(value) {
        Ok(body) => Response {
            status: 200,
            content_type: "application/json",
            body,
        },
        Err(e) => text(500, &format!("json: {e}\n")),
    }
}

/// Answer one request. HEAD gets GET's headers (the caller drops the body).
pub fn route(method: &str, target: &str, body: &[u8], user_data: &Path) -> Response {
    let target = target.split('#').next().unwrap_or("");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match (method, path) {
        ("GET" | "HEAD", "/") => html(PAGE),
        ("GET" | "HEAD", "/swipe") => html(SWIPE_PAGE),
        ("GET" | "HEAD", "/api/report") => json_ok(&report::load(user_data)),
        ("GET" | "HEAD", "/api/deck") => match deal(query) {
            Ok(d) => json_ok(&deck::load(user_data, &d)),
            Err(e) => text(400, &e),
        },
        ("POST", "/api/swipe") => swipe(body, user_data),
        ("POST", "/api/swipe/undo") => undo(body, user_data),
        _ => text(404, "Not found\n"),
    }
}

/// The deck request from the query: `limit=N` (default DEFAULT_LIMIT, at most MAX_LIMIT),
/// `seed=N` (shuffle a little), `jitter=N` (how much, in score points: 0 ..= MAX_JITTER;
/// default the stack's), `exclude=id,id,…` (cards the page holds; at most MAX_EXCLUDE)
/// and `stack=multi|single` (default multi).
fn deal(query: &str) -> Result<deck::Deal, String> {
    let mut d = deck::Deal::default();
    for (key, v) in query.split('&').filter_map(|p| p.split_once('=')) {
        match key {
            "limit" => {
                d.limit = v
                    .parse::<usize>()
                    .map(|n| n.min(deck::MAX_LIMIT))
                    .map_err(|_| format!("limit must be a whole number, got {v:?}\n"))?;
            }
            "seed" => {
                d.seed = Some(
                    v.parse::<u64>()
                        .map_err(|_| format!("seed must be a whole number, got {v:?}\n"))?,
                );
            }
            "jitter" => {
                let n = v
                    .parse::<f64>()
                    .ok()
                    .filter(|n| (0.0..=deck::MAX_JITTER).contains(n))
                    .ok_or_else(|| {
                        format!(
                            "jitter must be a number from 0 to {}, got {v:?}\n",
                            deck::MAX_JITTER
                        )
                    })?;
                d.jitter = Some(n);
            }
            "exclude" => {
                for id in v.split(',').filter(|s| !s.is_empty()) {
                    let id = id
                        .parse::<u64>()
                        .map_err(|_| format!("exclude takes topic ids, got {id:?}\n"))?;
                    d.exclude.insert(id);
                }
                if d.exclude.len() > deck::MAX_EXCLUDE {
                    return Err(format!("exclude takes at most {} ids\n", deck::MAX_EXCLUDE));
                }
            }
            "stack" => {
                d.stack = deck::Stack::parse(v)
                    .ok_or_else(|| format!("stack is multi or single, got {v:?}\n"))?;
            }
            _ => {}
        }
    }
    Ok(d)
}

/// The JSON object a POST carries, and its topic `id`.
fn swipe_request(body: &[u8]) -> Result<(Value, u64), Response> {
    let req: Value = serde_json::from_slice(body)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| text(400, "body must be a JSON object\n"))?;
    let id = req
        .get("id")
        .and_then(report::id_of)
        .ok_or_else(|| text(400, "id must be a topic id\n"))?;
    Ok((req, id))
}

/// `POST /api/swipe {id, dir}`: record a swipe on a waiting post.
fn swipe(body: &[u8], user_data: &Path) -> Response {
    let (req, id) = match swipe_request(body) {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    let Some(dir) = req.get("dir").and_then(Value::as_str).and_then(Dir::parse) else {
        return text(400, "dir must be \"right\" or \"left\"\n");
    };
    let mut problems = Vec::new();
    let waiting = report::read_json(user_data, report::WAITING_FILE, &mut problems);
    if !problems.is_empty() {
        return text(500, &format!("{}\n", problems.join("\n")));
    }
    if !deck::is_waiting(&waiting, id) {
        return text(400, &format!("{id} is not on the waiting list\n"));
    }
    match swipes::set(user_data, id, dir, video_tags::timefmt::now_iso()) {
        Ok(s) => json_ok(&json!({"id": id, "dir": s.dir, "at": s.at})),
        Err(e) => text(500, &format!("{e}\n")),
    }
}

/// `POST /api/swipe/undo {id}`: forget a swipe (any id; `removed` says whether there was one).
fn undo(body: &[u8], user_data: &Path) -> Response {
    let id = match swipe_request(body) {
        Ok((_, id)) => id,
        Err(resp) => return resp,
    };
    match swipes::undo(user_data, id) {
        Ok(removed) => json_ok(&json!({"id": id, "removed": removed})),
        Err(e) => text(500, &format!("{e}\n")),
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        413 => "Content Too Large",
        _ => "Internal Server Error",
    }
}

/// The request body: `Content-Length` bytes (none without the header), the
/// part already read with the head first, the rest from `more`. Over
/// MAX_BODY → 413 (not read); a bad length or a short body → 400.
fn read_body(head: &str, rest: &[u8], more: &mut impl Read) -> Result<Vec<u8>, Response> {
    let mut len = None;
    for line in head.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("content-length") {
            continue;
        }
        let n: usize = value
            .trim()
            .parse()
            .map_err(|_| text(400, "Bad Content-Length\n"))?;
        if len.is_some_and(|l| l != n) {
            return Err(text(400, "Conflicting Content-Length\n"));
        }
        len = Some(n);
    }
    let len = len.unwrap_or(0);
    if len > MAX_BODY {
        return Err(text(413, "Body over 4 KB\n"));
    }
    let mut body = rest[..rest.len().min(len)].to_vec();
    let start = body.len();
    body.resize(len, 0);
    more.read_exact(&mut body[start..])
        .map_err(|_| text(400, "Body shorter than its Content-Length\n"))?;
    Ok(body)
}

/// Bind `127.0.0.1:<port>` (0 = any free port), or every interface (`0.0.0.0`) when `lan`.
pub fn bind(port: u16, lan: bool) -> io::Result<TcpListener> {
    let ip = if lan {
        Ipv4Addr::UNSPECIFIED
    } else {
        Ipv4Addr::LOCALHOST
    };
    TcpListener::bind((ip, port))
}

/// The Mac's address on the home network: the source address the OS would use to reach
/// a private address (a UDP `connect` sends nothing). None without a network.
pub fn lan_address() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket
        .connect((Ipv4Addr::new(192, 168, 255, 255), 9))
        .ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

/// Serve requests one after another, forever.
pub fn serve(listener: &TcpListener, user_data: &Path, err: &mut dyn Write) {
    for stream in listener.incoming() {
        let result = stream.and_then(|s| handle(s, user_data));
        if let Err(e) = result {
            // A client that went away or timed out; keep serving.
            let _ = writeln!(err, "scout-report: {e}");
        }
    }
}

/// Read the request (head, then body) and write the response.
fn handle(mut stream: TcpStream, user_data: &Path) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        let n = stream.read(&mut buf)?;
        if n == 0 || raw.len() + n > MAX_HEAD {
            break raw.len();
        }
        raw.extend_from_slice(&buf[..n]);
    };
    let head = String::from_utf8_lossy(&raw[..head_end]);
    let rest = raw.get(head_end + 4..).unwrap_or(&[]);
    let line = head.lines().next().unwrap_or("");
    let mut parts = line.split_whitespace();
    let (method, target) = match (parts.next(), parts.next(), parts.next()) {
        (Some(m), Some(t), Some(v)) if v.starts_with("HTTP/") => (m, t),
        _ => ("", ""),
    };
    let resp = if method.is_empty() {
        text(400, "Bad request\n")
    } else {
        match read_body(&head, rest, &mut stream) {
            Ok(body) => route(method, target, &body, user_data),
            Err(resp) => resp,
        }
    };
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        resp.status,
        reason(resp.status),
        resp.content_type,
        resp.body.len()
    )?;
    if method != "HEAD" {
        stream.write_all(&resp.body)?;
    }
    stream.flush()?;
    if resp.status == 413 {
        // The body was never read: drain what the client sends (until it hangs up, the
        // read timeout or 1 MB) so closing with unread data doesn't reset the connection
        // before the client sees the answer.
        stream.shutdown(Shutdown::Write)?;
        let _ = io::copy(&mut (&mut stream).take(1 << 20), &mut io::sink());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn user_data(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("scout-report-route-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(report::STATE_FILE),
            br#"{"version":1,"topics":{"7":{"verdict":"known","rules":1,"at":"2026-11-01T00:00:00Z"}},"feeds":{}}"#,
        )
        .unwrap();
        dir
    }

    fn with_waiting(name: &str) -> std::path::PathBuf {
        let dir = user_data(name);
        std::fs::write(
            dir.join(report::WAITING_FILE),
            br#"{"candidates":[{"id":7,"score":1,"axisType":"multi"},{"id":8,"score":3,"axisType":"multi"},{"id":9,"score":2,"axisType":"multi"}]}"#,
        )
        .unwrap();
        dir
    }

    fn get_json(target: &str, dir: &Path) -> Value {
        let r = route("GET", target, b"", dir);
        assert_eq!(
            (r.status, r.content_type),
            (200, "application/json"),
            "{target}"
        );
        serde_json::from_slice(&r.body).unwrap()
    }

    fn ids(deck: &Value) -> Vec<u64> {
        deck["cards"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_u64().unwrap())
            .collect()
    }

    #[test]
    fn routes_the_pages_and_the_report() {
        let dir = user_data("pages");
        let r = route("GET", "/", b"", &dir);
        assert_eq!(
            (r.status, r.content_type),
            (200, "text/html; charset=utf-8")
        );
        assert_eq!(r.body, PAGE.as_bytes());
        assert_eq!(route("HEAD", "/?x=1", b"", &dir).status, 200);
        let r = route("GET", "/swipe#top", b"", &dir);
        assert_eq!(
            (r.status, r.content_type),
            (200, "text/html; charset=utf-8")
        );
        assert_eq!(r.body, SWIPE_PAGE.as_bytes());

        let v = get_json("/api/report?t=123", &dir);
        assert_eq!(v["posts"][0]["id"], 7);
        assert_eq!(v["posts"][0]["url"], "https://discuss.eroscripts.com/t/7");
        assert_eq!(v["verdicts"]["known"], 1);

        // The report is rebuilt on every request.
        std::fs::remove_file(dir.join(report::STATE_FILE)).unwrap();
        let v = get_json("/api/report", &dir);
        assert_eq!(v["posts"], serde_json::json!([]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deck_limits() {
        let dir = with_waiting("deck");
        let v = get_json("/api/deck", &dir);
        assert_eq!(ids(&v), [8, 9, 7]);
        assert_eq!(
            (v["remaining"].as_u64(), v["wanted"].as_u64()),
            (Some(3), Some(0))
        );
        assert_eq!(ids(&get_json("/api/deck?limit=1", &dir)), [8]);
        assert_eq!(
            ids(&get_json("/api/deck?x=1&limit=0", &dir)),
            [] as [u64; 0]
        );
        assert_eq!(get_json("/api/deck?limit=100000", &dir)["remaining"], 3);
        // Cards the page holds are left out; the count still covers them.
        let v = get_json("/api/deck?exclude=8,7&limit=5", &dir);
        assert_eq!((ids(&v), v["remaining"].as_u64()), (vec![9], Some(3)));
        for bad in [
            "/api/deck?limit=",
            "/api/deck?limit=-1",
            "/api/deck?limit=x",
            "/api/deck?seed=-3",
            "/api/deck?exclude=8,x",
            "/api/deck?stack=both",
            "/api/deck?stack=mixed",
            "/api/deck?jitter=",
            "/api/deck?jitter=31",
            "/api/deck?jitter=30.01",
            "/api/deck?jitter=-1",
            "/api/deck?jitter=x",
            "/api/deck?jitter=NaN",
            "/api/deck?jitter=inf",
        ] {
            assert_eq!(route("GET", bad, b"", &dir).status, 400, "{bad}");
        }
        let many = (0..=deck::MAX_EXCLUDE)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            route("GET", &format!("/api/deck?exclude={many}"), b"", &dir).status,
            400
        );
        assert_eq!(deal("limit=100000").map(|d| d.limit), Ok(deck::MAX_LIMIT));
        assert_eq!(deal(""), Ok(deck::Deal::default()));
        assert_eq!(
            deal("seed=9&exclude=").map(|d| (d.seed, d.exclude.len())),
            Ok((Some(9), 0))
        );
        // jitter: 0 ..= MAX_JITTER score points; 0 deals strictly by score, seed or not.
        assert_eq!(deal("jitter=0").map(|d| d.jitter), Ok(Some(0.0)));
        assert_eq!(deal("jitter=2.5").map(|d| d.jitter), Ok(Some(2.5)));
        assert_eq!(
            deal("jitter=30").map(|d| d.jitter),
            Ok(Some(deck::MAX_JITTER))
        );
        assert_eq!(deal("seed=1").map(|d| d.jitter), Ok(None));
        assert_eq!(
            ids(&get_json("/api/deck?seed=12345&jitter=0", &dir)),
            [8, 9, 7]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn swipe_and_undo() {
        let dir = with_waiting("swipe");
        let r = route("POST", "/api/swipe", br#"{"id":8,"dir":"right"}"#, &dir);
        assert_eq!(r.status, 200);
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(
            (v["id"].as_u64(), v["dir"].as_str()),
            (Some(8), Some("right"))
        );
        assert!(v["at"].as_str().unwrap().ends_with('Z'));
        // A numeric string id works too.
        let r = route("POST", "/api/swipe", br#"{"id":"9","dir":"left"}"#, &dir);
        assert_eq!(r.status, 200);

        let v = get_json("/api/deck", &dir);
        assert_eq!(ids(&v), [7]);
        assert_eq!(
            (v["remaining"].as_u64(), v["wanted"].as_u64()),
            (Some(1), Some(1))
        );
        let s = swipes::read(&dir).unwrap();
        assert_eq!((s.dir(8), s.dir(9)), (Some(Dir::Right), Some(Dir::Left)));
        let report = get_json("/api/report", &dir);
        let post = |id: u64| {
            report["posts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == id)
                .unwrap()
                .clone()
        };
        assert_eq!(post(8)["swipe"], "right");
        assert_eq!(post(9)["swipe"], "left");
        assert!(post(7).get("swipe").is_none());

        let r = route("POST", "/api/swipe/undo", br#"{"id":8}"#, &dir);
        assert_eq!(r.status, 200);
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v, serde_json::json!({"id": 8, "removed": true}));
        let r = route("POST", "/api/swipe/undo", br#"{"id":8}"#, &dir);
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v["removed"], false);
        assert_eq!(ids(&get_json("/api/deck", &dir)), [8, 7]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_swipes_are_rejected() {
        let dir = with_waiting("bad");
        for (path, body) in [
            ("/api/swipe", &b""[..]),
            ("/api/swipe", b"{not json"),
            ("/api/swipe", b"[8, \"right\"]"),
            ("/api/swipe", br#"{"dir":"right"}"#),
            ("/api/swipe", br#"{"id":"eight","dir":"right"}"#),
            ("/api/swipe", br#"{"id":-8,"dir":"right"}"#),
            ("/api/swipe", br#"{"id":8}"#),
            ("/api/swipe", br#"{"id":8,"dir":"up"}"#),
            ("/api/swipe", br#"{"id":8,"dir":"RIGHT"}"#),
            ("/api/swipe", br#"{"id":99,"dir":"right"}"#),
            ("/api/swipe/undo", b""),
            ("/api/swipe/undo", b"{}"),
            ("/api/swipe/undo", br#"{"id":null}"#),
        ] {
            let r = route("POST", path, body, &dir);
            assert_eq!(r.status, 400, "{path} {}", String::from_utf8_lossy(body));
        }
        assert!(!dir.join(swipes::FILE).exists(), "nothing written");

        // A broken swipes file is never overwritten.
        std::fs::write(dir.join(swipes::FILE), b"oops").unwrap();
        let r = route("POST", "/api/swipe", br#"{"id":8,"dir":"right"}"#, &dir);
        assert_eq!(r.status, 500);
        assert!(String::from_utf8_lossy(&r.body).starts_with(swipes::FILE));
        assert_eq!(
            route("POST", "/api/swipe/undo", br#"{"id":8}"#, &dir).status,
            500
        );
        assert_eq!(std::fs::read(dir.join(swipes::FILE)).unwrap(), b"oops");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_bodies_by_content_length() {
        let head =
            |len: &str| format!("POST /api/swipe HTTP/1.1\r\nHost: x\r\nContent-length: {len}");
        let mut none = io::empty();
        // Part with the head, the rest from the stream; extra bytes ignored.
        let mut more = io::Cursor::new(b"lo!!".to_vec());
        assert_eq!(read_body(&head("5"), b"hel", &mut more).unwrap(), b"hello");
        assert_eq!(read_body(&head("2"), b"hello", &mut none).unwrap(), b"he");
        assert_eq!(
            read_body("GET / HTTP/1.1\r\nHost: x", b"", &mut none).unwrap(),
            b""
        );
        assert_eq!(
            read_body(&head("4096"), &[b'x'; 4096], &mut none)
                .unwrap()
                .len(),
            4096
        );
        let status = |r: Result<Vec<u8>, Response>| r.unwrap_err().status;
        assert_eq!(status(read_body(&head("4097"), b"", &mut none)), 413);
        assert_eq!(
            status(read_body(&head("99999999999999999999999"), b"", &mut none)),
            400
        );
        assert_eq!(status(read_body(&head("x"), b"", &mut none)), 400);
        assert_eq!(status(read_body(&head("-1"), b"", &mut none)), 400);
        assert_eq!(status(read_body(&head("10"), b"short", &mut none)), 400);
        let twice = format!("{}\r\nContent-Length: 3", head("2"));
        assert_eq!(status(read_body(&twice, b"abc", &mut none)), 400);
    }

    #[test]
    fn everything_else_is_not_found() {
        let dir = std::env::temp_dir();
        for (m, t) in [
            ("GET", "/index.html"),
            ("GET", "/api/report/"),
            ("GET", "/api"),
            ("GET", "/../eroscripts-scout.json"),
            ("GET", "/eroscripts-scout-state.json"),
            ("GET", "/eroscripts-scout-swipes.json"),
            ("GET", "/api/swipe"),
            ("GET", "/swipe/"),
            ("POST", "/api/report"),
            ("POST", "/api/deck"),
            ("PUT", "/api/swipe"),
            ("DELETE", "/"),
        ] {
            assert_eq!(route(m, t, b"", &dir).status, 404, "{m} {t}");
        }
    }

    #[test]
    fn binds_loopback_unless_lan() {
        let l = bind(0, false).unwrap();
        assert!(l.local_addr().unwrap().ip().is_loopback());
        let l = bind(0, true).unwrap();
        assert!(l.local_addr().unwrap().ip().is_unspecified());
    }
}
