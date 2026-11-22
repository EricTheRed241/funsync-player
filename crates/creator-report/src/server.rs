//! The HTTP side, on scout-report's plumbing (scout_report::server::serve_site) behind its
//! access-PIN gate and login page (scout_report::gate):
//!
//! - `GET /creators` (the Creators tab), `GET /swiped` (Swiped) and `GET /collection`
//!   (Collection): the same page. `GET /` redirects to `/creators`. The yes creators' posts
//!   are swiped on the swipe deck (scout-report, scout_report::creators).
//! - `GET /api/creators/deck?seed=N&offset=N&limit=N`: a page of the deck (crate::deck).
//! - `GET /api/creators/swiped`: the swiped creators' cards, newest swipe first.
//!   `GET /api/creators/collection`: the yes creators' cards. `GET /api/summary`: the counts.
//! - `POST /api/creators/swipe {slug, choice: yes|no}`, `POST /api/creators/swipe/undo {slug}`:
//!   the only writes (crate::swipes).
//! - `GET /pictures/<slug>/<file>`: a picture (scout_report::creator_pictures).

use crate::creators::{self, Cache, Creators};
use crate::deck::{self, Page};
use crate::swipes::{self, Choice, Swipes};
use scout_report::creator_pictures as pictures;
use scout_report::gate::Gate;
use scout_report::server::{
    PICTURE_CACHE, Response, html, json_ok, serve_site, text, with_preloader,
};
use serde_json::{Value, json};
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// The page: every tab, HTML, CSS and JS in one file (the shared picture lookahead inlined),
/// no external scripts.
pub static PAGE: LazyLock<String> = LazyLock::new(|| with_preloader(include_str!("creators.html")));
/// The site's name (log lines) and its pages, for the gate.
pub const NAME: &str = "creator-report";
pub const PAGES: &[&str] = &["/", "/creators", "/swiped", "/collection"];

/// The site's state between requests: userData and creators.json as last read.
#[derive(Debug)]
pub struct App {
    user_data: PathBuf,
    creators: Cache,
}

impl App {
    pub fn new(user_data: &Path) -> App {
        App {
            user_data: user_data.to_path_buf(),
            creators: Cache::default(),
        }
    }

    /// Answer one request. HEAD gets GET's headers (the caller drops the body).
    pub fn route(&mut self, method: &str, target: &str, body: &[u8]) -> Response {
        let target = target.split('#').next().unwrap_or("");
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        match (method, path) {
            ("GET" | "HEAD", "/") => {
                let mut resp = text(303, "");
                let location = if query.is_empty() {
                    "/creators".to_string()
                } else {
                    format!("/creators?{query}")
                };
                resp.headers.push(("Location", location));
                resp
            }
            ("GET" | "HEAD", "/creators" | "/swiped" | "/collection") => html(&PAGE),
            ("GET" | "HEAD", "/api/creators/deck") => match page(query) {
                Ok(page) => {
                    let (creators, swipes, problems) = self.state();
                    json_ok(&deck::creator_deck(creators, &swipes, &page, problems))
                }
                Err(e) => text(400, &e),
            },
            ("GET" | "HEAD", "/api/creators/swiped") => {
                let (creators, swipes, problems) = self.state();
                json_ok(&deck::swiped_list(creators, &swipes, problems))
            }
            ("GET" | "HEAD", "/api/creators/collection") => {
                let (creators, swipes, problems) = self.state();
                json_ok(&deck::collection(creators, &swipes, problems))
            }
            ("GET" | "HEAD", "/api/summary") => {
                let (creators, swipes, problems) = self.state();
                json_ok(&deck::summary(creators, &swipes, problems))
            }
            ("GET" | "HEAD", p) if p.starts_with("/pictures/") => picture(&self.user_data, p),
            ("POST", "/api/creators/swipe") => self.creator_swipe(body),
            ("POST", "/api/creators/swipe/undo") => match field(body, "slug") {
                Ok(slug) => match swipes::undo_creator(&self.user_data, &slug) {
                    Ok(removed) => json_ok(&json!({"slug": slug, "removed": removed})),
                    Err(e) => text(500, &format!("{e}\n")),
                },
                Err(resp) => resp,
            },
            _ => text(404, "Not found\n"),
        }
    }

