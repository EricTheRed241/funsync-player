//! End to end through the binary, against a temp userData and library, with
//! the network faked: `PMVHAVEN_SCOUT_FAKE_HTTP` points the CLI at canned
//! responses.

use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

const SITE: &str = "https://pmvhaven.com";
const S3: &str = "https://pmvhavencloud.s3.example";

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "pmvhaven-scout-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("userData")).unwrap();
        fs::create_dir_all(dir.join("GVP/Other")).unwrap();
        fs::create_dir_all(dir.join("GVP/EroScripts")).unwrap();
        let config = json!({"settings": {"backendPort": 5999, "library": {
            "sources": [
                {"name": "EroScripts", "path": dir.join("GVP/EroScripts"), "enabled": true},
                {"name": "Other", "path": dir.join("GVP/Other"), "enabled": true}
            ],
            "favouriteArtists": {"Vassago": true}
        }}});
        fs::write(dir.join("userData/config.json"), config.to_string()).unwrap();
        TempDir(dir)
    }

    fn user_data(&self) -> PathBuf {
        self.0.join("userData")
    }

    fn other(&self) -> PathBuf {
        self.0.join("GVP/Other")
    }

    fn fixtures(&self, responses: &Value) -> PathBuf {
        let path = self.0.join("fixtures.json");
        fs::write(&path, responses.to_string()).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&str], fixtures: Option<&Path>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pmvhaven-scout"));
    cmd.args(args).env_remove("FUNSYNC_USER_DATA");
    match fixtures {
        Some(f) => cmd.env("PMVHAVEN_SCOUT_FAKE_HTTP", f),
        // An unreadable fixture file fails the run before any request.
        None => cmd.env("PMVHAVEN_SCOUT_FAKE_HTTP", "/nonexistent/fixtures.json"),
    };
    cmd.output().unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn ok(body: Value) -> Value {
    json!({ "status": 200, "body": body })
}

fn list_url(page: u32) -> String {
    format!("{SITE}/api/videos?funscriptOnly=true&limit=100&page={page}")
}

fn item(id: &str, title: &str, extra: Value) -> Value {
    let mut v = json!({
        "_id": id, "title": title, "uploader": "Scruffy6", "uploaderUsername": "Scruffy6",
        "originalCreator": "", "featuredStar": "", "starsTags": ["Riley reid"],
        "videoUrl": format!("{S3}/videos/{id}.mp4"),
        "thumbnailSizes": {"lg": {"url": format!("{S3}/thumbnails/{id}/thumb_lg.webp")}},
        "durationSeconds": 293, "aspectRatio": 1.7778, "width": 1920, "height": 1080,
        "views": 9000, "likes": 375, "hasGayContent": false, "hasTransContent": false,
        "hasExtremeContent": false, "hasVoiceOver": false,
        "releaseDate": "2026-10-14T10:00:00.000Z", "tags": ["Blowjob", "Pmv，music"], "top5Tags": []
    });
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    v
}

fn detail(extra: Value) -> Value {
    let mut d = json!({
        "description": "", "funScriptUrl": null, "userFunScripts": [], "fileSize": 7,
        "timelineThumbnails": [
            {"url": format!("{S3}/timeline/1.webp"), "captureTime": 10},
            {"url": format!("{S3}/timeline/2.webp"), "captureTime": 20}
        ]
    });
    for (k, x) in extra.as_object().unwrap() {
        d[k] = x.clone();
    }
    ok(json!({"success": true, "data": d}))
}

fn funscript() -> Value {
    json!({"version": "1.0", "actions": [{"at": 0, "pos": 0}, {"at": 500, "pos": 100}]})
}

