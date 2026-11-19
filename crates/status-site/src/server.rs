//! The HTTP side, on scout-report's plumbing (scout_report::server::serve_site) behind its
//! access-PIN gate and login page (scout_report::gate):
//!
//! - `GET /`: the page (status.html), which loads `/api/status` every 15 s.
//! - `GET /api/status`: `{checkedAt (Unix seconds), network (the Mac's address or null),
//!   sites: [crate::probe::Status]}`, probed on the request.

use crate::probe::{self, Site};
use scout_report::gate::Gate;
use scout_report::server::{Response, html, json_ok, serve_site, text};
use serde_json::json;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::path::Path;

/// The page: HTML, CSS and JS in one file, no external scripts.
pub const PAGE: &str = include_str!("status.html");
/// The site's name (log lines) and its pages, for the gate.
pub const NAME: &str = "status-site";
pub const PAGES: &[&str] = &["/"];

/// Answer one request against `sites`. HEAD gets GET's headers (the caller drops the body).
pub fn route(method: &str, target: &str, sites: &[Site]) -> Response {
    let target = target.split('#').next().unwrap_or("");
    let path = target.split_once('?').map_or(target, |(p, _)| p);
    match (method, path) {
        ("GET" | "HEAD", "/") => html(PAGE),
        ("GET" | "HEAD", "/api/status") => {
            let network = scout_report::server::lan_address();
            let sites = probe::check_all(sites, IpAddr::V4(Ipv4Addr::LOCALHOST), network);
            json_ok(&json!({
                "checkedAt": now_secs(),
                "network": network.map(|ip| ip.to_string()),
                "sites": sites,
            }))
        }
        _ => text(404, "Not found\n"),
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Serve the status page behind the gate, forever.
pub fn serve(listener: &TcpListener, user_data: &Path, err: &mut dyn Write) {
    serve_site(listener, user_data, Gate::new(NAME, PAGES), err, |r| {
        route(r.method, r.target, probe::SITES)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn status_lists_every_site_in_order() {
        let closed = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let sites = [Site {
            name: "Gone",
            port: closed,
            probe: "/",
            open: "/x",
            start: "npm run x",
        }];
        let r = route("GET", "/api/status?t=1", &sites);
        assert_eq!(r.status, 200);
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v["sites"][0]["name"], "Gone");
        assert_eq!(v["sites"][0]["up"], false);
        assert_eq!(v["sites"][0]["error"], "not running");
        assert_eq!(v["sites"][0]["open"], "/x");
    }

    #[test]
    fn unknown_paths_and_writes_are_404() {
        assert_eq!(route("GET", "/", &[]).status, 200);
        assert_eq!(route("GET", "/nowhere", &[]).status, 404);
        assert_eq!(route("POST", "/api/status", &[]).status, 404);
    }
}
