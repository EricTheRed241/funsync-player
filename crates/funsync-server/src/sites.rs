//! The local Rust sites of `deploy/sites.json` (swipe deck, Creators, Status)
//! on this port, as the old backend serves them (`backend/routes/sites.py`):
//! `<prefix>` → `<prefix>/`, `<prefix>/<rest>` relayed to the site's
//! `127.0.0.1:<port>/<rest>` (`<prefix>/` = its home), `Location` answers
//! mapped back under the prefix, HTML pages given `<base href="<prefix>/">`
//! and the nav bar; each site page's bare `<path>` redirects to
//! `<prefix><path>`. With `--supervise` the server also runs the sites and
//! the job runner (`electron/sites.js`, `electron/job-runner.js`): built
//! from the checkout and restarted after they exit.

use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{any, get};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::server::AppState;

const SITES_JSON: &str = include_str!("../../../deploy/sites.json");
const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

#[derive(Debug, Clone, Deserialize)]
pub struct Site {
    pub name: String,
    pub prefix: String,
    pub package: String,
    pub port: u16,
    pub home: String,
    #[serde(default)]
    pub pages: Vec<Page>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Page {
    pub name: String,
    pub path: String,
}

static SITES: LazyLock<Vec<Site>> = LazyLock::new(|| {
    #[derive(Deserialize)]
    struct File {
        sites: Vec<Site>,
    }
    serde_json::from_str::<File>(SITES_JSON)
        .map(|f| f.sites)
        .unwrap_or_default()
});

pub fn sites() -> &'static [Site] {
    &SITES
}

/// The nav bar's links: the web remote, each site and its pages, then this
/// app's own pages.
pub fn nav_links() -> serde_json::Value {
    let mut links = vec![serde_json::json!({"name": "Remote", "href": "/remote/"})];
    for s in sites() {
        links.push(serde_json::json!({"name": s.name, "href": format!("{}/", s.prefix)}));
        for p in &s.pages {
            links.push(
                serde_json::json!({"name": p.name, "href": format!("{}{}", s.prefix, p.path)}),
            );
        }
    }
    links.push(serde_json::json!({"name": "Devices", "href": "/remote/devices.html"}));
    links.push(serde_json::json!({"name": "Overview", "href": "/remote/overview.html"}));
    links.push(serde_json::json!({"name": "EroScripts", "href": "/remote/eroscripts.html"}));
    links.push(serde_json::json!({"name": "Settings", "href": "/remote/settings.html"}));
    links.push(serde_json::json!({"name": "Parity", "href": "/parity"}));
    serde_json::Value::Array(links)
}

/// A `Location` from the site as the browser must follow it.
pub fn outside_location(site: &Site, location: &str) -> String {
    if !location.starts_with('/') || location.starts_with("//") {
        return location.to_string();
    }
    let (path, query) = location
        .split_once('?')
        .map_or((location, None), |(p, q)| (p, Some(q)));
    let path = if path == site.home { "/" } else { path };
    format!(
        "{}{path}{}",
        site.prefix,
        query.map(|q| format!("?{q}")).unwrap_or_default()
    )
}

/// `<base>` and the nav script right after `<head …>` (unchanged without one).
pub fn inject(site: &Site, page: &[u8]) -> Vec<u8> {
    let lower = page.to_ascii_lowercase();
    let Some(start) = lower.windows(5).position(|w| w == b"<head") else {
        return page.to_vec();
    };
    let Some(end) = lower[start..]
        .iter()
        .position(|&b| b == b'>')
        .map(|i| start + i + 1)
    else {
        return page.to_vec();
    };
    let tag = format!(
        r#"<base href="{}/"><script src="/site-nav.js" defer></script>"#,
        site.prefix
    );
    [&page[..end], tag.as_bytes(), &page[end..]].concat()
}

const PASS_REQUEST: [&str; 5] = [
    "accept",
    "content-type",
    "if-none-match",
    "if-modified-since",
    "range",
];
const PASS_ANSWER: [&str; 8] = [
    "content-type",
    "cache-control",
    "etag",
    "last-modified",
    "retry-after",
    "content-range",
    "accept-ranges",
    "location",
];

/// One HTTP/1.1 exchange with a local site (they answer with
/// `Content-Length` and `Connection: close`): status, headers, body.
async fn exchange(
    port: u16,
    head: String,
    body: &[u8],
) -> std::io::Result<(u16, Vec<(String, String)>, Vec<u8>)> {
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
    s.write_all(head.as_bytes()).await?;
    s.write_all(body).await?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await?;
    let bad = || std::io::Error::other("bad answer");
    let split = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(bad)?;
    let text = String::from_utf8_lossy(&buf[..split]).into_owned();
    let mut lines = text.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|c| c.parse().ok())
        .ok_or_else(bad)?;
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Ok((status, headers, buf[split + 4..].to_vec()))
}