/// Two list pages: a VR video with the uploader's script, a flat one with
/// community scripts (and a twist companion), and one for each way out.
fn responses() -> Value {
    let page1 = json!({"success": true, "videos": [
        item("aaa", "Shrooms", json!({"originalCreator": "Vassago", "aspectRatio": 2.0,
            "width": 3840, "height": 1920, "tags": ["Vr", "Riding"], "likes": 750})),
        item("bbb", "Neurotica", json!({})),
        item("gay", "G", json!({"hasGayContent": true})),
        item("aud", "Corrupted - Binaural Story Audio", json!({"tags": ["Binaural"]})),
        item("fut", "Futa night", json!({})),
    ], "pagination": {"page": 1, "total": 9, "totalPages": 2}});
    let page2 = json!({"success": true, "videos": [
        item("bbb", "Neurotica", json!({})),
        item("sho", "Short", json!({"durationSeconds": 60})),
        item("non", "No script here", json!({})),
        item("err", "Detail fails", json!({})),
        item("lib", "In the library", json!({})),
    ], "pagination": {"page": 2, "total": 9, "totalPages": 2}});
    let community = |user: &str, name: &str, description: &str, likes: u64| {
        json!({"username": user, "description": description, "likes": likes,
               "uploadDate": "2026-08-05T18:35:07.422Z",
               "scriptUrl": format!("{S3}/funscripts/bbb/68fdc784b99aaf24a4c0d42a_1785338305589_{name}")})
    };
    let mut r = json!({});
    r[format!("GET {}", list_url(1))] = ok(page1);
    r[format!("GET {}", list_url(2))] = ok(page2);
    r[format!("GET {SITE}/api/videos/aaa")] =
        detail(json!({"funScriptUrl": format!("{S3}/funscripts/aaa.funscript")}));
    r[format!("GET {SITE}/api/videos/bbb")] = detail(json!({"userFunScripts": [
        community("auto", "A.funscript", "Quick Audio Based FunScript", 9),
        community("hand", "N.funscript", "Made for Handy", 1),
        community("hand", "N.twist.funscript", "", 0),
    ]}));
    r[format!("GET {SITE}/api/videos/non")] = detail(json!({}));
    r[format!("GET {SITE}/api/videos/err")] = json!({"status": 500, "body": "oops"});
    r[format!("GET {SITE}/api/videos/lib")] =
        detail(json!({"funScriptUrl": format!("{S3}/funscripts/lib.funscript")}));
    r[format!("GET {S3}/funscripts/aaa.funscript")] = ok(funscript());
    r[format!("GET {S3}/videos/aaa.mp4")] = ok(json!("VIDEO"));
    r[format!("GET {S3}/funscripts/bbb/68fdc784b99aaf24a4c0d42a_1785338305589_N.funscript")] =
        ok(funscript());
    r[format!(
        "GET {S3}/funscripts/bbb/68fdc784b99aaf24a4c0d42a_1785338305589_N.twist.funscript"
    )] = ok(funscript());
    r[format!("GET {S3}/videos/bbb.mp4")] = ok(json!("VIDEO2"));
    r
}

