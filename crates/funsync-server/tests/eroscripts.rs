//! End to end against a fake forum (a local HTTP server, never the network):
//! login with 2FA, search, a topic's scripts, saving a script next to its
//! video, a post picture as the poster.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use common::Kill;
use serde_json::{Value, json};

/// The fake forum: canned answers by method + path.
fn forum(listener: TcpListener) {
    for s in listener.incoming().flatten() {
        let mut r = BufReader::new(s);
        let mut line = String::new();
        if r.read_line(&mut line).is_err() {
            continue;
        }
        let mut headers = String::new();
        let mut len = 0;
        loop {
            let mut h = String::new();
            r.read_line(&mut h).unwrap();
            if h.trim().is_empty() {
                break;
            }
            if let Some(v) = h.to_lowercase().strip_prefix("content-length:") {
                len = v.trim().parse().unwrap();
            }
            headers.push_str(&h);
        }
        let mut body = vec![0; len];
        r.read_exact(&mut body).unwrap();
        let body = String::from_utf8_lossy(&body);
        let target = line.split(' ').nth(1).unwrap_or("");
        let path = target.split('?').next().unwrap();
        let authed = headers.contains("_t=TOKEN");
        // A zip upload: the forum redirects to its CDN, which serves bytes.
        if path == "/uploads/short-url/pack.zip" && authed {
            let mut s = r.into_inner();
            let _ = write!(
                s,
                "HTTP/1.1 302 Found\r\nLocation: /cdn/pack.zip\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            continue;
        }
        if path == "/cdn/pack.zip" {
            let zip = pack_zip();
            let mut s = r.into_inner();
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                zip.len()
            );
            let _ = s.write_all(&zip);
            continue;
        }
        let (status, cookie, payload): (u16, &str, Value) = match path {
            "/session/csrf.json" => (200, "_forum_session=s1", json!({"csrf": "tok"})),
            "/session" if !headers.contains("tok") || !headers.contains("_forum_session=s1") => {
                (403, "", json!({}))
            }
            "/session" if body.contains("second_factor_token=123456") => {
                (200, "_t=TOKEN", json!({"user": {"username": "me"}}))
            }
            "/session" => (
                200,
                "",
                json!({"error": "code", "reason": "invalid_second_factor"}),
            ),
            _ if !authed && !path.starts_with("/api/") && !path.starts_with("/pics/") => {
                (403, "", json!({}))
            }
            "/session/current.json" => (200, "", json!({"current_user": {"username": "me"}})),
            "/categories.json" => (
                200,
                "",
                json!({"category_list": {"categories": [{"id": 1, "slug": "scripts",
                "subcategory_list": [{"id": 15, "slug": "paid-scripts"}, {"id": 14, "slug": "free-scripts"}]}]}}),
            ),
            "/search.json" => (
                200,
                "",
                json!({"topics": [
                {"id": 7, "title": "Free scene", "slug": "free-scene", "category_id": 14},
                {"id": 8, "title": "Paid scene", "slug": "paid-scene", "category_id": 15}],
                "posts": [{"topic_id": 7, "username": "maker"}]}),
            ),
            "/t/7.json" => (
                200,
                "",
                json!({"title": "Free scene", "category_id": 14,
                "thumbnails": [{"url": "/pics/p.jpg", "width": 800}],
                "post_stream": {"stream": [1, 2], "posts": [{"id": 1, "post_number": 1, "username": "maker",
                    "cooked": "<a href=\"/uploads/short-url/main.funscript\">Scene.funscript</a> <a href=\"https://pixeldrain.com/l/L1\">pd</a> <a href=\"https://mega.nz/x\">m</a>"}]}}),
            ),
            "/t/7/posts.json" => (
                200,
                "",
                json!({"post_stream": {"posts": [{"id": 2, "post_number": 2, "username": "helper",
                "cooked": "<a href=\"upload://twist.funscript\">Scene.twist.funscript</a>"}]}}),
            ),
            "/api/list/L1" => (
                200,
                "",
                json!({"files": [{"id": "f1", "name": "Alt.funscript"}, {"id": "f2", "name": "v.mp4"}]}),
            ),
            "/uploads/short-url/main.funscript" | "/uploads/short-url/twist.funscript" => {
                (200, "", json!({"actions": [{"at": 0, "pos": 50}]}))
            }
            "/pics/p.jpg" => (200, "", json!("jpeg")),
            _ => (404, "", json!({})),
        };
        let payload = payload.to_string();
        let set = if cookie.is_empty() {
            String::new()
        } else {
            format!("Set-Cookie: {cookie}; path=/\r\n")
        };
        let mut s = r.into_inner();
        let _ = write!(
            s,
            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{set}Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
            payload.len()
        );
    }
}

