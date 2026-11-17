//! The access PIN in front of every route (video_tags::access): this Mac
//! (loopback) always passes; other devices pass with a valid `funsync_access`
//! cookie (or `k=<token>` query) while `<userData>/access-pin.json` sets a
//! PIN, which is reread on every request. Otherwise pages (`/`, `/swipe`, or
//! anything a browser asks for as HTML) redirect to `GET /login?next=…` and
//! everything else gets 401 `{"error":"pin required"}`. `POST /login {pin,
//! next}` sets the cookie (30 days); 5 wrong PINs from one address within
//! 10 minutes lock it out (429) until the first of them is 10 minutes old.
//! `POST /logout` clears the cookie. An unreadable PIN file refuses every
//! other device (503).

use crate::server::{self, Response};
use serde_json::json;
use std::collections::HashMap;
use std::io::Write;
use std::net::IpAddr;
use std::path::Path;
use video_tags::access::{self, AccessPin};

/// The login form; `{{MESSAGE}}` and `{{NEXT}}` are filled in.
pub const LOGIN_PAGE: &str = include_str!("login.html");

/// Wrong PINs from one address within FAILURE_WINDOW_SECS that lock it out.
pub const MAX_FAILURES: usize = 5;
pub const FAILURE_WINDOW_SECS: u64 = 10 * 60;

/// What the gate needs from a request.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    pub method: &'a str,
    pub target: &'a str,
    pub body: &'a [u8],
    pub client: IpAddr,
    /// Every `Cookie` header, joined with `; `.
    pub cookie: &'a str,
    /// The `Accept` header.
    pub accept: &'a str,
}

/// The gate's memory between requests: wrong PINs per address, and the last
/// PIN file problem logged.
#[derive(Debug, Default)]
pub struct Gate {
    failures: HashMap<IpAddr, Vec<u64>>,
    last_problem: Option<String>,
}

impl Gate {
    /// Answer a request: the gate's own answer (login, logout, refusals), else
    /// the route's. `now` is Unix seconds; PIN file problems go to `err`.
    pub fn respond(
        &mut self,
        req: &Request,
        user_data: &Path,
        now: u64,
        err: &mut dyn Write,
    ) -> Response {
        match self.check(req, user_data, now, err) {
            Some(resp) => resp,
            None => server::route(req.method, req.target, req.body, user_data),
        }
    }

    fn check(
        &mut self,
        req: &Request,
        user_data: &Path,
        now: u64,
        err: &mut dyn Write,
    ) -> Option<Response> {
        let target = req.target.split('#').next().unwrap_or("");
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        match (req.method, path) {
            ("GET" | "HEAD", "/login") => {
                let next = query_value(query, "next");
                return Some(match self.load(user_data, err) {
                    Ok(Some(_)) => login_form(200, safe_next(&next), None),
                    Ok(None) => redirect(safe_next(&next)),
                    Err(()) => unreadable(true),
                });
            }
            ("POST", "/login") => return Some(self.login(req, user_data, now, err)),
            ("POST", "/logout") => {
                let mut resp = redirect("/");
                resp.headers
                    .push(("Set-Cookie", access::CLEAR_COOKIE.to_string()));
                return Some(resp);
            }
            _ => {}
        }
        if access::is_loopback(req.client) {
            return None;
        }
        let page = matches!(req.method, "GET" | "HEAD")
            && (matches!(path, "/" | "/swipe" | "/creators") || req.accept.contains("text/html"));
        let pin = match self.load(user_data, err) {
            Ok(Some(pin)) => pin,
            Ok(None) => return None,
            Err(()) => return Some(unreadable(page)),
        };
        let mut tokens = access::cookie_values(req.cookie, access::COOKIE_NAME)
            .chain(access::query_values(query, access::QUERY_KEY));
        if tokens.any(|t| pin.verify(t, now)) {
            return None;
        }
        Some(if page {
            redirect(&format!("/login?next={}", encode(target)))
        } else {
            server::json(401, &json!({"error": "pin required"}))
        })
    }