#[test]
fn help_and_bad_flags() {
    let o = run(&["--help"], None);
    assert_eq!(o.status.code(), Some(0));
    assert!(stdout(&o).contains("--fetch"));
    let o = run(&["--bogus"], None);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("unknown argument"));
    let o = run(&["--user-data", "/nonexistent/x"], None);
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn scans_screens_scores_and_fetches() {
    let t = TempDir::new();
    let ud = t.user_data();
    let ud_s = ud.to_str().unwrap();
    // Already in the library: a scene folder with this scout's sidecar.
    let held = t
        .other()
        .join("Someone/Someone - In the library (PMVHaven)");
    fs::create_dir_all(&held).unwrap();
    fs::write(
        held.join("pmvhaven.json"),
        r#"{"site": "pmvhaven", "id": "lib"}"#,
    )
    .unwrap();
    fs::write(held.join("Someone - In the library (PMVHaven).mp4"), "x").unwrap();
    let fixtures = t.fixtures(&responses());

    let o = run(&["--user-data", ud_s], Some(&fixtures));
    assert_eq!(o.status.code(), Some(0), "{}{}", stdout(&o), stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("9 scripted video(s) listed of 9"), "{out}");
    assert!(
        out.contains(
            "Dropped 6: audio 1, detail-failed 1, excluded 1, gay 1, no-script 1, too-short 1."
        ),
        "{out}"
    );
    let file = read_json(&ud.join("pmvhaven-scout.json"));
    assert_eq!(file["version"], 1);
    let scenes = file["scenes"].as_array().unwrap();
    let ids: Vec<&str> = scenes.iter().map(|s| s["id"].as_str().unwrap()).collect();
    // Best first: VR + favourite + likes.
    assert_eq!(ids, ["aaa", "bbb", "lib"]);
    let a = &scenes[0];
    assert_eq!(a["site"], "pmvhaven");
    assert_eq!(a["url"], "https://pmvhaven.com/video/shrooms_aaa");
    assert_eq!(a["studio"], "Vassago");
    assert_eq!(a["favourite"], "Vassago");
    assert_eq!(a["actors"], json!(["Riley reid"]));
    assert_eq!(
        (a["vr"].as_bool(), a["vrSuffix"].as_str()),
        (Some(true), Some("_180_LR"))
    );
    assert_eq!(a["date"], "2026-10-14");
    assert_eq!(a["durationS"], 293);
    assert_eq!(a["images"].as_array().unwrap().len(), 2);
    assert_eq!(a["script"]["source"], "uploader");
    assert_eq!(a["script"]["axes"], json!([]));
    assert_eq!(a["scoreParts"]["likes"], 2);
    assert_eq!(a["scoreParts"]["vr"], 3);
    assert_eq!(a["inLibrary"], false);
    assert_eq!(a["fetchedAt"], Value::Null);
    let b = &scenes[1];
    assert_eq!(b["vr"], false);
    assert_eq!(b["studio"], "Scruffy6");
    assert_eq!(b["script"]["by"], "hand");
    assert_eq!(b["script"]["axes"], json!(["twist"]));
    assert_eq!(b["scoreParts"]["multiAxis"], 4);
    assert_eq!(scenes[2]["inLibrary"], true);
    let cache = read_json(&ud.join("pmvhaven-scout-cache.json"));
    assert_eq!(cache["videos"].as_object().unwrap().len(), 4);

    // A second scan reads details from the cache.
    let mut lists = json!({});
    for p in [1, 2] {
        let key = format!("GET {}", list_url(p));
        lists[&key] = responses()[&key].clone();
    }
    let lists_only = t.0.join("lists.json");
    fs::write(&lists_only, lists.to_string()).unwrap();
    let o = run(&["--user-data", ud_s], Some(&lists_only));
    assert_eq!(o.status.code(), Some(0), "{}{}", stdout(&o), stderr(&o));
    assert!(stdout(&o).contains("0 detail(s) read"), "{}", stdout(&o));
    assert_eq!(
        read_json(&ud.join("pmvhaven-scout.json"))["scenes"]
            .as_array()
            .unwrap()
            .len(),
        3
    );

    // Right swipes: aaa and bbb (and lib, which the library has, and a gone id).
    fs::write(
        ud.join("pmvhaven-scout-swipes.json"),
        json!({"version": 1, "swipes": {
            "aaa": {"dir": "right", "at": "2026-11-17T00:00:00Z"},
            "bbb": {"dir": "right", "at": "2026-11-17T00:00:00Z"},
            "lib": {"dir": "right", "at": "2026-11-17T00:00:00Z"},
            "gone": {"dir": "right", "at": "2026-11-17T00:00:00Z"},
            "gay": {"dir": "left", "at": "2026-11-17T00:00:00Z"}}})
        .to_string(),
    )
    .unwrap();
    let o = run(
        &["--user-data", ud_s, "--fetch", "--dry-run"],
        Some(&fixtures),
    );
    assert_eq!(o.status.code(), Some(0), "{}{}", stdout(&o), stderr(&o));
    let out = stdout(&o);
    assert!(
        out.contains(
            "4 video(s) swiped right: 2 to fetch, 1 fetched or in the library, 1 no longer"
        ),
        "{out}"
    );
    assert!(
        out.contains("Vassago - Shrooms (PMVHaven)_180_LR.mp4"),
        "{out}"
    );
    assert!(out.contains("Dry run"), "{out}");
    assert!(!t.other().join("Vassago").exists());

    let o = run(
        &["--user-data", ud_s, "--fetch", "--limit", "1"],
        Some(&fixtures),
    );
    assert_eq!(o.status.code(), Some(2), "{}{}", stdout(&o), stderr(&o));
    let dir = t
        .other()
        .join("Vassago/Vassago - Shrooms (PMVHaven)_180_LR");
    let stem = "Vassago - Shrooms (PMVHaven)_180_LR";
    assert_eq!(
        fs::read_to_string(dir.join(format!("{stem}.mp4"))).unwrap(),
        "VIDEO"
    );
    assert!(read_json(&dir.join(format!("{stem}.funscript")))["actions"].is_array());
    let sidecar = read_json(&dir.join("pmvhaven.json"));
    assert_eq!(
        (sidecar["site"].as_str(), sidecar["id"].as_str()),
        (Some("pmvhaven"), Some("aaa"))
    );
    assert_eq!(sidecar["script"]["source"], "uploader");
    let staging = t.0.join("GVP/_staging");
    assert_eq!(fs::read_dir(&staging).unwrap().count(), 0);
    let file = read_json(&ud.join("pmvhaven-scout.json"));
    assert!(file["scenes"][0]["fetchedAt"].is_string());
    assert!(stdout(&o).contains("--limit 1 reached"), "{}", stdout(&o));

    let o = run(&["--user-data", ud_s, "--fetch"], Some(&fixtures));
    assert_eq!(o.status.code(), Some(0), "{}{}", stdout(&o), stderr(&o));
    let dir = t.other().join("Scruffy6/Scruffy6 - Neurotica (PMVHaven)");
    let stem = "Scruffy6 - Neurotica (PMVHaven)";
    assert!(dir.join(format!("{stem}.mp4")).is_file());
    assert!(dir.join(format!("{stem}.funscript")).is_file());
    assert!(dir.join(format!("{stem}.twist.funscript")).is_file());

    // fetchedAt survives a scan; nothing left to fetch.
    let o = run(&["--user-data", ud_s], Some(&lists_only));
    assert_eq!(o.status.code(), Some(0));
    let file = read_json(&ud.join("pmvhaven-scout.json"));
    assert!(
        file["scenes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["fetchedAt"].is_string())
            .count()
            == 2
    );
    let o = run(&["--user-data", ud_s, "--fetch"], Some(&fixtures));
    assert_eq!(o.status.code(), Some(0));
    assert!(stdout(&o).contains("0 to fetch"), "{}", stdout(&o));
}

#[test]
fn a_failed_download_leaves_nothing_behind() {
    let t = TempDir::new();
    let ud = t.user_data();
    let ud_s = ud.to_str().unwrap();
    let mut r = responses();
    r[format!("GET {S3}/videos/aaa.mp4")] = json!({"status": 403, "body": "denied"});
    let fixtures = t.fixtures(&r);
    let o = run(&["--user-data", ud_s], Some(&fixtures));
    assert_eq!(o.status.code(), Some(0));
    fs::write(
        ud.join("pmvhaven-scout-swipes.json"),
        r#"{"version": 1, "swipes": {"aaa": {"dir": "right", "at": "x"}}}"#,
    )
    .unwrap();
    let o = run(&["--user-data", ud_s, "--fetch"], Some(&fixtures));
    assert_eq!(o.status.code(), Some(1), "{}{}", stdout(&o), stderr(&o));
    assert!(stdout(&o).contains("video download"), "{}", stdout(&o));
    assert!(!t.other().join("Vassago").exists());
    assert_eq!(fs::read_dir(t.0.join("GVP/_staging")).unwrap().count(), 0);
    let file = read_json(&ud.join("pmvhaven-scout.json"));
    assert_eq!(file["scenes"][0]["fetchedAt"], Value::Null);
}

#[test]
fn a_rate_limited_list_writes_nothing() {
    let t = TempDir::new();
    let ud = t.user_data();
    let mut r = json!({});
    r[format!("GET {}", list_url(1))] = json!({"status": 429, "body": ""});
    let fixtures = t.fixtures(&r);
    let o = run(&["--user-data", ud.to_str().unwrap()], Some(&fixtures));
    assert_eq!(o.status.code(), Some(2), "{}{}", stdout(&o), stderr(&o));
    assert!(!ud.join("pmvhaven-scout.json").exists());
}
