//! The HTTP side: a loopback-only listener answering one request at a time
//! with `GET /` (the page) and `GET /api/report` (the report, rebuilt from the
//! files on every request).

use crate::report;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

/// The page: HTML, CSS and JS in one file, no external scripts.
pub const PAGE: &str = include_str!("page.html");

/// Longest request head read before giving up on a request.
const MAX_HEAD: usize = 16 * 1024;

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

/// Answer one request. HEAD gets GET's headers (the caller drops the body).
pub fn route(method: &str, target: &str, user_data: &Path) -> Response {
    let path = target.split(['?', '#']).next().unwrap_or("");
    match (method, path) {
        ("GET" | "HEAD", "/") => Response {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: PAGE.as_bytes().to_vec(),
        },
        ("GET" | "HEAD", "/api/report") => match serde_json::to_vec(&report::load(user_data)) {
            Ok(body) => Response {
                status: 200,
                content_type: "application/json",
                body,
            },
            Err(e) => text(500, &format!("report: {e}\n")),
        },
        _ => text(404, "Not found\n"),
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Internal Server Error",
    }
}

/// Bind `127.0.0.1:<port>` (0 = any free port). Never another interface.
pub fn bind(port: u16) -> io::Result<TcpListener> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port))
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

/// Read the request head and write the response.
fn handle(mut stream: TcpStream, user_data: &Path) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut head = Vec::new();
    let mut buf = [0u8; 4096];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut buf)?;
        if n == 0 || head.len() + n > MAX_HEAD {
            break;
        }
        head.extend_from_slice(&buf[..n]);
    }
    let line = head
        .split(|&b| b == b'\r' || b == b'\n')
        .next()
        .unwrap_or(&[]);
    let line = String::from_utf8_lossy(line);
    let mut parts = line.split_whitespace();
    let (method, target) = match (parts.next(), parts.next(), parts.next()) {
        (Some(m), Some(t), Some(v)) if v.starts_with("HTTP/") => (m, t),
        _ => ("", ""),
    };
    let resp = if method.is_empty() {
        text(400, "Bad request\n")
    } else {
        route(method, target, user_data)
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
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn user_data() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("scout-report-route-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(report::STATE_FILE),
            br#"{"version":1,"topics":{"7":{"verdict":"known","rules":1,"at":"2026-11-01T00:00:00Z"}},"feeds":{}}"#,
        )
        .unwrap();
        dir
    }

    #[test]
    fn routes_the_page_and_the_report() {
        let dir = user_data();
        let r = route("GET", "/", &dir);
        assert_eq!(
            (r.status, r.content_type),
            (200, "text/html; charset=utf-8")
        );
        assert_eq!(r.body, PAGE.as_bytes());
        assert_eq!(route("HEAD", "/?x=1", &dir).status, 200);

        let r = route("GET", "/api/report?t=123", &dir);
        assert_eq!((r.status, r.content_type), (200, "application/json"));
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v["posts"][0]["id"], 7);
        assert_eq!(v["posts"][0]["url"], "https://discuss.eroscripts.com/t/7");
        assert_eq!(v["verdicts"]["known"], 1);

        // The report is rebuilt on every request.
        std::fs::remove_file(dir.join(report::STATE_FILE)).unwrap();
        let v: Value = serde_json::from_slice(&route("GET", "/api/report", &dir).body).unwrap();
        assert_eq!(v["posts"], serde_json::json!([]));
        let _ = std::fs::remove_dir_all(&dir);
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
            ("POST", "/api/report"),
            ("DELETE", "/"),
        ] {
            assert_eq!(route(m, t, &dir).status, 404, "{m} {t}");
        }
    }

    #[test]
    fn binds_loopback_only() {
        let l = bind(0).unwrap();
        assert!(l.local_addr().unwrap().ip().is_loopback());
    }
}
