//! HTTPS requests. The real client runs `curl` (no HTTP crate); everything
//! that talks to SLR or the forum takes an [`Http`] so tests can fake the
//! network. Secrets (cookies, tokens) go to curl on stdin as a config file,
//! never on its command line, where `ps` would show them.

use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Point the CLI at canned responses instead of the network (tests): a JSON
/// file `{"GET <url>": {"status": 200, "headers": [["name", "value"]],
/// "body": <JSON or string>}}`; unlisted requests get a 404.
pub const FAKE_ENV: &str = "SLR_SCOUT_FAKE_HTTP";

/// A browser User-Agent; SLR's API answers browsers.
pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

const TIMEOUT_S: u32 = 30;

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

impl Request {
    pub fn get(url: impl Into<String>) -> Self {
        Request {
            method: "GET",
            url: url.into(),
            headers: Vec::new(),
            body: None,
        }
    }

    pub fn post(url: impl Into<String>, body: impl Into<String>) -> Self {
        Request {
            method: "POST",
            url: url.into(),
            headers: Vec::new(),
            body: Some(body.into()),
        }
    }

    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: u16,
    /// Names in lower case.
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Response {
    /// Every value of a header (`set-cookie` comes several times).
    pub fn header_values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> {
        self.headers
            .iter()
            .filter(move |(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

pub trait Http {
    /// Send a request; `Err` only when no HTTP response came back.
    fn send(&self, req: &Request) -> Result<Response, String>;

    /// GET `req` into the file `dest` (redirects followed; send no secret
    /// headers here, they would follow the redirect); `Err` unless it ends
    /// in a 2xx with the whole body written.
    fn download(&self, req: &Request, dest: &Path) -> Result<(), String> {
        let res = self.send(req)?;
        if !(200..300).contains(&res.status) {
            return Err(format!("HTTP {}", res.status));
        }
        fs::write(dest, res.body.as_bytes()).map_err(|e| format!("{}: {e}", dest.display()))
    }
}

/// Why a request didn't give what was asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum NetError {
    /// HTTP 429.
    RateLimited(String),
    Failed(String),
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetError::RateLimited(m) => write!(f, "rate limited: {m}"),
            NetError::Failed(m) => f.write_str(m),
        }
    }
}

/// Send and parse a 2xx JSON body; 429 is [`NetError::RateLimited`].
/// Messages name the URL without its query (it may name a performer, never
/// a secret, but keeps lines short).
pub fn json(http: &dyn Http, req: &Request) -> Result<Value, NetError> {
    let res = http
        .send(req)
        .map_err(|e| NetError::Failed(format!("{}: {e}", short(&req.url))))?;
    parse_json(&res, &req.url)
}

fn short(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

/// A response's 2xx JSON body (see [`json`]).
pub fn parse_json(res: &Response, url: &str) -> Result<Value, NetError> {
    let where_ = short(url);
    if res.status == 429 {
        return Err(NetError::RateLimited(format!("{where_}: HTTP 429")));
    }
    if !(200..300).contains(&res.status) {
        return Err(NetError::Failed(format!("{where_}: HTTP {}", res.status)));
    }
    serde_json::from_str(&res.body)
        .map_err(|e| NetError::Failed(format!("{where_}: not JSON ({e})")))
}

/// Retries of a rate-limited request before giving up.
pub const RATE_LIMIT_RETRIES: u32 = 3;

/// [`json`] through the pacer: waits its turn, holds off when the host says
/// its rate limit is nearly spent (`x-ratelimit-remaining` /
/// `x-ratelimit-reset`), and retries a 429 after `Retry-After` (else the
/// reset, else a minute) up to [`RATE_LIMIT_RETRIES`] times when the pacer
/// retries (real network only).
pub fn paced_json(http: &dyn Http, pacer: &mut Pacer, req: &Request) -> Result<Value, NetError> {
    let mut attempt = 0;
    loop {
        pacer.wait();
        let res = http
            .send(req)
            .map_err(|e| NetError::Failed(format!("{}: {e}", short(&req.url))))?;
        pacer.observe(&res);
        if res.status == 429 && pacer.retries && attempt < RATE_LIMIT_RETRIES {
            attempt += 1;
            let secs = header_secs(&res, "retry-after")
                .or_else(|| header_secs(&res, "x-ratelimit-reset"))
                .unwrap_or(60);
            pacer.hold(Duration::from_secs(secs.min(600) + 1));
            continue;
        }
        return parse_json(&res, &req.url);
    }
}

fn header_secs(res: &Response, name: &str) -> Option<u64> {
    res.header_values(name).next()?.trim().parse().ok()
}

/// Keeps requests to one host at least `gap` apart, and off it while its
/// rate limit resets.
#[derive(Debug)]
pub struct Pacer {
    gap: Duration,
    last: Option<Instant>,
    until: Option<Instant>,
    /// Retry rate-limited requests (a real network: a nonzero gap).
    pub retries: bool,
}

impl Pacer {
    pub fn new(gap: Duration) -> Self {
        Pacer {
            gap,
            last: None,
            until: None,
            retries: !gap.is_zero(),
        }
    }

    /// Sleep until `gap` has passed since the previous call (and any hold).
    pub fn wait(&mut self) {
        if let Some(until) = self.until.take() {
            let now = Instant::now();
            if until > now {
                std::thread::sleep(until - now);
            }
        }
        if let Some(last) = self.last {
            let since = last.elapsed();
            if since < self.gap {
                std::thread::sleep(self.gap - since);
            }
        }
        self.last = Some(Instant::now());
    }

    /// No request before `d` from now.
    pub fn hold(&mut self, d: Duration) {
        let until = Instant::now() + d;
        self.until = Some(self.until.map_or(until, |u| u.max(until)));
    }

    /// Hold off until the reset when the response says at most one request
    /// is left in the window.
    pub fn observe(&mut self, res: &Response) {
        if let (Some(left), Some(reset)) = (
            header_secs(res, "x-ratelimit-remaining"),
            header_secs(res, "x-ratelimit-reset"),
        ) && left <= 1
        {
            self.hold(Duration::from_secs(reset.min(600) + 1));
        }
    }
}

/// The real client: one `curl` per request.
pub struct Curl;

impl Http for Curl {
    fn send(&self, req: &Request) -> Result<Response, String> {
        let mut config = String::new();
        config.push_str(&format!("url = {}\n", quote(&req.url)));
        config.push_str(&format!("request = {}\n", quote(req.method)));
        config.push_str(&format!("user-agent = {}\n", quote(USER_AGENT)));
        for (name, value) in &req.headers {
            config.push_str(&format!(
                "header = {}\n",
                quote(&format!("{name}: {value}"))
            ));
        }
        if let Some(body) = &req.body {
            config.push_str(&format!("data-binary = {}\n", quote(body)));
        }
        let mut child = Command::new("curl")
            .args(["-s", "-S", "--compressed", "-D", "-", "--max-time"])
            .arg(TIMEOUT_S.to_string())
            .args(["-K", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("can't run curl: {e}"))?;
        child
            .stdin
            .take()
            .ok_or("curl: no stdin")?
            .write_all(config.as_bytes())
            .map_err(|e| format!("curl: {e}"))?;
        let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
        if !out.status.success() {
            let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(if msg.is_empty() {
                format!("curl exited with {}", out.status)
            } else {
                msg
            });
        }
        parse_dump(&String::from_utf8_lossy(&out.stdout))
    }

    /// curl straight into the file, resuming a partial one (`-C -`), with
    /// retries; the URL goes on stdin like every other value.
    fn download(&self, req: &Request, dest: &Path) -> Result<(), String> {
        let mut config = String::new();
        config.push_str(&format!("url = {}\n", quote(&req.url)));
        config.push_str(&format!("user-agent = {}\n", quote(USER_AGENT)));
        config.push_str(&format!("output = {}\n", quote(&dest.to_string_lossy())));
        for (name, value) in &req.headers {
            config.push_str(&format!(
                "header = {}\n",
                quote(&format!("{name}: {value}"))
            ));
        }
        let mut child = Command::new("curl")
            .args(["-s", "-S", "-f", "-L", "-C", "-", "--retry", "3"])
            .args(["--speed-limit", "10240", "--speed-time", "120"])
            .args(["-K", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("can't run curl: {e}"))?;
        child
            .stdin
            .take()
            .ok_or("curl: no stdin")?
            .write_all(config.as_bytes())
            .map_err(|e| format!("curl: {e}"))?;
        let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
        if out.status.success() {
            return Ok(());
        }
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if msg.is_empty() {
            format!("curl exited with {}", out.status)
        } else {
            msg
        })
    }
}

/// A curl config string: double-quoted, `\` and `"` escaped, newlines as
/// `\n`.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// curl's `-D -` output: header block(s), a blank line, the body. Interim
/// (1xx) blocks are skipped.
pub fn parse_dump(raw: &str) -> Result<Response, String> {
    let mut rest = raw;
    loop {
        let (head, body) = rest
            .split_once("\r\n\r\n")
            .or_else(|| rest.split_once("\n\n"))
            .unwrap_or((rest, ""));
        let mut lines = head.lines();
        let status_line = lines.next().unwrap_or("");
        let status: u16 = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| "curl: no HTTP status in the response".to_string())?;
        if (100..200).contains(&status) && !body.is_empty() {
            rest = body;
            continue;
        }
        let headers = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        return Ok(Response {
            status,
            headers,
            body: body.to_string(),
        });
    }
}

/// Canned responses from a fixture file (see [`FAKE_ENV`]).
pub struct Fake {
    responses: HashMap<String, Response>,
}

impl Fake {
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let map: serde_json::Map<String, Value> =
            serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut responses = HashMap::new();
        for (key, v) in map {
            let status = v.get("status").and_then(Value::as_u64).unwrap_or(200) as u16;
            let headers = v
                .get("headers")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|h| {
                    Some((
                        h.get(0)?.as_str()?.to_ascii_lowercase(),
                        h.get(1)?.as_str()?.to_string(),
                    ))
                })
                .collect();
            let body = match v.get("body") {
                Some(Value::String(s)) => s.clone(),
                Some(other) => other.to_string(),
                None => String::new(),
            };
            responses.insert(
                key,
                Response {
                    status,
                    headers,
                    body,
                },
            );
        }
        Ok(Fake { responses })
    }
}

impl Http for Fake {
    fn send(&self, req: &Request) -> Result<Response, String> {
        Ok(self
            .responses
            .get(&format!("{} {}", req.method, req.url))
            .cloned()
            .unwrap_or(Response {
                status: 404,
                headers: Vec::new(),
                body: "{}".into(),
            }))
    }
}

#[cfg(test)]
pub mod testing {
    //! An in-memory [`Http`] for unit tests.
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct Canned {
        pub responses: HashMap<String, Response>,
        pub seen: RefCell<Vec<Request>>,
    }