    /// creators.json (None when missing or broken) and the swipes (empty when broken),
    /// with what couldn't be read.
    fn state(&mut self) -> (Option<&Creators>, Swipes, Vec<String>) {
        let mut problems = Vec::new();
        let creators = self.creators.get(&self.user_data).unwrap_or_else(|e| {
            problems.push(e);
            None
        });
        let swipes = swipes::read(&self.user_data).unwrap_or_else(|e| {
            problems.push(e);
            Swipes::default()
        });
        (creators, swipes, problems)
    }

    /// `POST /api/creators/swipe {slug, choice}`: a creator in creators.json, yes or no.
    fn creator_swipe(&mut self, body: &[u8]) -> Response {
        let (slug, choice) = match field(body, "slug").and_then(|slug| {
            let choice = field(body, "choice")
                .ok()
                .and_then(|c| Choice::parse(&c))
                .ok_or_else(|| text(400, "choice must be \"yes\" or \"no\"\n"))?;
            Ok((slug, choice))
        }) {
            Ok(r) => r,
            Err(resp) => return resp,
        };
        match self.creators.get(&self.user_data) {
            Err(e) => return text(500, &format!("{e}\n")),
            Ok(c) if !c.is_some_and(|c| c.creators.contains_key(&slug)) => {
                return text(400, &format!("{slug} is not in {}\n", creators::FILE));
            }
            Ok(_) => {}
        }
        match swipes::set_creator(
            &self.user_data,
            &slug,
            choice,
            video_tags::timefmt::now_iso(),
        ) {
            Ok(s) => json_ok(&json!({"slug": slug, "choice": s.choice, "at": s.at})),
            Err(e) => text(500, &format!("{e}\n")),
        }
    }
}

/// The deck request: `seed=N` (shuffle a little), `offset=N`
/// (cards to skip), `limit=N` (default deck::DEFAULT_LIMIT, at most deck::MAX_LIMIT).
fn page(query: &str) -> Result<Page, String> {
    let mut p = Page::default();
    let number = |key: &str, v: &str| {
        v.parse::<u64>()
            .map_err(|_| format!("{key} must be a whole number, got {v:?}\n"))
    };
    for (key, v) in query.split('&').filter_map(|p| p.split_once('=')) {
        match key {
            "seed" if !v.is_empty() => p.seed = Some(number(key, v)?),
            "offset" => p.offset = usize::try_from(number(key, v)?).unwrap_or(usize::MAX),
            "limit" => {
                p.limit = usize::try_from(number(key, v)?)
                    .map_or(deck::MAX_LIMIT, |n| n.min(deck::MAX_LIMIT));
            }
            _ => {}
        }
    }
    Ok(p)
}

/// A non-empty string field of the JSON object a POST carries.
fn field(body: &[u8], name: &str) -> Result<String, Response> {
    let req: Value = serde_json::from_slice(body)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| text(400, "body must be a JSON object\n"))?;
    req.get(name)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| text(400, &format!("{name} must be a non-empty string\n")))
}

/// `GET /pictures/<slug>/<file>`: the file (crate::pictures::file_for), else 404.
fn picture(user_data: &Path, path: &str) -> Response {
    match pictures::file_for(user_data, path).map(|(file, kind)| (std::fs::read(file), kind)) {
        Some((Ok(body), content_type)) => Response {
            status: 200,
            content_type,
            cache: PICTURE_CACHE,
            body,
            headers: Vec::new(),
        },
        _ => text(404, "Not found\n"),
    }
}

