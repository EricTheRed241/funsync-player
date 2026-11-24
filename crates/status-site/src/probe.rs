//! The sites and how one is probed: an HTTP GET from this Mac to `127.0.0.1:<port><probe>`
//! (loopback passes the access PIN), up when it answers 2xx in time; and a TCP connect to
//! the Mac's network address, which tells whether other devices can reach the port. Every
//! site is on the desktop app's one port: the backend serves the web remote and proxies the
//! swipe deck (`/swipe/`, its Downloads page `/swipe/downloads`) and the Creators site
//! (`/creators/`), which it starts itself.

use serde::Serialize;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

/// Longest wait to connect.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// Longest wait for the status line (a deck is rebuilt from the scouts' files per request).
const READ_TIMEOUT: Duration = Duration::from_secs(8);

/// A site on the page.
#[derive(Debug, Clone, Copy)]
pub struct Site {
    pub name: &'static str,
    pub port: u16,
    /// What the probe asks for: an endpoint that reads the site's data where there is one.
    pub probe: &'static str,
    /// The page the status page links to.
    pub open: &'static str,
    /// How to start it when it's down.
    pub start: &'static str,
}

const APP: &str = "the desktop app (it runs the sites; restart it)";

pub const SITES: &[Site] = &[
    Site {
        name: "Backend",
        port: 5123,
        probe: "/health",
        open: "/health",
        start: APP,
    },
    Site {
        name: "Web remote",
        port: 5123,
        probe: "/remote/",
        open: "/remote/",
        start: APP,
    },
    Site {
        name: "Swipe deck",
        port: 5123,
        probe: "/swipe/api/deck?limit=1",
        open: "/swipe/",
        start: APP,
    },
    Site {
        name: "Downloads",
        port: 5123,
        probe: "/swipe/api/downloads",
        open: "/swipe/downloads",
        start: APP,
    },
    Site {
        name: "Creators",
        port: 5123,
        probe: "/creators/api/creators/deck?limit=1",
        open: "/creators/",
        start: APP,
    },
    Site {
        name: "Collection",
        port: 5123,
        probe: "/creators/api/creators/collection",
        open: "/creators/collection",
        start: APP,
    },
];

/// One site's result, as `/api/status` sends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub name: &'static str,
    pub port: u16,
    pub open: &'static str,
    pub start: &'static str,
    pub up: bool,
    /// The HTTP status, when it answered.
    pub http: Option<u16>,
    pub ms: u64,
    /// Why it's down (not running, timed out, the status it answered).
    pub error: Option<String>,
    /// Whether the port answers on the Mac's network address; None without a network.
    pub lan: Option<bool>,
}