    impl Canned {
        pub fn on(&mut self, method: &str, url: &str, status: u16, body: &str) {
            self.with_headers(method, url, status, &[], body);
        }

        pub fn with_headers(
            &mut self,
            method: &str,
            url: &str,
            status: u16,
            headers: &[(&str, &str)],
            body: &str,
        ) {
            self.responses.insert(
                format!("{method} {url}"),
                Response {
                    status,
                    headers: headers
                        .iter()
                        .map(|(n, v)| (n.to_string(), v.to_string()))
                        .collect(),
                    body: body.to_string(),
                },
            );
        }
    }

    impl Http for Canned {
        fn send(&self, req: &Request) -> Result<Response, String> {
            self.seen.borrow_mut().push(req.clone());
            self.responses
                .get(&format!("{} {}", req.method, req.url))
                .cloned()
                .ok_or_else(|| format!("no canned response for {} {}", req.method, req.url))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_curl_dumps() {
        let r = parse_dump(
            "HTTP/1.1 100 Continue\r\n\r\nHTTP/2 200\r\nSet-Cookie: a=1\r\nset-cookie: b=2\r\ncontent-type: application/json\r\n\r\n{\"x\":1}",
        )
        .unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(
            r.header_values("set-cookie").collect::<Vec<_>>(),
            ["a=1", "b=2"]
        );
        assert_eq!(r.body, "{\"x\":1}");
        assert_eq!(parse_dump("HTTP/2 401\r\n\r\n").unwrap().status, 401);
        assert!(parse_dump("garbage").is_err());
    }

    #[test]
    fn quotes_curl_config_values() {
        assert_eq!(quote(r#"a "b" \c"#), r#""a \"b\" \\c""#);
        assert_eq!(quote("x\ny"), r#""x\ny""#);
    }

    #[test]
    fn json_maps_statuses() {
        let mut http = testing::Canned::default();
        http.on("GET", "https://h/ok?q=1", 200, r#"{"a":1}"#);
        http.on("GET", "https://h/slow", 429, "");
        http.on("GET", "https://h/gone", 500, "");
        http.on("GET", "https://h/html", 200, "<html>");
        assert_eq!(
            json(&http, &Request::get("https://h/ok?q=1")).unwrap()["a"],
            1
        );
        assert!(matches!(
            json(&http, &Request::get("https://h/slow")),
            Err(NetError::RateLimited(_))
        ));
        assert_eq!(
            json(&http, &Request::get("https://h/gone")),
            Err(NetError::Failed("https://h/gone: HTTP 500".into()))
        );
        assert!(json(&http, &Request::get("https://h/html")).is_err());
        assert!(json(&http, &Request::get("https://h/none")).is_err());
    }

    fn response(status: u16, headers: &[(&str, &str)]) -> Response {
        Response {
            status,
            headers: headers
                .iter()
                .map(|(n, v)| (n.to_string(), v.to_string()))
                .collect(),
            body: "{}".into(),
        }
    }

    #[test]
    fn holds_off_when_the_rate_limit_is_spent() {
        let mut p = Pacer::new(Duration::ZERO);
        p.observe(&response(
            200,
            &[("x-ratelimit-remaining", "15"), ("x-ratelimit-reset", "53")],
        ));
        assert!(p.until.is_none());
        p.observe(&response(
            200,
            &[("x-ratelimit-remaining", "1"), ("x-ratelimit-reset", "53")],
        ));
        let held = p.until.unwrap() - Instant::now();
        assert!(held > Duration::from_secs(50) && held <= Duration::from_secs(54));
        // A shorter hold doesn't cut a longer one.
        p.hold(Duration::from_secs(1));
        assert!(p.until.unwrap() - Instant::now() > Duration::from_secs(50));
    }

    #[test]
    fn retries_a_rate_limited_request_on_a_real_network_only() {
        let mut http = testing::Canned::default();
        http.with_headers("GET", "https://h/slow", 429, &[("retry-after", "0")], "");
        let req = Request::get("https://h/slow");
        // Canned network (zero gap): no retry.
        let mut p = Pacer::new(Duration::ZERO);
        assert!(matches!(
            paced_json(&http, &mut p, &req),
            Err(NetError::RateLimited(_))
        ));
        assert_eq!(http.seen.borrow().len(), 1);
        // Retrying: RATE_LIMIT_RETRIES more tries, a second apart.
        p.retries = true;
        let started = Instant::now();
        assert!(matches!(
            paced_json(&http, &mut p, &req),
            Err(NetError::RateLimited(_))
        ));
        assert_eq!(http.seen.borrow().len(), 2 + RATE_LIMIT_RETRIES as usize);
        assert!(started.elapsed() >= Duration::from_secs(RATE_LIMIT_RETRIES as u64));
    }
}