/// Serve the site behind the access-PIN gate, forever.
pub fn serve(listener: &TcpListener, user_data: &Path, err: &mut dyn Write) {
    let mut app = App::new(user_data);
    serve_site(listener, user_data, Gate::new(NAME, PAGES), err, |r| {
        app.route(r.method, r.target, r.body)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use scout_report::gate::Request;
    use std::net::{IpAddr, Ipv4Addr};
    use video_tags::access::{self, AccessPin};

    fn user_data(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "creator-report-server-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const SAMPLE: &str = r#"{"version":1,"updatedAt":"2026-11-18T01:40:00Z","creators":{
        "ann":{"name":"Ann","score":5.0,"pictures":[{"file":"ig-1.jpg","source":"instagram"}],
          "posts":[{"kind":"eroscripts","id":"1","title":"One","free":true,"downloadable":true,"score":7.0},
                   {"kind":"eroscripts","id":"2","title":"Paid","free":false,"downloadable":false}]},
        "bea":{"name":"Bea","score":6.0,"posts":[{"kind":"faptap","id":"2122637633308856321","title":"Tap","free":true,"downloadable":true,"score":3.0}]}}}"#;

    fn with_sample(name: &str) -> PathBuf {
        let dir = user_data(name);
        std::fs::write(dir.join(creators::FILE), SAMPLE).unwrap();
        dir
    }

    fn get(app: &mut App, target: &str) -> Value {
        let r = app.route("GET", target, b"");
        assert_eq!(
            r.status,
            200,
            "{target}: {}",
            String::from_utf8_lossy(&r.body)
        );
        assert_eq!(r.content_type, "application/json");
        serde_json::from_slice(&r.body).unwrap()
    }

    fn post(app: &mut App, target: &str, body: Value) -> Response {
        app.route("POST", target, body.to_string().as_bytes())
    }

    fn slugs(v: &Value) -> Vec<String> {
        v["cards"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["slug"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn serves_every_tab_the_same_page_and_redirects_the_root() {
        let dir = user_data("pages");
        let mut app = App::new(&dir);
        for p in ["/creators", "/swiped", "/collection", "/swiped?x=1#y"] {
            let r = app.route("GET", p, b"");
            assert_eq!(
                (r.status, r.content_type),
                (200, "text/html; charset=utf-8")
            );
            assert_eq!(r.body, PAGE.as_bytes());
        }
        for (target, to) in [("/", "/creators"), ("/?k=v1.1.ab", "/creators?k=v1.1.ab")] {
            let r = app.route("GET", target, b"");
            assert_eq!(r.status, 303, "{target}");
            assert_eq!(r.headers, [("Location", to.to_string())]);
        }
        // The Posts tab moved to the swipe deck.
        for gone in ["/nowhere", "/posts", "/api/posts/deck"] {
            assert_eq!(app.route("GET", gone, b"").status, 404, "{gone}");
        }
        assert_eq!(app.route("POST", "/creators", b"").status, 404);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_creators_json_the_decks_are_empty_and_say_so() {
        let dir = user_data("missing");
        let mut app = App::new(&dir);
        for target in ["/api/creators/deck", "/api/summary"] {
            let v = get(&mut app, target);
            assert_eq!(v["missing"], true, "{target}");
            assert_eq!(v["problems"], json!([]));
        }
        for target in ["/api/creators/swiped", "/api/creators/collection"] {
            let v = get(&mut app, target);
            assert_eq!((&v["cards"], &v["missing"]), (&json!([]), &json!(true)));
        }
        let r = post(
            &mut app,
            "/api/creators/swipe",
            json!({"slug": "ann", "choice": "yes"}),
        );
        assert_eq!(r.status, 400);
        // A broken file is a problem, not "missing".
        std::fs::write(dir.join(creators::FILE), "{oops").unwrap();
        let v = get(&mut app, "/api/creators/deck");
        assert_eq!(v["missing"], false);
        assert!(
            v["problems"][0]
                .as_str()
                .unwrap()
                .starts_with(creators::FILE)
        );
        let r = post(
            &mut app,
            "/api/creators/swipe",
            json!({"slug": "ann", "choice": "yes"}),
        );
        assert_eq!(r.status, 500);
        assert!(!dir.join(swipes::FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn swiping_creators_and_counting_their_posts() {
        let dir = with_sample("swipe");
        let mut app = App::new(&dir);
        let v = get(&mut app, "/api/creators/deck?seed=3&offset=0&limit=10");
        assert_eq!(slugs(&v), ["bea", "ann"]);
        assert_eq!(
            (v["remaining"].as_u64(), v["yes"].as_u64()),
            (Some(2), Some(0))
        );
        assert_eq!(v["cards"][1]["pictures"][0]["src"], "pictures/ann/ig-1.jpg");
        assert_eq!(v["cards"][1]["counts"]["eroscriptsPaid"], 1);
        assert_eq!(
            slugs(&get(&mut app, "/api/creators/deck?offset=1")),
            ["ann"]
        );
        assert_eq!(
            get(&mut app, "/api/creators/collection")["cards"],
            json!([])
        );

        let r = post(
            &mut app,
            "/api/creators/swipe",
            json!({"slug": "ann", "choice": "yes"}),
        );
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(
            (v["slug"].as_str(), v["choice"].as_str()),
            (Some("ann"), Some("yes"))
        );
        assert!(v["at"].as_str().unwrap().ends_with('Z'));
        let v = get(&mut app, "/api/creators/deck");
        assert_eq!(slugs(&v), ["bea"]);
        assert_eq!(v["yes"], 1);

        // Ann's downloadable post is to swipe (on the swipe deck); the deck's want lands in
        // the same file, which the site keeps when it writes.
        let yes = get(&mut app, "/api/creators/collection");
        assert_eq!(
            (&yes["cards"][0]["slug"], &yes["cards"][0]["unswiped"]),
            (&json!("ann"), &json!(1))
        );
        let mut file: Value =
            serde_json::from_slice(&std::fs::read(dir.join(swipes::FILE)).unwrap()).unwrap();
        file["posts"]["eroscripts:1"] = json!({"choice": "want", "at": "t", "creator": "ann"});
        std::fs::write(dir.join(swipes::FILE), file.to_string()).unwrap();
        let sum = get(&mut app, "/api/summary");
        assert_eq!(
            sum["creators"],
            json!({"total": 2, "yes": 1, "no": 0, "unswiped": 1})
        );
        let yes = get(&mut app, "/api/creators/collection");
        assert_eq!(yes["cards"][0]["slug"], "ann");
        assert_eq!(
            (&yes["cards"][0]["unswiped"], &yes["cards"][0]["wanted"]),
            (&json!(0), &json!(1))
        );
        let swiped = get(&mut app, "/api/creators/swiped");
        assert_eq!(slugs(&swiped), ["ann"]);
        assert_eq!(swiped["cards"][0]["choice"], "yes");
        assert!(swiped["cards"][0]["at"].is_string());

        let on_disk: Value =
            serde_json::from_slice(&std::fs::read(dir.join(swipes::FILE)).unwrap()).unwrap();
        assert_eq!(on_disk["creators"]["ann"]["choice"], "yes");
        assert_eq!(on_disk["posts"]["eroscripts:1"]["creator"], "ann");

        // Changing Ann to no keeps her post's swipe; she leaves the collection.
        post(
            &mut app,
            "/api/creators/swipe",
            json!({"slug": "ann", "choice": "no"}),
        );
        assert_eq!(
            get(&mut app, "/api/creators/collection")["cards"],
            json!([])
        );
        let on_disk: Value =
            serde_json::from_slice(&std::fs::read(dir.join(swipes::FILE)).unwrap()).unwrap();
        assert_eq!(on_disk["posts"]["eroscripts:1"]["choice"], "want");
        let r = post(&mut app, "/api/creators/swipe/undo", json!({"slug": "ann"}));
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v["removed"], true);
        let r = post(&mut app, "/api/creators/swipe/undo", json!({"slug": "ann"}));
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v["removed"], false);
        assert_eq!(get(&mut app, "/api/creators/deck")["remaining"], 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_bad_swipes_and_queries() {
        let dir = with_sample("bad");
        let mut app = App::new(&dir);
        for (target, body) in [
            (
                "/api/creators/swipe",
                json!({"slug": "ann", "choice": "want"}),
            ),
            (
                "/api/creators/swipe",
                json!({"slug": "nobody", "choice": "yes"}),
            ),
            ("/api/creators/swipe", json!({"choice": "yes"})),
            ("/api/creators/swipe", json!(["ann"])),
            ("/api/creators/swipe/undo", json!({"slug": ""})),
        ] {
            assert_eq!(
                post(&mut app, target, body.clone()).status,
                400,
                "{target} {body}"
            );
        }
        for q in ["seed=x", "offset=-1", "limit=1.5"] {
            assert_eq!(
                app.route("GET", &format!("/api/creators/deck?{q}"), b"")
                    .status,
                400,
                "{q}"
            );
        }
        // Limits are capped; unknown keys and an empty seed are ignored.
        assert_eq!(
            page("limit=5000&seed=&x=1"),
            Ok(Page {
                seed: None,
                offset: 0,
                limit: deck::MAX_LIMIT
            })
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rereads_creators_json_when_it_changes() {
        let dir = with_sample("reread");
        let mut app = App::new(&dir);
        assert_eq!(get(&mut app, "/api/creators/deck")["remaining"], 2);
        std::fs::write(
            dir.join(creators::FILE),
            r#"{"creators":{"cat":{"name":"Cat","score":1.0}}}"#,
        )
        .unwrap();
        assert_eq!(slugs(&get(&mut app, "/api/creators/deck")), ["cat"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serves_picture_files_only() {
        let dir = user_data("pictures");
        let folder = dir.join(pictures::DIR).join("ann");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("ig-1.jpg"), b"\xff\xd8jpeg").unwrap();
        std::fs::write(folder.join("notes.txt"), b"x").unwrap();
        std::fs::write(dir.join("secret.jpg"), b"x").unwrap();
        let mut app = App::new(&dir);
        let r = app.route("GET", "/pictures/ann/ig-1.jpg", b"");
        assert_eq!(
            (r.status, r.content_type, r.cache),
            (200, "image/jpeg", PICTURE_CACHE)
        );
        assert_eq!(r.body, b"\xff\xd8jpeg");
        for bad in [
            "/pictures/ann/notes.txt",
            "/pictures/ann/missing.jpg",
            "/pictures/ann/../../secret.jpg",
            "/pictures/../secret.jpg",
            "/pictures/ann/%2e%2e%2fsecret.jpg",
        ] {
            assert_eq!(app.route("GET", bad, b"").status, 404, "{bad}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_gate_guards_both_tabs_and_every_api() {
        let dir = with_sample("gate");
        let pin = AccessPin::new("1234", [1; 32], "2026-11-16T10:00:00.000Z").unwrap();
        std::fs::write(access::file_path(&dir), pin.to_json()).unwrap();
        let mut gate = Gate::new(NAME, PAGES);
        let mut app = App::new(&dir);
        let now = 1_763_290_000;
        let mut send = |ip: IpAddr, method: &str, target: &str, cookie: &str| {
            let req = Request {
                method,
                target,
                body: b"",
                client: ip,
                cookie,
                accept: "*/*",
            };
            let mut err = Vec::new();
            gate.respond(&req, &dir, now, &mut err, |r| {
                app.route(r.method, r.target, r.body)
            })
        };
        let lan = IpAddr::V4(Ipv4Addr::new(192, 168, 2, 30));
        let location = |r: &Response| {
            r.headers
                .iter()
                .find(|(n, _)| *n == "Location")
                .map(|(_, v)| v.clone())
        };
        for (target, next) in [
            ("/", "/login?next=/"),
            ("/creators", "/login?next=/creators"),
            ("/swiped", "/login?next=/swiped"),
            ("/collection", "/login?next=/collection"),
        ] {
            let r = send(lan, "GET", target, "");
            assert_eq!((r.status, location(&r).as_deref()), (303, Some(next)));
        }
        for (method, target) in [
            ("GET", "/api/creators/deck"),
            ("GET", "/api/summary"),
            ("GET", "/api/creators/swiped"),
            ("GET", "/api/creators/collection"),
            ("GET", "/pictures/ann/ig-1.jpg"),
            ("POST", "/api/creators/swipe"),
            ("POST", "/api/creators/swipe/undo"),
        ] {
            assert_eq!(
                send(lan, method, target, "").status,
                401,
                "{method} {target}"
            );
        }
        // The login page is scout-report's.
        let r = send(lan, "GET", "/login?next=/swiped", "");
        assert_eq!(r.status, 200);
        assert!(String::from_utf8_lossy(&r.body).contains("access PIN"));
        // With the cookie, or from this Mac, it's open.
        let cookie = format!("{}={}", access::COOKIE_NAME, pin.token(now));
        assert_eq!(send(lan, "GET", "/api/creators/deck", &cookie).status, 200);
        assert_eq!(send(lan, "GET", "/collection", &cookie).status, 200);
        let local = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert_eq!(send(local, "GET", "/api/summary", "").status, 200);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
