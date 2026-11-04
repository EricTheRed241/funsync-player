//! Ask the running app to rescan (`POST /api/remote/request-rescan` on the
//! local backend; the desktop polls the counter and rescans, merging the new
//! file). Raw HTTP/1.1 over a TcpStream: one local request needs no HTTP
//! crate.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

pub const PATH: &str = "/api/remote/request-rescan";
pub const TIMEOUT: Duration = Duration::from_secs(2);

/// Send the request; `Ok(status line)` on a 2xx answer.
pub fn request_rescan(port: u16, timeout: Duration) -> Result<String, String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream =
        TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("{addr}: {e}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    let request = format!(
        "POST {PATH} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("{addr}: {e}"))?;
    let mut head = Vec::with_capacity(256);
    let mut buf = [0u8; 256];
    while !head.windows(2).any(|w| w == b"\r\n") {
        let n = stream.read(&mut buf).map_err(|e| format!("{addr}: {e}"))?;
        if n == 0 || head.len() > 4096 {
            break;
        }
        head.extend_from_slice(&buf[..n]);
    }
    let text = String::from_utf8_lossy(&head);
    let status = text.lines().next().unwrap_or("").trim().to_string();
    let code = status
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok());
    match code {
        Some(200..=299) => Ok(status),
        _ if status.is_empty() => Err(format!("{addr}: no answer")),
        _ => Err(format!("{addr}: {status}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::thread;

    fn serve_once(answer: &'static str) -> (u16, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut first = String::new();
            BufReader::new(&conn).read_line(&mut first).unwrap();
            conn.write_all(answer.as_bytes()).unwrap();
            first
        });
        (port, handle)
    }

    #[test]
    fn posts_to_the_rescan_endpoint() {
        let (port, server) = serve_once("HTTP/1.1 200 OK\r\ncontent-length: 9\r\n\r\n{\"seq\":3}");
        assert_eq!(
            request_rescan(port, TIMEOUT),
            Ok("HTTP/1.1 200 OK".to_string())
        );
        assert_eq!(server.join().unwrap(), format!("POST {PATH} HTTP/1.1\r\n"));
    }

    #[test]
    fn reports_an_error_status_or_no_backend() {
        let (port, server) = serve_once("HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n");
        assert!(
            request_rescan(port, TIMEOUT)
                .unwrap_err()
                .ends_with("HTTP/1.1 404 Not Found")
        );
        server.join().unwrap();
        // A port nobody listens on (bound, then released).
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(request_rescan(port, TIMEOUT).is_err());
    }
}
