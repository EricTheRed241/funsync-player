//! The access PIN in front of every route: the swipe deck's gate
//! (`scout_report::gate`: loopback always passes; other devices need the
//! `funsync_access` cookie or `?k=` while `<old userData>/access-pin.json`
//! sets a PIN; `/login`, `/logout`, lockout after 5 wrong PINs). The PIN file
//! is the old app's, so one login covers both apps.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;
use scout_report::gate::{self, Gate};

/// Paths that are pages: a device without the PIN is sent to the login page.
const PAGES: &[&str] = &["/", "/remote", "/remote/", "/status"];

pub struct Access {
    gate: Mutex<Gate>,
    pin_dir: PathBuf,
}

impl Access {
    pub fn new(pin_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            gate: Mutex::new(Gate::new("funsync-server", PAGES)),
            pin_dir,
        })
    }
}

pub async fn layer(
    State(acc): State<Arc<Access>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let (cookie, accept) = {
        let header = |name: header::HeaderName| -> String {
            req.headers()
                .get_all(&name)
                .iter()
                .filter_map(|v| v.to_str().ok())
                .collect::<Vec<_>>()
                .join("; ")
        };
        (header(header::COOKIE), header(header::ACCEPT))
    };
    let method = req.method().as_str().to_string();
    let target = req
        .uri()
        .path_and_query()
        .map_or("/".into(), |p| p.to_string());
    // Only the login form's body is read; every other body goes on untouched.
    let (req, body) = if method == "POST" && req.uri().path() == "/login" {
        let (parts, b) = req.into_parts();
        let bytes = to_bytes(b, 64 * 1024).await.unwrap_or_default();
        (Request::from_parts(parts, Body::empty()), bytes.to_vec())
    } else {
        (req, Vec::new())
    };
    let greq = gate::Request {
        method: &method,
        target: &target,
        body: &body,
        client: addr.ip(),
        cookie: &cookie,
        accept: &accept,
    };
    let refused = {
        let mut g = acc.gate.lock().unwrap();
        let mut passed = false;
        let resp = g.respond(
            &greq,
            &acc.pin_dir,
            video_tags::access::now_secs(),
            &mut std::io::stderr(),
            |_| {
                passed = true;
                scout_report::server::Response {
                    status: 200,
                    content_type: "",
                    cache: "",
                    body: Vec::new(),
                    headers: Vec::new(),
                }
            },
        );
        (!passed).then_some(resp)
    };
    match refused {
        None => next.run(req).await,
        Some(r) => {
            let mut out = Response::new(Body::from(r.body));
            *out.status_mut() = axum::http::StatusCode::from_u16(r.status).unwrap_or_default();
            let h = out.headers_mut();
            for (k, v) in [
                ("content-type", r.content_type.to_string()),
                ("cache-control", r.cache.to_string()),
            ]
            .into_iter()
            .chain(r.headers)
            {
                if let (Ok(name), Ok(val)) =
                    (header::HeaderName::try_from(k), HeaderValue::from_str(&v))
                    && !v.is_empty()
                {
                    h.append(name, val);
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use tower::ServiceExt;
    use video_tags::access::AccessPin;

    async fn get(
        app: axum::Router,
        ip: &str,
        path: &str,
        extra: Option<(&str, String)>,
    ) -> Response {
        let mut b = axum::http::Request::get(path).header("accept", "text/html");
        if let Some((k, v)) = extra {
            b = b.header(k, v);
        }
        let mut req = b.body(Body::empty()).unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(ip.parse().unwrap(), 1)));
        app.oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn pin_gates_other_devices_only() {
        let data = tempfile::tempdir().unwrap();
        let pins = tempfile::tempdir().unwrap();
        let st = crate::server::AppState::load(data.path().into()).unwrap();
        let app = || crate::server::router_with_pin(st.clone(), pins.path().into());
        // No PIN file: open to everyone.
        assert_eq!(
            get(app(), "192.168.2.50", "/health", None).await.status(),
            StatusCode::OK
        );
        let pin = AccessPin::new("1234", [7; 32], "now").unwrap();
        std::fs::write(pins.path().join("access-pin.json"), pin.to_json()).unwrap();
        assert_eq!(
            get(app(), "127.0.0.1", "/health", None).await.status(),
            StatusCode::OK
        );
        let page = get(app(), "192.168.2.50", "/status", None).await;
        assert_eq!(page.status(), StatusCode::SEE_OTHER);
        assert!(
            page.headers()["location"]
                .to_str()
                .unwrap()
                .starts_with("/login")
        );
        let cookie = format!(
            "funsync_access={}",
            pin.token(video_tags::access::now_secs())
        );
        let ok = get(app(), "192.168.2.50", "/health", Some(("cookie", cookie))).await;
        assert_eq!(ok.status(), StatusCode::OK);
    }
}