/// A post's zip: the scene's script and its roll axis, plus macOS junk.
fn pack_zip() -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut z = zip::ZipWriter::new(&mut buf);
    for (name, body) in [
        (
            "Scene/Scene.funscript",
            r#"{"actions":[{"at":0,"pos":10}]}"#,
        ),
        ("Scene/Scene.roll.funscript", r#"{"actions":[]}"#),
        ("__MACOSX/Scene/._Scene.funscript", "junk"),
    ] {
        z.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(body.as_bytes()).unwrap();
    }
    z.finish().unwrap();
    buf.into_inner()
}

/// Import a library of `files` (relative paths, empty contents) and serve it
/// against the fake forum; the data dir gets `session` as its saved login.
fn serve(
    files: &[&str],
    fake_url: &str,
    session: Option<Value>,
) -> (Kill, u16, Vec<tempfile::TempDir>) {
    let lib = tempfile::tempdir().unwrap();
    for f in files {
        let p = lib.path().join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"{}").unwrap();
    }
    let data = common::import(&json!({"settings": {"library": {"sources": [
        {"id": "1", "name": "Lib", "path": lib.path(), "enabled": true}]}}}));
    if let Some(s) = session {
        std::fs::write(data.path().join("eroscripts-session.json"), s.to_string()).unwrap();
    }
    let (srv, port) = common::start(data.path(), &forum_env(fake_url));
    (srv, port, vec![lib, data])
}

/// The server's settings for the fake forum.
fn forum_env(fake_url: &str) -> [(&'static str, &str); 3] {
    [
        ("FUNSYNC_EROSCRIPTS_URL", fake_url),
        ("FUNSYNC_PIXELDRAIN_URL", fake_url),
        ("FUNSYNC_OWNS_LIBRARY", "1"),
    ]
}

/// Poll `path` until `ok` holds for its answer.
fn until(port: u16, path: &str, ok: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            let (_, v) = call(port, "GET", path, None);
            if ok(&v) {
                return v;
            }
        }
        assert!(Instant::now() < deadline, "{path} never got there");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn session_check_auto_match_and_zip() {
    let fake = TcpListener::bind("127.0.0.1:0").unwrap();
    let fake_url = format!("http://{}", fake.local_addr().unwrap());
    std::thread::spawn(move || forum(fake));
    let (_srv, port, dirs) = serve(
        &["A/Scene.mp4", "B/Done.mp4", "B/Done.funscript"],
        &fake_url,
        Some(json!({"cookie": "OLD", "username": "me"})),
    );
    // The saved session is checked at start: the forum refuses it.
    let st = until(port, "/api/status", |v| v["eroscripts"]["expired"] == true);
    assert_eq!(st["eroscripts"]["loggedIn"], true);

    let (_, v) = call(
        port,
        "POST",
        "/api/eroscripts/login",
        Some(json!({"cookie": "_t=TOKEN"})),
    );
    assert_eq!(v, json!({"ok": true, "username": "me"}));
    let (_, s) = call(port, "GET", "/api/eroscripts/session", None);
    assert_eq!(s["expired"], false);

    // Auto-match: the scriptless video is searched by its name.
    let videos = until(port, "/api/remote/videos", |v| {
        v["videos"].as_array().is_some_and(|a| a.len() == 2)
    });
    let scene = videos["videos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "Scene")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let m = until(port, "/api/eroscripts/matches", |v| {
        v["matches"].get(&scene).is_some()
    });
    assert_eq!(m["scriptless"], 1);
    assert_eq!(m["matches"][&scene]["count"], 2);
    assert_eq!(m["matches"][&scene]["top"]["title"], "Free scene");

    // A zip is unpacked next to the video.
    let (code, v) = call(
        port,
        "POST",
        "/api/eroscripts/download",
        Some(
            json!({"videoId": scene, "url": format!("{fake_url}/uploads/short-url/pack.zip"), "name": "pack.zip"}),
        ),
    );
    assert_eq!(code, 200, "{v}");
    let dir = dirs[0].path().join("A");
    assert_eq!(
        std::fs::read_to_string(dir.join("Scene.funscript")).unwrap(),
        r#"{"actions":[{"at":0,"pos":10}]}"#
    );
    assert!(dir.join("Scene.roll.funscript").is_file());
    assert!(!dir.join("._Scene.funscript").exists());
    let (code, _) = call(
        port,
        "POST",
        "/api/eroscripts/download",
        Some(
            json!({"videoId": scene, "url": format!("{fake_url}/uploads/short-url/pack.zip"), "name": "pack.zip"}),
        ),
    );
    assert_eq!(code, 409);
    let m = until(port, "/api/eroscripts/matches", |v| v["scriptless"] == 0);
    assert_eq!(m["matches"], json!({}));
}

fn call(port: u16, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let status = String::from_utf8_lossy(&buf[9..12]).parse().unwrap();
    (
        status,
        serde_json::from_slice(&buf[split + 4..]).unwrap_or(Value::Null),
    )
}

