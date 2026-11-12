//! End to end through the binary: a fake backend (a TcpListener thread
//! serving the video list and VR flags, recording POSTs), a temp userData
//! and library with tiny real videos made by ffmpeg, and a shell-script
//! judge that writes fixed results.

use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "age-screen-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

type Posts = Arc<Mutex<Vec<(String, Value)>>>;

/// Serve `videos` and `vr` until the test ends; returns the URL and the
/// POSTs received (path, JSON body).
fn fake_backend(videos: Value, vr: Value) -> (String, Posts) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let posts: Posts = Arc::default();
    let seen = posts.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { continue };
            let mut reader = BufReader::new(conn.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some((k, v)) = line.split_once(':')
                    && k.eq_ignore_ascii_case("content-length")
                {
                    length = v.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let mut parts = first.split_whitespace();
            let (method, path) = (parts.next().unwrap(), parts.next().unwrap().to_string());
            let answer = match (method, path.as_str()) {
                ("GET", "/api/remote/videos") => json!({ "videos": videos }),
                ("GET", "/api/media/vr-flags") => json!({ "flags": vr, "pending": 0 }),
                ("POST", _) => {
                    seen.lock()
                        .unwrap()
                        .push((path, serde_json::from_slice(&body).unwrap()));
                    json!({ "ok": true })
                }
                _ => Value::Null,
            };
            let text = answer.to_string();
            let status = if answer.is_null() {
                "404 Not Found"
            } else {
                "200 OK"
            };
            let _ = write!(
                conn,
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                text.len()
            );
        }
    });
    (url, posts)
}