/// The HTTP part of a probe: the status code and the time to it, or what went wrong.
pub fn http_get(addr: SocketAddr, path: &str) -> (Result<u16, String>, u64) {
    let started = Instant::now();
    let result = (|| {
        let mut s =
            TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| match e.kind() {
                std::io::ErrorKind::ConnectionRefused => "not running".to_string(),
                std::io::ErrorKind::TimedOut => "no answer (connect timed out)".to_string(),
                _ => e.to_string(),
            })?;
        s.set_read_timeout(Some(READ_TIMEOUT))
            .and_then(|()| s.set_write_timeout(Some(READ_TIMEOUT)))
            .and_then(|()| {
                write!(
                    s,
                    "GET {path} HTTP/1.1\r\nHost: {addr}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
                )
            })
            .map_err(|e| e.to_string())?;
        let mut head = Vec::new();
        let mut buf = [0u8; 512];
        while !head.contains(&b'\n') && head.len() < 4096 {
            match s.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => head.extend_from_slice(&buf[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(format!("no answer within {} s", READ_TIMEOUT.as_secs()));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        let line = String::from_utf8_lossy(&head);
        let line = line.lines().next().unwrap_or("");
        line.split_whitespace()
            .nth(1)
            .filter(|_| line.starts_with("HTTP/"))
            .and_then(|code| code.parse().ok())
            .ok_or_else(|| "not an HTTP answer".to_string())
    })();
    (result, started.elapsed().as_millis() as u64)
}

/// Whether something accepts connections at `addr`.
pub fn accepts(addr: SocketAddr) -> bool {
    TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).is_ok()
}

/// Probe one site on `host` (127.0.0.1 in use; a fake server in tests), its port also on
/// `lan` when given.
pub fn check(site: &Site, host: IpAddr, lan: Option<IpAddr>) -> Status {
    let (result, ms) = http_get(SocketAddr::new(host, site.port), site.probe);
    let (up, http, error) = match result {
        Ok(code @ 200..=299) => (true, Some(code), None),
        Ok(code) => (false, Some(code), Some(format!("answered HTTP {code}"))),
        Err(e) => (false, None, Some(e)),
    };
    // A site that isn't running can't be reached from the network either.
    let lan = lan.map(|ip| http.is_some() && accepts(SocketAddr::new(ip, site.port)));
    Status {
        name: site.name,
        port: site.port,
        open: site.open,
        start: site.start,
        up,
        http,
        ms,
        error,
        lan,
    }
}

/// Probe every site at once, in their order.
pub fn check_all(sites: &[Site], host: IpAddr, lan: Option<Ipv4Addr>) -> Vec<Status> {
    let lan = lan.map(IpAddr::V4);
    std::thread::scope(|scope| {
        let handles: Vec<_> = sites
            .iter()
            .map(|site| scope.spawn(move || check(site, host, lan)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("probe thread panicked"))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    const LOCAL: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    /// A server on a free loopback port answering every connection with `reply`.
    fn fake(reply: &'static str) -> u16 {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut s in listener.incoming().flatten() {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let _ = s.write_all(reply.as_bytes());
            }
        });
        port
    }

    /// A loopback port nothing listens on.
    fn closed_port() -> u16 {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        l.local_addr().unwrap().port()
    }

    fn site(port: u16) -> Site {
        Site {
            name: "Test",
            port,
            probe: "/health",
            open: "/",
            start: "start it",
        }
    }

    #[test]
    fn a_2xx_answer_is_up() {
        let s = check(&site(fake("HTTP/1.1 200 OK\r\n\r\n{}")), LOCAL, None);
        assert!(s.up);
        assert_eq!((s.http, s.error, s.lan), (Some(200), None, None));
    }

    #[test]
    fn an_error_status_is_down_with_the_code() {
        let s = check(
            &site(fake("HTTP/1.1 503 Service Unavailable\r\n\r\n")),
            LOCAL,
            None,
        );
        assert!(!s.up);
        assert_eq!(s.http, Some(503));
        assert_eq!(s.error.as_deref(), Some("answered HTTP 503"));
    }

    #[test]
    fn a_closed_port_is_not_running_and_not_on_the_network() {
        let s = check(&site(closed_port()), LOCAL, Some(LOCAL));
        assert!(!s.up);
        assert_eq!(s.http, None);
        assert_eq!(s.error.as_deref(), Some("not running"));
        assert_eq!(s.lan, Some(false));
    }

    #[test]
    fn something_that_isnt_http_is_down() {
        let s = check(&site(fake("SSH-2.0-OpenSSH\r\n")), LOCAL, None);
        assert!(!s.up);
        assert_eq!(s.error.as_deref(), Some("not an HTTP answer"));
    }

    #[test]
    fn the_network_check_sees_whether_the_port_answers_there() {
        // Loopback stands in for the network address: the fake listens there.
        let s = check(&site(fake("HTTP/1.1 200 OK\r\n\r\n")), LOCAL, Some(LOCAL));
        assert_eq!(s.lan, Some(true));
    }

    #[test]
    fn check_all_keeps_the_sites_order() {
        let up = fake("HTTP/1.1 204 No Content\r\n\r\n");
        let sites = [site(closed_port()), site(up)];
        let got: Vec<bool> = check_all(&sites, LOCAL, None)
            .iter()
            .map(|s| s.up)
            .collect();
        assert_eq!(got, [false, true]);
    }
}