#[test]
fn login_search_topic_download_poster() {
    let fake = TcpListener::bind("127.0.0.1:0").unwrap();
    let fake_url = format!("http://{}", fake.local_addr().unwrap());
    std::thread::spawn(move || forum(fake));

    let lib = tempfile::tempdir().unwrap();
    let video = lib.path().join("A/Scene.mp4");
    std::fs::create_dir_all(video.parent().unwrap()).unwrap();
    std::fs::write(&video, b"0123456789").unwrap();
    let data = common::import(&json!({"settings": {"library": {"sources": [
        {"id": "1", "name": "Lib", "path": lib.path(), "enabled": true}]}}}));
    let (_srv, port) = common::start(data.path(), &forum_env(&fake_url));
    let deadline = Instant::now() + Duration::from_secs(10);
    let id = loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            let (_, v) = call(port, "GET", "/api/videos", None);
            if let Some(id) = v[0]["id"].as_str() {
                break id.to_string();
            }
        }
        assert!(Instant::now() < deadline, "server never listed the video");
        std::thread::sleep(Duration::from_millis(100));
    };

    let (code, _) = call(port, "GET", "/api/eroscripts/search?q=scene", None);
    assert_eq!(code, 401);
    let creds = json!({"username": "me", "password": "pw"});
    let (_, v) = call(port, "POST", "/api/eroscripts/login", Some(creds.clone()));
    assert_eq!(v["needsCode"], true);
    let mut with_code = creds;
    with_code["code"] = json!("123456");
    let (_, v) = call(port, "POST", "/api/eroscripts/login", Some(with_code));
    assert_eq!(v, json!({"ok": true, "username": "me"}));
    let session = data.path().join("eroscripts-session.json");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&session).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let (_, v) = call(port, "GET", "/api/eroscripts/session", None);
    assert_eq!(v["username"], "me");

    let (_, v) = call(port, "GET", "/api/eroscripts/search?q=scene", None);
    let paid: Vec<(&str, bool)> = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["title"].as_str().unwrap(), r["paid"] == true))
        .collect();
    assert_eq!(paid, [("Free scene", false), ("Paid scene", true)]);

    let (_, t) = call(port, "GET", "/api/eroscripts/topic/7", None);
    let att: Vec<(&str, &str, bool)> = t["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["name"].as_str().unwrap(),
                a["username"].as_str().unwrap(),
                a["byAuthor"] == true,
            )
        })
        .collect();
    assert_eq!(
        att,
        [
            ("Scene.funscript", "maker", true),
            ("Scene.twist.funscript", "helper", false)
        ]
    );
    assert_eq!(t["linked"][0]["name"], "Alt.funscript");
    assert_eq!(t["linked"][0]["url"], "https://pixeldrain.com/u/f1");
    assert_eq!(t["external"].as_array().unwrap().len(), 2); // pixeldrain + MEGA
    assert_eq!(t["preview"], format!("{fake_url}/pics/p.jpg"));

    for a in t["attachments"].as_array().unwrap() {
        let (code, v) = call(
            port,
            "POST",
            "/api/eroscripts/download",
            Some(json!({"videoId": id, "url": a["url"], "name": a["name"]})),
        );
        assert_eq!(code, 200, "{v}");
    }
    assert!(video.with_file_name("Scene.funscript").is_file());
    assert!(video.with_file_name("Scene.twist.funscript").is_file());
    let main = &t["attachments"][0];
    let (code, v) = call(
        port,
        "POST",
        "/api/eroscripts/download",
        Some(json!({"videoId": id, "url": main["url"], "name": main["name"]})),
    );
    assert_eq!((code, v["exists"].clone()), (409, json!(true)));
    let (code, _) = call(
        port,
        "POST",
        "/api/eroscripts/download",
        Some(json!({"videoId": id, "url": main["url"], "name": main["name"], "variant": "maker"})),
    );
    assert_eq!(code, 200);
    assert!(video.with_file_name("Scene (maker).funscript").is_file());
    let (_, v) = call(port, "GET", &format!("/api/videos/{id}"), None);
    assert!(v["funscriptPath"].as_str().is_some(), "rescanned: {v}");

    let (code, _) = call(
        port,
        "POST",
        "/api/eroscripts/preview",
        Some(json!({"videoId": id, "url": t["preview"]})),
    );
    assert_eq!(code, 200);
    let (_, v) = call(port, "GET", "/api/remote/videos", None);
    let thumb = v["videos"][0]["thumbUrl"].as_str().unwrap().to_string();
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        s,
        "GET {thumb} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    assert!(buf.ends_with(b"\"jpeg\""), "poster served");

    let (_, v) = call(port, "POST", "/api/eroscripts/logout", None);
    assert_eq!(v["ok"], true);
    assert!(!session.exists());
}
