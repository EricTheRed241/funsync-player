//! The access PIN in front of every route: the swipe deck's gate
//! (`scout_report::gate`: loopback always passes; other devices need the
//! `funsync_access` cookie or `?k=` while `<old userData>/access-pin.json`
//! sets a PIN; `/login`, `/logout`, lockout after 5 wrong PINs). The PIN file
//! is the old app's, so one login covers both apps.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::extract::ws::{CloseFrame, Message, WebSocketUpgrade};
use axum::extract::{ConnectInfo, FromRequest, Request, State};
use axum::http::{HeaderValue, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
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

/// What a headset that logged in appends to the URLs it's handed (`k=`);
/// both `None` on this computer or without a PIN.
#[derive(Clone, Default)]
pub struct HeadsetKeys {
    pub media: Option<String>,
    pub thumb: Option<String>,
}

/// `Some(true)` for HereSphere's library / scene, `Some(false)` for DeoVR's:
/// they log in from their own body instead of the gate.
fn headset_route(path: &str) -> Option<bool> {
    let (hs, rest) = if let Some(r) = path.strip_prefix("/heresphere") {
        (true, r)
    } else {
        (false, path.strip_prefix("/deovr")?)
    };
    (rest.is_empty()
        || rest
            .strip_prefix('/')
            .is_some_and(|id| !id.is_empty() && !id.contains('/')))
    .then_some(hs)
}

/// `password` from a JSON or form body.
fn password(body: &[u8]) -> Option<String> {
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) {
        return v["password"].as_str().map(str::to_string);
    }
    let text = std::str::from_utf8(body).ok()?;
    video_tags::access::query_values(text.trim(), "password")
        .next()
        .map(str::to_string)
}

/// `access_pin.headset_access`: Ok with the keys, or Err(0) login required /
/// Err(-1) login failed (wrong PIN, locked out, unusable PIN file).
fn headset_access(
    acc: &Access,
    ip: std::net::IpAddr,
    cookie: &str,
    query: &str,
    password: Option<&str>,
) -> Result<HeadsetKeys, i32> {
    use video_tags::access::{
        COOKIE_NAME, QUERY_KEY, cookie_values, is_loopback, now_secs, query_values,
    };
    if is_loopback(ip) {
        return Ok(HeadsetKeys::default());
    }
    let mut g = acc.gate.lock().unwrap();
    let pin = match g.pin(&acc.pin_dir, &mut std::io::stderr()) {
        Ok(Some(p)) => p,
        Ok(None) => return Ok(HeadsetKeys::default()),
        Err(()) => return Err(-1),
    };
    let now = now_secs();
    let keys = || HeadsetKeys {
        media: Some(pin.media_token(now)),
        thumb: Some(pin.thumb_token()),
    };
    if cookie_values(cookie, COOKIE_NAME)
        .chain(query_values(query, QUERY_KEY))
        .any(|t| pin.verify(t, now))
    {
        return Ok(keys());
    }
    match password.filter(|p| !p.is_empty()) {
        None => Err(0),
        Some(p) if g.try_pin(&pin, ip, p, now) == Some(true) => Ok(keys()),
        Some(_) => Err(-1),
    }
}

fn headset_refusal(heresphere: bool, state: i32) -> Response {
    let name = if state == -1 {
        "Login Failed"
    } else {
        "Login Required"
    };
    let list = serde_json::json!([{"name": name, "list": []}]);
    let mut r = if heresphere {
        axum::Json(serde_json::json!({"access": state, "library": list})).into_response()
    } else {
        axum::Json(serde_json::json!({"scenes": list, "authorized": state.to_string()}))
            .into_response()
    };
    let h = r.headers_mut();
    h.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    if heresphere {
        h.insert("heresphere-json-version", HeaderValue::from_static("1"));
    }
    r
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
    if let Some(hs) = headset_route(req.uri().path()) {
        let (parts, b) = req.into_parts();
        let bytes = to_bytes(b, 64 * 1024).await.unwrap_or_default();
        let query = parts.uri.query().unwrap_or("").to_string();
        let pw = password(&bytes);
        return match headset_access(&acc, addr.ip(), &cookie, &query, pw.as_deref()) {
            Ok(keys) => {
                let mut req = Request::from_parts(parts, Body::from(bytes));
                req.extensions_mut().insert(keys);
                next.run(req).await
            }
            Err(state) => headset_refusal(hs, state),
        };
    }
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
        // A refused WebSocket is accepted and closed with 4401, which the web
        // remote (remote-sync.js) takes as "log in".
        Some(_) if req.headers().contains_key(header::UPGRADE) => {
            match WebSocketUpgrade::from_request(req, &()).await {
                Ok(ws) => ws.on_upgrade(|mut s| async move {
                    let close = CloseFrame {
                        code: 4401,
                        reason: "pin required".into(),
                    };
                    let _ = s.send(Message::Close(Some(close))).await;
                }),
                Err(e) => e.into_response(),
            }
        }
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

    async fn post(app: axum::Router, path: &str, body: &str) -> serde_json::Value {
        let mut req = axum::http::Request::post(path)
            .body(Body::from(body.to_string()))
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            "192.168.2.50".parse().unwrap(),
            1,
        )));
        let resp = app.oneshot(req).await.unwrap();
        serde_json::from_slice(&to_bytes(resp.into_body(), 1 << 20).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn headsets_log_in_from_their_body() {
        let data = tempfile::tempdir().unwrap();
        let pins = tempfile::tempdir().unwrap();
        let st = crate::server::AppState::load(data.path().into()).unwrap();
        // One router, so the gate's lockout carries across requests.
        let router = crate::server::router_with_pin(st.clone(), pins.path().into());
        let app = || router.clone();
        let pin = AccessPin::new("1234", [7; 32], "now").unwrap();
        std::fs::write(pins.path().join("access-pin.json"), pin.to_json()).unwrap();
        let v = post(app(), "/heresphere", "{}").await;
        assert_eq!(
            (v["access"].clone(), v["library"][0]["name"].clone()),
            (0.into(), "Login Required".into())
        );
        let v = post(
            app(),
            "/heresphere",
            r#"{"username":"x","password":"9999"}"#,
        )
        .await;
        assert_eq!(
            (v["access"].clone(), v["library"][0]["name"].clone()),
            ((-1).into(), "Login Failed".into())
        );
        assert_eq!(
            post(app(), "/heresphere", r#"{"password":"1234"}"#).await["access"],
            1
        );
        let v = post(app(), "/deovr", "login=x&password=1234").await;
        assert_eq!(v["authorized"], "1");
        assert_eq!(post(app(), "/deovr", "").await["authorized"], "0");
        // Five wrong PINs lock the address out, the right one included.
        for _ in 0..5 {
            post(app(), "/deovr", "password=0000").await;
        }
        assert_eq!(
            post(app(), "/heresphere", r#"{"password":"1234"}"#).await["access"],
            -1
        );
        // Keys: the thumbnail one opens thumbnails only; the media one is a login.
        let thumb = format!("k={}", pin.thumb_token());
        let t = get(
            app(),
            "192.168.2.50",
            &format!("/api/media/thumb/x?{thumb}"),
            None,
        )
        .await;
        assert_eq!(t.status(), StatusCode::NOT_FOUND);
        let h = get(app(), "192.168.2.50", &format!("/api/status?{thumb}"), None).await;
        assert_ne!(h.status(), StatusCode::OK);
        let media = format!(
            "/api/status?k={}",
            pin.media_token(video_tags::access::now_secs())
        );
        assert_eq!(
            get(app(), "192.168.2.50", &media, None).await.status(),
            StatusCode::OK
        );
    }
}