    /// `POST /login`: check the PIN, unless this address is locked out.
    fn login(
        &mut self,
        req: &Request,
        user_data: &Path,
        now: u64,
        err: &mut dyn Write,
    ) -> Response {
        let next = form_value(req.body, "next");
        let next = safe_next(&next);
        let pin = match self.load(user_data, err) {
            Ok(Some(pin)) => pin,
            Ok(None) => return redirect(next),
            Err(()) => return unreadable(true),
        };
        if let Some(wait) = self.locked_for(req.client, now) {
            let minutes = wait.div_ceil(60);
            let message = format!(
                "Too many wrong PINs. Try again in {minutes} minute{}.",
                if minutes == 1 { "" } else { "s" }
            );
            let mut resp = login_form(429, next, Some(&message));
            resp.headers.push(("Retry-After", wait.to_string()));
            return resp;
        }
        if pin.check_pin(form_value(req.body, "pin").trim()) {
            self.failures.remove(&req.client);
            let mut resp = redirect(next);
            resp.headers
                .push(("Set-Cookie", access::set_cookie(&pin.token(now))));
            return resp;
        }
        self.failures.entry(req.client).or_default().push(now);
        login_form(401, next, Some("Wrong PIN"))
    }

    /// Seconds until `client` may try a PIN again, if it's locked out. Forgets
    /// failures older than the window, for every address.
    fn locked_for(&mut self, client: IpAddr, now: u64) -> Option<u64> {
        self.failures.retain(|_, times| {
            times.retain(|&t| t + FAILURE_WINDOW_SECS > now);
            !times.is_empty()
        });
        let times = self.failures.get(&client)?;
        (times.len() >= MAX_FAILURES)
            .then(|| times[times.len() - MAX_FAILURES] + FAILURE_WINDOW_SECS - now)
    }

    /// The PIN file; a problem with it is logged once (until it changes).
    fn load(&mut self, user_data: &Path, err: &mut dyn Write) -> Result<Option<AccessPin>, ()> {
        match AccessPin::load(user_data) {
            Ok(pin) => {
                self.last_problem = None;
                Ok(pin)
            }
            Err(e) => {
                if self.last_problem.as_ref() != Some(&e) {
                    let _ = writeln!(
                        err,
                        "scout-report: access PIN file unreadable, refusing other devices: {e} \
                         (fix it with npm run access:pin -- set <pin>, or -- clear)"
                    );
                    self.last_problem = Some(e);
                }
                Err(())
            }
        }
    }
}

const UNREADABLE: &str = "The access PIN file (access-pin.json) can't be read, so other devices are \
                          refused. On the Mac: npm run access:pin -- set <pin>, or -- clear.";

fn unreadable(page: bool) -> Response {
    if page {
        server::text(503, &format!("{UNREADABLE}\n"))
    } else {
        server::json(
            503,
            &json!({"error": "access pin file unreadable", "message": UNREADABLE}),
        )
    }
}

fn redirect(location: &str) -> Response {
    let mut resp = server::text(303, "");
    resp.headers.push(("Location", location.to_string()));
    resp
}

fn login_form(status: u16, next: &str, message: Option<&str>) -> Response {
    let message = message.map_or(String::new(), |m| {
        format!(r#"<p class="error" role="alert">{}</p>"#, escape(m))
    });
    let page = LOGIN_PAGE
        .replace("{{MESSAGE}}", &message)
        .replace("{{NEXT}}", &escape(next));
    let mut resp = server::html(&page);
    resp.status = status;
    resp
}

/// Where to go after logging in: a path on this site (starts with one `/`,
/// printable ASCII only, not the login or logout page), else `/`.
pub fn safe_next(next: &str) -> &str {
    let path = next.split(['?', '#']).next().unwrap_or("");
    let ok = next.starts_with('/')
        && !next.starts_with("//")
        && next.bytes().all(|b| b.is_ascii_graphic() && b != b'\\')
        && !matches!(path, "/login" | "/logout");
    if ok { next } else { "/" }
}

/// The decoded value of `key` in a query string, or "".
fn query_value(query: &str, key: &str) -> String {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| decode(k) == key)
        .map_or(String::new(), |(_, v)| decode(v))
}

/// The decoded value of `key` in a form body (`application/x-www-form-urlencoded`), or "".
fn form_value(body: &[u8], key: &str) -> String {
    query_value(&String::from_utf8_lossy(body), key)
}