fn has_ffmpeg() -> bool {
    ["ffmpeg", "ffprobe"].iter().all(|b| {
        Command::new(b)
            .arg("-version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

fn make_video(path: &Path) {
    let status = Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi"])
        .args(["-i", "testsrc=duration=2:size=320x240:rate=10"])
        .args(["-pix_fmt", "yuv420p"])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

fn height(image: &Path) -> u32 {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=height",
            "-of",
            "csv=p=0",
        ])
        .arg(image)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

/// The binary with no FUNSYNC_* variables inherited from the test's
/// environment.
fn command() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_age-screen"));
    c.env_remove("FUNSYNC_USER_DATA")
        .env_remove("FUNSYNC_BACKEND_URL");
    c
}

fn run(user_data: &Path, judge_out: &Path, args: &[&str]) -> Output {
    command()
        .arg("--user-data")
        .arg(user_data)
        .args(args)
        .env("JUDGE_OUT", judge_out)
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

const JUDGE: &str = r#"#!/bin/sh
# age-screen's test judge: runs in the work folder, prompt in $1.
set -e
echo run >> "$JUDGE_OUT/runs"
printf '%s' "$1" > "$JUDGE_OUT/prompt.txt"
cp batches/01.json "$JUDGE_OUT/batch.json"
cp ./*.jpg "$JUDGE_OUT/"
cat > results/01.json <<'EOF'
[
  {"id": "v-purge", "p_minor": 0.9, "reason": "childlike proportions"},
  {"id": "v-fav", "p_minor": 0.95, "reason": "canonically 16"},
  {"id": "v-review", "p_minor": 0.5, "reason": "could look underage"},
  {"id": "v-clear", "p_minor": 0.4999, "reason": "adult"},
  {"id": "v-vr", "p_minor": 0.1, "reason": "clearly adult"}
]
EOF
"#;

#[test]
fn screens_applies_records_and_skips_what_was_screened() {
    if !has_ffmpeg() {
        eprintln!("ffmpeg/ffprobe not found; skipping the end-to-end screening test");
        return;
    }
    let tmp = TempDir::new();
    let user_data = tmp.0.join("userData");
    let source = tmp.0.join("GVP").join("Other");
    let scene = source.join("Artist").join("Scene");
    let judge_out = tmp.0.join("judge-out");
    for d in [&user_data, &scene, &judge_out] {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(
        user_data.join("config.json"),
        json!({ "settings": { "library": { "sources": [{ "name": "Other", "path": source }] } } })
            .to_string(),
    )
    .unwrap();
    let base = tmp.0.join("base.mp4");
    make_video(&base);
    let path = |name: &str| scene.join(name).to_str().unwrap().to_string();
    for name in [
        "purge.mp4",
        "fav.mp4",
        "review.mp4",
        "clear.mp4",
        "vr.mp4",
        "._purge.mp4",
        "rated.mp4",
    ] {
        fs::copy(&base, scene.join(name)).unwrap();
    }
    let judge = tmp.0.join("judge.sh");
    fs::write(&judge, JUDGE).unwrap();
    Command::new("chmod")
        .arg("+x")
        .arg(&judge)
        .status()
        .unwrap();
    let judge_cmd = judge.to_str().unwrap();

    let entry = |id: &str, name: &str, favorite: bool, rating: f64, duration: f64| json!({ "id": id, "path": path(name), "favorite": favorite, "rating": rating, "duration": duration });
    let videos = json!([
        entry("v-purge", "purge.mp4", false, 0.0, 2.0),
        entry("v-fav", "fav.mp4", true, 4.0, 2.0),
        entry("v-review", "review.mp4", false, 0.0, 0.0),
        entry("v-clear", "clear.mp4", false, 3.0, 2.0),
        entry("v-vr", "vr.mp4", false, 0.0, 2.0),
        entry("v-apple", "._purge.mp4", false, 0.0, 2.0),
        entry("v-gone", "gone.mp4", false, 0.0, 2.0),
        entry("v-rated", "rated.mp4", false, 1.0, 2.0),
    ]);
    let (url, posts) = fake_backend(
        videos,
        json!({ path("vr.mp4"): true, path("purge.mp4"): false }),
    );
    let base_args = ["--backend", url.as_str(), "--judge-cmd", judge_cmd];

    // First run: five screened, three applied.
    let o = run(
        &user_data,
        &judge_out,
        &[&["--run"][..], &base_args].concat(),
    );
    let all = text(&o);
    assert_eq!(o.status.code(), Some(0), "{all}");
    assert!(all.contains("5 to screen"), "{all}");
    assert!(
        all.contains("1 rated 2 or lower, 1 AppleDouble, 1 not on disk"),
        "{all}"
    );
    assert!(all.contains("1 purged (1★), 1 to \"Underage review\", 1 favourite(s) to \"Underage review\", 2 clear; 0 unscreened"), "{all}");
    assert_eq!(
        *posts.lock().unwrap(),
        vec![
            (
                "/api/remote/rating".to_string(),
                json!({ "videoId": "v-purge", "rating": 1.0 })
            ),
            (
                "/api/remote/playlists/add-video".to_string(),
                json!({ "videoId": "v-fav", "name": "Underage review" })
            ),
            (
                "/api/remote/playlists/add-video".to_string(),
                json!({ "videoId": "v-review", "name": "Underage review" })
            ),
        ]
    );

    // What the judge got: the prompt (rubric, batch → results) and one
    // batch of titles relative to the source, with real sheets.
    let prompt = fs::read_to_string(judge_out.join("prompt.txt")).unwrap();
    assert!(prompt.contains("a character who is canonically a minor (e.g. Sarada Uchiha)"));
    assert!(prompt.contains("batches/01.json → "));
    let batch: Value =
        serde_json::from_str(&fs::read_to_string(judge_out.join("batch.json")).unwrap()).unwrap();
    let titles: Vec<&str> = batch
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["title"].as_str().unwrap())
        .collect();
    assert_eq!(
        titles,
        [
            "Artist/Scene/purge.mp4",
            "Artist/Scene/fav.mp4",
            "Artist/Scene/review.mp4",
            "Artist/Scene/clear.mp4",
            "Artist/Scene/vr.mp4"
        ]
    );
    // 4×3 tiles of 400 px wide frames: flat 320×240 → 300 high; VR keeps
    // the left half (160×240) → 600 high; plus 2 × 4 px padding.
    assert_eq!(height(&judge_out.join("v-purge.jpg")), 3 * 300 + 8);
    assert_eq!(height(&judge_out.join("v-review.jpg")), 3 * 300 + 8);
    assert_eq!(height(&judge_out.join("v-vr.jpg")), 3 * 600 + 8);

    // The state: every judged video, its size and action; work dir gone.
    let state: Value =
        serde_json::from_str(&fs::read_to_string(user_data.join("age-screen.json")).unwrap())
            .unwrap();
    assert_eq!(state["version"], 1);
    assert!(state["lastRun"].as_str().unwrap().ends_with('Z'));
    let size = fs::metadata(&base).unwrap().len();
    let actions: Vec<(String, String, f64)> = state["videos"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| {
            assert_eq!(v["size"], size);
            (
                k.rsplit('/').next().unwrap().to_string(),
                v["action"].as_str().unwrap().to_string(),
                v["pMinor"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        actions,
        vec![
            ("clear.mp4".into(), "clear".into(), 0.4999),
            ("fav.mp4".into(), "review-favourite".into(), 0.95),
            ("purge.mp4".into(), "purged".into(), 0.9),
            ("review.mp4".into(), "review".into(), 0.5),
            ("vr.mp4".into(), "clear".into(), 0.1),
        ]
    );
    assert!(!user_data.join("age-screen-work").exists());
    assert!(!user_data.join("age-screen.lock").exists());

    // --list shows the three flagged ones.
    let o = run(&user_data, &judge_out, &["--list"]);
    let listed = text(&o);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(listed.matches("p=").count(), 3, "{listed}");
    assert!(listed.contains("purged") && listed.contains("review-favourite"));

    // --weekly right after: not due; --run: nothing new. No judge, no POST.
    let o = run(
        &user_data,
        &judge_out,
        &[&["--weekly"][..], &base_args].concat(),
    );
    assert_eq!(o.status.code(), Some(0));
    assert!(text(&o).contains("Not due"), "{}", text(&o));
    let o = run(
        &user_data,
        &judge_out,
        &[&["--run"][..], &base_args].concat(),
    );
    assert_eq!(o.status.code(), Some(0));
    assert!(text(&o).contains("Nothing to screen."), "{}", text(&o));
    assert_eq!(fs::read_to_string(judge_out.join("runs")).unwrap(), "run\n");
    assert_eq!(posts.lock().unwrap().len(), 3);

    // A changed size is screened again; this judge's file then lists ids
    // not in the batch, so it's invalid: left unscreened, exit 2; a dry run
    // writes nothing either way.
    let mut f = fs::OpenOptions::new()
        .append(true)
        .open(scene.join("clear.mp4"))
        .unwrap();
    f.write_all(b"more").unwrap();
    drop(f);
    let before = fs::read_to_string(user_data.join("age-screen.json")).unwrap();
    let o = run(
        &user_data,
        &judge_out,
        &[&["--dry-run"][..], &base_args].concat(),
    );
    let all = text(&o);
    assert_eq!(o.status.code(), Some(2), "{all}");
    assert!(all.contains("1 to screen"), "{all}");
    assert!(all.contains("is not in the batch"), "{all}");
    assert!(all.contains("1 unscreened"), "{all}");
    assert_eq!(
        fs::read_to_string(user_data.join("age-screen.json")).unwrap(),
        before
    );
    assert_eq!(posts.lock().unwrap().len(), 3);
}

#[test]
fn backend_down_exits_2_and_help_works() {
    let tmp = TempDir::new();
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let backend = format!("http://127.0.0.1:{port}");
    let o = run(&tmp.0, &tmp.0, &["--run", "--backend", &backend]);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains("is FunSync running?"));
    assert!(!tmp.0.join("age-screen.json").exists());
    assert!(!tmp.0.join("age-screen.lock").exists());

    let o = run(&tmp.0, &tmp.0, &["--help"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(text(&o).contains("Exit codes"));
    let o = run(&tmp.0, &tmp.0, &["--bogus"]);
    assert_eq!(o.status.code(), Some(1));
    let o = run(&tmp.0, &tmp.0, &["--list"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(text(&o).contains("No videos screened yet"));
}

#[test]
fn env_vars_set_the_defaults_and_flags_win() {
    let tmp = TempDir::new();
    let (url, posts) = fake_backend(json!([]), json!({}));
    let dead = format!(
        "http://127.0.0.1:{}",
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    );

    // FUNSYNC_BACKEND_URL is the backend when --backend isn't given.
    let o = command()
        .args(["--run", "--user-data"])
        .arg(&tmp.0)
        .env("FUNSYNC_BACKEND_URL", format!("{url}/"))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("0 video(s) in the library"));
    assert!(posts.lock().unwrap().is_empty());

    // --backend wins over it.
    let o = command()
        .args(["--run", "--backend", &dead, "--user-data"])
        .arg(&tmp.0)
        .env("FUNSYNC_BACKEND_URL", &url)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains("is FunSync running?"));

    // FUNSYNC_USER_DATA is the userData when --user-data isn't given, and
    // --user-data wins over it.
    let missing = tmp.0.join("no-such-userData");
    let o = command()
        .arg("--list")
        .env("FUNSYNC_USER_DATA", &missing)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(text(&o).contains("no-such-userData is not a folder"));
    let o = command()
        .args(["--list", "--user-data"])
        .arg(&tmp.0)
        .env("FUNSYNC_USER_DATA", &missing)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("No flagged videos."), "{}", text(&o));
}