async fn relay(site: &'static Site, rest: &str, req: Request) -> Response {
    let path = if rest.is_empty() {
        site.home.clone()
    } else {
        format!("/{rest}")
    };
    let query = req
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let html_wanted = req
        .headers()
        .get(header::ACCEPT)
        .and_then(|a| a.to_str().ok())
        .is_some_and(|a| a.contains("text/html"));
    let mut head = format!(
        "{} {path}{query} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n",
        req.method(),
        site.port
    );
    for h in PASS_REQUEST {
        if let Some(v) = req.headers().get(h).and_then(|v| v.to_str().ok()) {
            head.push_str(&format!("{h}: {v}\r\n"));
        }
    }
    let body = to_bytes(req.into_body(), 16 << 20)
        .await
        .unwrap_or_default();
    head.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    let (status, headers, mut out) =
        match tokio::time::timeout(Duration::from_secs(60), exchange(site.port, head, &body)).await
        {
            Ok(Ok(a)) => a,
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                return down(site, "not running", html_wanted);
            }
            Ok(Err(_)) => return down(site, "error", html_wanted),
            Err(_) => return down(site, "timed out", html_wanted),
        };
    let mut resp = Response::builder().status(status);
    for (k, v) in &headers {
        if !PASS_ANSWER.contains(&k.as_str()) {
            continue;
        }
        let v = if k == "location" {
            outside_location(site, v)
        } else {
            v.clone()
        };
        if k == "content-type" && v.starts_with("text/html") {
            out = inject(site, &out);
        }
        if let Ok(v) = HeaderValue::from_str(&v) {
            resp = resp.header(k.as_str(), v);
        }
    }
    resp.body(Body::from(out))
        .unwrap_or_else(|_| down(site, "bad answer", html_wanted))
}

fn down(site: &Site, why: &str, html: bool) -> Response {
    let body = if html {
        format!(
            "<!doctype html><html><head><meta charset=utf-8><meta name=viewport content=\"width=device-width, initial-scale=1\">\
             <title>{0}</title><script src=\"/site-nav.js\" defer></script></head><body style=\"font:15px -apple-system,sans-serif;padding:24px\">\
             <p><b>{0}</b> isn't answering ({why}).</p><p>The app that runs it (the desktop app, or this server with \
             <code>--supervise</code>) starts it again within seconds.</p></body></html>",
            site.name
        )
    } else {
        format!("{} isn't answering ({why})\n", site.name)
    };
    let ct = if html {
        "text/html; charset=utf-8"
    } else {
        "text/plain"
    };
    (StatusCode::BAD_GATEWAY, [(header::CONTENT_TYPE, ct)], body).into_response()
}

pub fn routes() -> Router<Arc<AppState>> {
    let mut r = Router::new();
    for site in sites() {
        let to = format!("{}/", site.prefix);
        r =
            r.route(&site.prefix, get(move || async move { Redirect::to(&to) }))
                .route(
                    &format!("{}/", site.prefix),
                    any(move |req: Request| relay(site, "", req)),
                )
                .route(
                    &format!("{}/{{*rest}}", site.prefix),
                    any(
                        move |axum::extract::Path(rest): axum::extract::Path<String>,
                              req: Request| async move {
                            relay(site, &rest, req).await
                        },
                    ),
                );
        for page in &site.pages {
            let to = format!("{}{}", site.prefix, page.path);
            r = r.route(&page.path, get(move || async move { Redirect::to(&to) }));
        }
    }
    r
}

/// Run every site and the job runner like the desktop app does: build, run,
/// restart a while after an exit. A site whose port already answers (the
/// desktop app runs it) is left alone and checked again later.
pub fn supervise(user_data: &Path) {
    for s in sites() {
        let args = vec![
            "--port".into(),
            s.port.to_string(),
            "--user-data".into(),
            user_data.display().to_string(),
        ];
        tokio::spawn(keep_running(
            s.package.clone(),
            args,
            Some(s.port),
            Duration::from_secs(5),
        ));
    }
    tokio::spawn(keep_running(
        "job-runner".into(),
        vec!["serve".into()],
        None,
        Duration::from_secs(60),
    ));
}

async fn keep_running(pkg: String, args: Vec<String>, port: Option<u16>, delay: Duration) {
    let path = format!(
        "{}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:{}",
        std::env::var("HOME").unwrap_or_default(),
        std::env::var("PATH").unwrap_or_default()
    );
    loop {
        let taken = match port {
            Some(p) => tokio::net::TcpStream::connect(("127.0.0.1", p))
                .await
                .is_ok(),
            None => false,
        };
        if !taken {
            let built = tokio::process::Command::new("cargo")
                .args(["build", "-q", "--release", "-p", &pkg])
                .current_dir(REPO)
                .env("PATH", &path)
                .status()
                .await;
            match built {
                Ok(s) if s.success() => {
                    let bin = Path::new(REPO).join("target/release").join(&pkg);
                    match tokio::process::Command::new(bin)
                        .args(&args)
                        .current_dir(REPO)
                        .env("PATH", &path)
                        .spawn()
                    {
                        Ok(mut child) => {
                            crate::log::line(format!("[{pkg}] started (pid {:?})", child.id()));
                            let status = child.wait().await;
                            crate::log::line(format!(
                                "[{pkg}] exited ({status:?}); again in {} s",
                                delay.as_secs()
                            ));
                        }
                        Err(e) => crate::log::line(format!("[{pkg}] can't start: {e}")),
                    }
                }
                other => crate::log::line(format!("[{pkg}] build failed ({other:?})")),
            }
        }
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations_and_injection() {
        let s = sites()
            .iter()
            .find(|s| s.prefix == "/swipe")
            .expect("swipe site");
        assert_eq!(outside_location(s, "/swipe?x=1"), "/swipe/?x=1");
        assert_eq!(outside_location(s, "/downloads"), "/swipe/downloads");
        assert_eq!(outside_location(s, "https://e.x/a"), "https://e.x/a");
        let page = inject(s, b"<html><HEAD lang=x><title>t</title>");
        assert_eq!(
            String::from_utf8(page).unwrap(),
            r#"<html><HEAD lang=x><base href="/swipe/"><script src="/site-nav.js" defer></script><title>t</title>"#
        );
        assert!(nav_links().to_string().contains("/swipe/downloads"));
    }
}