/// Percent-decoding, with `+` as a space.
fn decode(s: &str) -> String {
    let hex = |b: Option<&u8>| b.and_then(|&b| (b as char).to_digit(16));
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match (bytes[i], hex(bytes.get(i + 1)), hex(bytes.get(i + 2))) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
            (b'+', ..) => out.push(b' '),
            (b, ..) => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encoding for a query value: everything but unreserved characters and `/`.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// HTML-escape text for an element or a quoted attribute.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::path::PathBuf;

    const NOW: u64 = 1_763_290_000;
    const LAN: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 2, 30));
    const LOCAL: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
    const HTML: &str = "text/html,application/xhtml+xml,*/*;q=0.8";

    fn user_data(name: &str, pin: Option<[u8; 32]>) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("scout-report-gate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(report::STATE_FILE),
            br#"{"version":1,"topics":{},"feeds":{}}"#,
        )
        .unwrap();
        if let Some(secret) = pin {
            set_pin(&dir, secret);
        }
        dir
    }

    fn set_pin(dir: &Path, secret: [u8; 32]) {
        let pin = AccessPin::new("1234", secret, "2026-11-16T10:00:00.000Z").unwrap();
        std::fs::write(access::file_path(dir), pin.to_json()).unwrap();
    }

    struct Client<'a> {
        gate: &'a mut Gate,
        dir: &'a Path,
        ip: IpAddr,
        cookie: String,
        now: u64,
    }

    impl Client<'_> {
        fn send(&mut self, method: &str, target: &str, accept: &str, body: &str) -> Response {
            let req = Request {
                method,
                target,
                body: body.as_bytes(),
                client: self.ip,
                cookie: &self.cookie,
                accept,
            };
            let mut err = Vec::new();
            let resp = self.gate.respond(&req, self.dir, self.now, &mut err);
            assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
            resp
        }

        fn get(&mut self, target: &str) -> Response {
            self.send("GET", target, "*/*", "")
        }

        fn login(&mut self, pin: &str, next: &str) -> Response {
            self.send("POST", "/login", HTML, &format!("pin={pin}&next={next}"))
        }
    }

    fn client<'a>(gate: &'a mut Gate, dir: &'a Path, ip: IpAddr) -> Client<'a> {
        Client {
            gate,
            dir,
            ip,
            cookie: String::new(),
            now: NOW,
        }
    }

    fn header<'a>(r: &'a Response, name: &str) -> Option<&'a str> {
        r.headers
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }

    fn body(r: &Response) -> String {
        String::from_utf8_lossy(&r.body).into_owned()
    }

    fn is_pin_required(r: &Response) -> bool {
        r.status == 401
            && r.content_type == "application/json"
            && r.body == br#"{"error":"pin required"}"#
    }

    #[test]
    fn without_a_pin_everything_is_open() {
        let dir = user_data("open", None);
        let mut gate = Gate::default();
        let mut c = client(&mut gate, &dir, LAN);
        assert_eq!(c.get("/").status, 200);
        assert_eq!(c.get("/api/report").status, 200);
        let r = c.get("/login?next=%2Fswipe");
        assert_eq!((r.status, header(&r, "Location")), (303, Some("/swipe")));
        let r = c.login("0000", "/swipe");
        assert_eq!((r.status, header(&r, "Location")), (303, Some("/swipe")));
        assert_eq!(header(&r, "Set-Cookie"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn this_mac_never_needs_the_pin() {
        let dir = user_data("loopback", Some([1; 32]));
        let mut gate = Gate::default();
        for ip in [
            LOCAL,
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            IpAddr::V6(Ipv4Addr::LOCALHOST.to_ipv6_mapped()),
        ] {
            let mut c = client(&mut gate, &dir, ip);
            assert_eq!(c.get("/").status, 200, "{ip}");
            assert_eq!(c.get("/api/report").status, 200, "{ip}");
            assert_eq!(c.get("/api/deck").status, 200, "{ip}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn other_devices_are_sent_to_the_login_page() {
        let dir = user_data("gated", Some([1; 32]));
        let mut gate = Gate::default();
        let mut c = client(&mut gate, &dir, LAN);
        for (target, accept, location) in [
            ("/", "*/*", "/login?next=/"),
            ("/swipe?x=1#top", "", "/login?next=/swipe%3Fx%3D1"),
            ("/creators", "", "/login?next=/creators"),
            ("/api/report", HTML, "/login?next=/api/report"),
            ("/nowhere", HTML, "/login?next=/nowhere"),
        ] {
            let r = c.send("GET", target, accept, "");
            assert_eq!(r.status, 303, "{target}");
            assert_eq!(header(&r, "Location"), Some(location), "{target}");
            assert!(r.body.is_empty());
        }
        for (method, target) in [
            ("GET", "/api/report"),
            ("GET", "/api/deck?limit=3"),
            ("GET", "/frames/eroscripts/1/25.jpg"),
            ("POST", "/api/swipe"),
            ("POST", "/"),
            ("GET", "/api/creators"),
            ("HEAD", "/api/slr/deck"),
        ] {
            assert!(
                is_pin_required(&c.send(method, target, "application/json", "")),
                "{method} {target}"
            );
        }
        // A cookie or k= that isn't a valid token doesn't help.
        c.cookie = "funsync_access=v1.9999999999.00".into();
        assert!(is_pin_required(&c.get("/api/deck?k=nope")));
        // The login page itself is open.
        let r = c.get("/login?next=%2Fswipe%3Fx%3D1");
        assert_eq!(
            (r.status, r.content_type),
            (200, "text/html; charset=utf-8")
        );
        let page = body(&r);
        for part in [
            "<title>FunSync</title>",
            r#"inputmode="numeric""#,
            r#"name="pin""#,
            r#"name="next" value="/swipe?x=1""#,
            "prefers-color-scheme",
        ] {
            assert!(page.contains(part), "{part}");
        }
        assert!(!page.contains("{{") && !page.contains("Wrong PIN"));
        assert!(!page.contains("http"), "no external assets");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn logging_in_sets_a_30_day_cookie() {
        let dir = user_data("login", Some([1; 32]));
        let mut gate = Gate::default();
        let mut c = client(&mut gate, &dir, LAN);
        let r = c.login("1234", "%2Fswipe%3Fx%3D1");
        assert_eq!(
            (r.status, header(&r, "Location")),
            (303, Some("/swipe?x=1"))
        );
        let cookie = header(&r, "Set-Cookie").unwrap().to_string();
        assert!(cookie.ends_with("; Path=/; Max-Age=2592000; HttpOnly; SameSite=Lax"));
        let token = cookie
            .strip_prefix("funsync_access=")
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        assert!(token.starts_with(&format!("v1.{}.", NOW + 30 * 86400)));

        c.cookie = format!("theme=dark; funsync_access={token}");
        assert_eq!(c.get("/").status, 200);
        assert_eq!(c.get("/api/deck").status, 200);
        // The same token as k= (headsets' media fetches), without the cookie.
        c.cookie.clear();
        assert_eq!(c.get(&format!("/api/report?k={token}")).status, 200);
        assert!(is_pin_required(&c.get("/api/report")));
        // 30 days later it has run out.
        c.cookie = format!("funsync_access={token}");
        c.now = NOW + 30 * 86400;
        assert!(is_pin_required(&c.get("/api/report")));
        c.now = NOW;

        // Logging out clears the cookie.
        let r = c.send("POST", "/logout", HTML, "");
        assert_eq!((r.status, header(&r, "Location")), (303, Some("/")));
        assert_eq!(header(&r, "Set-Cookie"), Some(access::CLEAR_COOKIE));

        // A new PIN (new secret) logs every device out, and is read without a restart.
        assert_eq!(c.get("/api/report").status, 200);
        set_pin(&dir, [2; 32]);
        assert!(is_pin_required(&c.get("/api/report")));
        // Removing the PIN opens everything again.
        std::fs::remove_file(access::file_path(&dir)).unwrap();
        c.cookie.clear();
        assert_eq!(c.get("/api/report").status, 200);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_pins_are_refused_then_rate_limited() {
        let dir = user_data("wrong", Some([1; 32]));
        let mut gate = Gate::default();
        let mut c = client(&mut gate, &dir, LAN);
        let r = c.login("4321", "%2Fswipe");
        assert_eq!(r.status, 401);
        assert_eq!(header(&r, "Set-Cookie"), None);
        assert!(body(&r).contains("Wrong PIN"));
        assert!(body(&r).contains(r#"name="next" value="/swipe""#));
        // A right PIN clears the failures.
        assert_eq!(c.login("1234", "/").status, 303);
        for i in 0..MAX_FAILURES {
            c.now = NOW + i as u64 * 10;
            assert_eq!(c.login("0000", "/").status, 401, "try {i}");
        }
        // Locked out, even with the right PIN, until the first failure is 10 minutes old.
        c.now = NOW + 100;
        let r = c.login("1234", "/");
        assert_eq!(r.status, 429);
        assert_eq!(header(&r, "Retry-After"), Some("500"));
        assert_eq!(header(&r, "Set-Cookie"), None);
        assert!(body(&r).contains("Try again in 9 minutes"));
        c.now = NOW + FAILURE_WINDOW_SECS - 1;
        assert_eq!(c.login("1234", "/").status, 429);
        // Other devices aren't affected.
        c.ip = IpAddr::V4(Ipv4Addr::new(192, 168, 2, 31));
        assert_eq!(c.login("1234", "/").status, 303);
        c.ip = LAN;
        c.now = NOW + FAILURE_WINDOW_SECS;
        let r = c.login("1234", "/");
        assert_eq!(r.status, 303);
        assert!(header(&r, "Set-Cookie").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn next_stays_on_this_site() {
        for (next, safe) in [
            ("/swipe", "/swipe"),
            ("/swipe?x=1#top", "/swipe?x=1#top"),
            ("/", "/"),
            ("", "/"),
            ("swipe", "/"),
            ("//evil.example/x", "/"),
            ("/\\evil.example", "/"),
            ("https://evil.example/", "/"),
            ("/a b", "/"),
            ("/x\r\nSet-Cookie: a=b", "/"),
            ("/é", "/"),
            ("/login", "/"),
            ("/login?next=/swipe", "/"),
            ("/logout", "/"),
            ("/loginx", "/loginx"),
        ] {
            assert_eq!(safe_next(next), safe, "{next:?}");
        }
        let dir = user_data("next", Some([1; 32]));
        let mut gate = Gate::default();
        let mut c = client(&mut gate, &dir, LAN);
        let r = c.login("1234", "https%3A%2F%2Fevil.example%2F");
        assert_eq!(header(&r, "Location"), Some("/"));
        let r = c.login("1234", "%2F%2Fevil.example");
        assert_eq!(header(&r, "Location"), Some("/"));
        // What reaches the form is escaped.
        let page = body(&c.get("/login?next=/%22%3E%3Cscript%3E"));
        assert!(page.contains(r#"value="/&quot;&gt;&lt;script&gt;""#));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_pin_file_refuses_other_devices() {
        let dir = user_data("unreadable", None);
        std::fs::write(access::file_path(&dir), "{not json").unwrap();
        let mut gate = Gate::default();
        let mut err = Vec::new();
        let mut send = |ip, target: &str, accept: &str| {
            let req = Request {
                method: "GET",
                target,
                body: b"",
                client: ip,
                cookie: "",
                accept,
            };
            gate.respond(&req, &dir, NOW, &mut err)
        };
        let r = send(LAN, "/", HTML);
        assert_eq!(
            (r.status, r.content_type),
            (503, "text/plain; charset=utf-8")
        );
        assert!(body(&r).contains("access-pin.json"));
        let r = send(LAN, "/api/report", "*/*");
        assert_eq!((r.status, r.content_type), (503, "application/json"));
        let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v["error"], "access pin file unreadable");
        assert_eq!(send(LAN, "/login", HTML).status, 503);
        assert_eq!(send(LOCAL, "/api/report", "*/*").status, 200);
        // Logged once.
        let log = String::from_utf8(err).unwrap();
        assert_eq!(log.lines().count(), 1, "{log}");
        assert!(log.contains("access-pin.json"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decodes_and_encodes_form_values() {
        assert_eq!(form_value(b"pin=+12%334&next=%2Fa%2Bb", "pin"), " 1234");
        assert_eq!(form_value(b"pin=1&next=%2Fa%2Bb", "next"), "/a+b");
        assert_eq!(form_value(b"pin=1", "next"), "");
        assert_eq!(decode("%zz%4"), "%zz%4");
        assert_eq!(decode("%C3%A9"), "é");
        assert_eq!(encode("/swipe?x=1&y=a b"), "/swipe%3Fx%3D1%26y%3Da%20b");
    }
}
