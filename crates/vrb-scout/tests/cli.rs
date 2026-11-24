//! End to end through the binary, against a temp userData and library, with
//! the network faked: `VRB_SCOUT_FAKE_HTTP` points the CLI at canned
//! responses (the content API, the toy scripts, the member scene pages and
//! the downloads).

use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

const VRB: &str = "https://content.vrbangers.com/api/content/v1";
const CONK: &str = "https://content.vrconk.com/api/content/v1";
const TOKEN: &str = "SECRETTOKEN";

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "vrb-scout-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("userData")).unwrap();
        let other = dir.join("lib/Other");
        // Already in the library through the EroScripts scout.
        fs::create_dir_all(
            other.join("Adriana Chechik/Adriana Chechik - Wet Adventures (VRBangers)_180x180_3dh"),
        )
        .unwrap();
        fs::write(
            dir.join("userData/config.json"),
            json!({"settings": {"library": {
                "sources": [{"name": "Other", "path": other, "enabled": true}],
                "tagScores": {"pov": 1.5}
            }}})
            .to_string(),
        )
        .unwrap();
        TempDir(dir)
    }

    fn user_data(&self) -> PathBuf {
        self.0.join("userData")
    }

    fn other(&self) -> PathBuf {
        self.0.join("lib/Other")
    }

    fn fixtures(&self, responses: &Value) -> PathBuf {
        let path = self.0.join("fixtures.json");
        fs::write(&path, responses.to_string()).unwrap();
        path
    }

    fn swipe(&self, ids: &[&str]) {
        let swipes: serde_json::Map<String, Value> = ids
            .iter()
            .map(|id| {
                (
                    id.to_string(),
                    json!({"dir": "right", "at": "2026-11-17T00:00:00.000Z"}),
                )
            })
            .collect();
        fs::write(
            self.user_data().join("vrb-scout-swipes.json"),
            json!({"version": 1, "swipes": swipes}).to_string(),
        )
        .unwrap();
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&str], fixtures: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vrb-scout"))
        .args(args)
        .env_remove("FUNSYNC_USER_DATA")
        .env("VRB_SCOUT_FAKE_HTTP", fixtures)
        .output()
        .unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn read_file(user_data: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(user_data.join("vrb-scout.json")).unwrap()).unwrap()
}

fn ok(body: Value) -> Value {
    json!({ "status": 200, "body": body })
}

fn page(items: Vec<Value>, pages: u32) -> Value {
    ok(json!({"status": {"code": 1}, "data": {"items": items, "pages": pages}}))
}

fn item(id: &str, slug: &str, title: &str, models: &[&str], script: bool, views: u64) -> Value {
    let short = slug.replace('-', "_");
    json!({
        "id": id, "slug": slug, "title": title, "hasToyScript": script,
        "models": models.iter().map(|m| json!({"title": m})).collect::<Vec<_>>(),
        "poster": {"permalink": format!("/uploads/{id}.jpg")},
        "publishedAt": 1792738860, "viewCount": views,
        "videoSettings": {"duration": 2400, "videoShortName": short,
            "downloadsQuality": ["4K_180x180_3dh", "8K_180x180_3dh"]}
    })
}

fn detail(categories: &[&str], updated: i64) -> Value {
    ok(json!({"status": {"code": 1}, "data": {"item": {
        "updatedAt": updated,
        "categories": categories.iter().map(|c| json!({"name": c})).collect::<Vec<_>>(),
        "galleryImages": [{"permalink": "/u/1.jpg", "previews": [{"sizeAlias": "XL", "permalink": "/u/1_XL.jpg"}]}],
        "videoTechBar": {"fps": [1]},
        "videoSettings": {"transparency": {"mode": 0}}
    }}}))
}

fn digest() -> Value {
    ok(json!({"status": {"code": 1}, "data": {"digest": {"videoFps": [{"id": 1, "name": 60}]}}}))
}

/// VRBangers: a1 (kept), b2 (no toy script), c3 (screened out), d4 (in the
/// library), f6 (its detail fails); VRConk: e5.
fn catalogue(a1_title: &str) -> Value {
    json!({
        format!("GET {VRB}/digest"): digest(),
        format!("GET {CONK}/digest"): digest(),
        format!("GET {VRB}/videos?page=1&limit=100&sort=latest"): page(vec![
            item("a1", "wet-and-wild", a1_title, &["Angel Youngs", "Scarlette Moon"], true, 45000),
            item("b2", "no-script", "No Script", &["X"], false, 10),
            item("c3", "bad-one", "Futa Night", &["Y"], true, 10),
        ], 2),
        format!("GET {VRB}/videos?page=2&limit=100&sort=latest"): page(vec![
            item("d4", "wet-adventures", "Wet Adventures (Remastered)", &["Adriana Chechik"], true, 100),
            item("f6", "broken", "Broken Detail", &["Z"], true, 100),
        ], 2),
        format!("GET {CONK}/videos?page=1&limit=100&sort=latest"): page(vec![
            item("e5", "conk-scene", "Conk Scene", &["Vanna Bardot"], true, 2500),
        ], 1),
        format!("GET {VRB}/videos/wet-and-wild"): detail(&["Creampie", "POV", "3D"], 100),
        format!("GET {VRB}/videos/bad-one"): detail(&[], 100),
        format!("GET {VRB}/videos/wet-adventures"): detail(&["Babe"], 100),
        format!("GET {VRB}/videos/broken"): {"status": 500, "body": "oops"},
        format!("GET {CONK}/videos/conk-scene"): detail(&["Babe"], 100),
    })
}

fn signed(q: &str) -> String {
    format!(
        "https://mov.vrbangers.com/vrb_wet_and_wild/VRBANGERS_wet_and_wild_{q}_180x180_3dh.mp4?secure={TOKEN},1800000000"
    )
}

/// The member's scene page for a1: the scene with 10K / 8K / 6K downloads.
fn scene_page() -> Value {
    let flat = json!([
        {"video": 1},
        {"slug": 2, "videoSettings": 3, "downLoadVideoListForFront": 5, "videoFiles": 12},
        "wet-and-wild",
        {"videoShortName": 4},
        "wet_and_wild",
        [6, 8, 10],
        {"url": 7}, signed("10K"),
        {"url": 9}, signed("8K"),
        {"url": 11}, signed("6K"),
        [13],
        {"qualityKey": 14, "fileSize": 15}, "8K_180x180_3dh", 5
    ]);
    ok(Value::String(format!(
        "<html><script type=\"application/json\" id=\"__NUXT_DATA__\">{flat}</script></html>"
    )))
}

fn with(mut base: Value, more: Value) -> Value {
    for (k, v) in more.as_object().unwrap() {
        base[k] = v.clone();
    }
    base
}

fn scene<'a>(file: &'a Value, id: &str) -> Option<&'a Value> {
    file["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
}

#[test]
fn help_and_bad_flags() {
    let t = TempDir::new();
    let f = t.fixtures(&json!({}));
    let o = run(&["--help"], &f);
    assert_eq!(o.status.code(), Some(0));
    assert!(stdout(&o).contains("--fetch"));
    let o = run(&["--bogus"], &f);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("unknown argument"));
}

#[test]
fn scans_scores_and_marks_the_library() {
    let t = TempDir::new();
    let ud = t.user_data();
    let ud_s = ud.to_str().unwrap();
    let f = t.fixtures(&catalogue("Wet & Wild In Malibu"));
    let o = run(&["--user-data", ud_s], &f);
    assert_eq!(o.status.code(), Some(0), "{}{}", stdout(&o), stderr(&o));
    let file = read_file(&ud);
    assert_eq!(file["version"], 1);
    let ids: Vec<&str> = file["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    // Best first: a1 has VR, creampie, pov and many views.
    assert_eq!(ids[0], "a1");
    assert!(ids.contains(&"d4") && ids.contains(&"e5") && ids.contains(&"f6"));
    assert!(!ids.contains(&"b2") && !ids.contains(&"c3"));
    let a = scene(&file, "a1").unwrap();
    assert_eq!(a["site"], "vrbangers");
    assert_eq!(a["url"], "https://vrbangers.com/video/wet-and-wild/");
    assert_eq!(a["studio"], "VRBangers");
    assert_eq!(a["actors"], json!(["Angel Youngs", "Scarlette Moon"]));
    assert_eq!(a["date"], "2026-10-23");
    assert_eq!(a["durationS"], 2400);
    assert_eq!(
        a["thumbnail"],
        "https://content.vrbangers.com/uploads/a1.jpg"
    );
    assert_eq!(
        a["images"],
        json!(["https://content.vrbangers.com/u/1_XL.jpg"])
    );
    assert_eq!(a["vr"], true);
    assert_eq!(a["fps"], 60);
    assert_eq!(a["creampie"], true);
    assert_eq!(a["likes"], 45000);
    assert_eq!(
        a["script"],
        json!({"url": "https://mov.vrbangers.com/vrb_wet_and_wild/MISC/VRBANGERS_wet_and_wild_toy.funscript", "axes": []})
    );
    assert!(a["tags"].as_array().unwrap().contains(&json!("creampie")));
    assert!(!a["tags"].as_array().unwrap().contains(&json!("3d")));
    assert_eq!(a["tagScores"]["pov"], 1.5);
    assert_eq!(a["scoreParts"]["vr"], 3);
    assert_eq!(a["scoreParts"]["likes"], 1.5);
    assert_eq!(a["inLibrary"], false);
    assert_eq!(a["fetchedAt"], Value::Null);
    assert_eq!(scene(&file, "d4").unwrap()["inLibrary"], true);
    let e = scene(&file, "e5").unwrap();
    assert_eq!(
        (e["site"].as_str(), e["studio"].as_str()),
        (Some("vrconk"), Some("VRConk"))
    );
    assert_eq!(e["scoreParts"]["likes"], 1);
    // The failed detail is asked again next time.
    assert_eq!(scene(&file, "f6").unwrap()["updatedAt"], Value::Null);
    let excluded = file["excluded"].as_array().unwrap();
    assert_eq!(excluded.len(), 1);
    assert_eq!(excluded[0]["id"], "c3");
    assert_eq!(excluded[0]["excluded"], "excluded (futa)");
    let out = stdout(&o);
    assert!(
        out.contains("VRBangers: 5 listed, 4 with a toy script; 3 kept (1 in the library), 1 excluded; 3 detail(s) read, 1 failed."),
        "{out}"
    );
    assert!(
        out.contains("VRConk: 1 listed, 1 with a toy script; 1 kept"),
        "{out}"
    );
    assert!(out.contains("Excluded: excluded 1."), "{out}");

    // Rescan: unchanged scenes come from the file (their details now 404),
    // a1's listing changed, so its detail is read again; f6 is retried.
    let mut next = catalogue("Wet & Wild In Malibu (Remastered)");
    next[format!("GET {VRB}/videos/wet-and-wild")] = detail(&["Babe"], 200);
    next[format!("GET {VRB}/videos/broken")] = detail(&["Babe"], 300);
    for slug in ["bad-one", "wet-adventures"] {
        next.as_object_mut()
            .unwrap()
            .remove(&format!("GET {VRB}/videos/{slug}"));
    }
    next.as_object_mut()
        .unwrap()
        .remove(&format!("GET {CONK}/videos/conk-scene"));
    let o = run(&["--user-data", ud_s], &t.fixtures(&next));
    assert_eq!(o.status.code(), Some(0), "{}{}", stdout(&o), stderr(&o));
    let file = read_file(&ud);
    let a = scene(&file, "a1").unwrap();
    assert_eq!(a["updatedAt"], 200);
    assert_eq!(a["creampie"], false);
    assert_eq!(scene(&file, "f6").unwrap()["updatedAt"], 300);
    assert_eq!(scene(&file, "e5").unwrap()["categories"], json!(["Babe"]));
    assert_eq!(file["excluded"][0]["id"], "c3");
    assert!(stdout(&o).contains("2 detail(s) read."), "{}", stdout(&o));
    assert!(ud.join("vrb-scout.json.bak").exists());

    // --sites keeps the other site's scenes as they were.
    let o = run(
        &["--user-data", ud_s, "--sites", "vrconk"],
        &t.fixtures(&next),
    );
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let file = read_file(&ud);
    assert!(scene(&file, "a1").is_some() && scene(&file, "e5").is_some());
    assert!(stdout(&o).contains("VRBangers: not scanned; kept the previous 3 scene(s)."));
}

#[test]
fn fetches_right_swipes() {
    let t = TempDir::new();
    let ud = t.user_data();
    let ud_s = ud.to_str().unwrap();
    let base = catalogue("Wet & Wild In Malibu");
    let o = run(&["--user-data", ud_s], &t.fixtures(&base));
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    // a1 and e5 swiped right; d4 too, but it's in the library.
    t.swipe(&["a1", "e5", "d4"]);
    let script =
        "https://mov.vrbangers.com/vrb_wet_and_wild/MISC/VRBANGERS_wet_and_wild_toy.funscript";
    let net = with(
        base,
        json!({
            "GET https://vrbangers.com/video/wet-and-wild/": scene_page(),
            "GET https://vrconk.com/login/": {"status": 403, "body": ""},
            format!("GET {script}"): ok(json!({"actions": [{"at": 0, "pos": 0}, {"at": 500, "pos": 100}]})),
            format!("GET {}", signed("8K")): ok(Value::String("VIDEO".into())),
        }),
    );
    let f = t.fixtures(&net);

    let o = run(&["--user-data", ud_s, "--fetch", "--dry-run"], &f);
    let out = stdout(&o);
    assert_eq!(o.status.code(), Some(0), "{out}{}", stderr(&o));
    assert!(
        out.contains("3 scene(s) swiped right: 2 to fetch, 1 fetched or in the library."),
        "{out}"
    );
    assert!(out.contains("video: 8K 180x180_3dh"), "{out}");
    assert!(out.contains("secure=<redacted>,1800000000"), "{out}");
    assert!(!out.contains(TOKEN), "the token stays secret");
    assert!(out.contains("needs a VRConk membership"), "{out}");
    assert!(out.contains("Dry run: nothing downloaded."));
    assert!(!t.other().join("Angel Youngs").exists());

    let o = run(&["--user-data", ud_s, "--fetch"], &f);
    let out = stdout(&o);
    assert_eq!(o.status.code(), Some(0), "{out}{}", stderr(&o));
    assert!(out.contains("Fetched 1, 0 failed, 1 skipped."), "{out}");
    let name = "Angel Youngs, Scarlette Moon - Wet & Wild In Malibu (VRBangers)_180x180_3dh";
    let dir = t.other().join("Angel Youngs").join(name);
    assert_eq!(
        fs::read_to_string(dir.join(format!("{name}.mp4"))).unwrap(),
        "VIDEO"
    );
    assert!(
        fs::read_to_string(dir.join(format!("{name}.funscript")))
            .unwrap()
            .contains("actions")
    );
    let sidecar: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("vrb.json")).unwrap()).unwrap();
    assert_eq!(sidecar["site"], "vrbangers");
    assert_eq!(sidecar["id"], "a1");
    assert_eq!(sidecar["video"]["quality"], "8K");
    assert!(!sidecar.to_string().contains(TOKEN));
    // Nothing left in staging.
    let staging = t.0.join("lib/_staging");
    assert_eq!(fs::read_dir(&staging).unwrap().count(), 0);
    let file = read_file(&ud);
    assert!(scene(&file, "a1").unwrap()["fetchedAt"].is_string());

    // Fetched scenes aren't fetched again; a scan keeps fetchedAt and marks it.
    let o = run(&["--user-data", ud_s, "--fetch"], &f);
    assert!(stdout(&o).contains("1 to fetch"), "{}", stdout(&o));
    let o = run(&["--user-data", ud_s], &f);
    assert_eq!(o.status.code(), Some(0));
    let a = read_file(&ud);
    let a = scene(&a, "a1").unwrap();
    assert!(a["fetchedAt"].is_string());
    assert_eq!(a["inLibrary"], true);
}

#[test]
fn a_video_without_disk_room_is_left_for_later() {
    let t = TempDir::new();
    let ud = t.user_data();
    let ud_s = ud.to_str().unwrap();
    let base = catalogue("Wet & Wild In Malibu");
    let o = run(&["--user-data", ud_s], &t.fixtures(&base));
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    t.swipe(&["a1"]);
    let script =
        "https://mov.vrbangers.com/vrb_wet_and_wild/MISC/VRBANGERS_wet_and_wild_toy.funscript";
    let f = t.fixtures(&with(
        base,
        json!({
            "GET https://vrbangers.com/video/wet-and-wild/": scene_page(),
            format!("GET {script}"): ok(json!({"actions": [{"at": 0, "pos": 0}, {"at": 500, "pos": 100}]})),
            format!("GET {}", signed("8K")): ok(Value::String("VIDEO".into())),
        }),
    ));
    // The 8K file is 5 bytes: 50 GB + 4 free is one byte short of keeping 50 GB free.
    let fetch = |free: u64| {
        Command::new(env!("CARGO_BIN_EXE_vrb-scout"))
            .args(["--user-data", ud_s, "--fetch"])
            .env_remove("FUNSYNC_USER_DATA")
            .env("VRB_SCOUT_FAKE_HTTP", &f)
            .env("FUNSYNC_FAKE_FREE_BYTES", free.to_string())
            .output()
            .unwrap()
    };
    let o = fetch(50_000_000_004);
    let out = stdout(&o);
    assert_eq!(o.status.code(), Some(2), "{out}{}", stderr(&o));
    assert!(
        out.contains("  skipped: not enough disk space (50 GB free, 50 GB needed to keep 50 GB free); left for later\n"),
        "{out}"
    );
    assert!(out.contains("Fetched 0, 0 failed, 1 skipped."), "{out}");
    assert!(!t.other().join("Angel Youngs").exists());

    let o = fetch(50_000_000_005);
    let out = stdout(&o);
    assert_eq!(o.status.code(), Some(0), "{out}{}", stderr(&o));
    assert!(out.contains("Fetched 1, 0 failed, 0 skipped."), "{out}");
}

#[test]
fn a_refused_login_is_exit_3() {
    let t = TempDir::new();
    let ud = t.user_data();
    let ud_s = ud.to_str().unwrap();
    let net = with(
        catalogue("Wet & Wild In Malibu"),
        json!({"GET https://vrbangers.com/login/": {"status": 401, "body": ""}}),
    );
    let f = t.fixtures(&net);
    assert_eq!(run(&["--user-data", ud_s], &f).status.code(), Some(0));
    t.swipe(&["a1"]);
    let o = run(&["--user-data", ud_s, "--fetch", "--dry-run"], &f);
    assert_eq!(o.status.code(), Some(3), "{}", stdout(&o));
    assert!(stdout(&o).contains("not logged in"), "{}", stdout(&o));
}

#[test]
fn fetch_needs_a_scan_first() {
    let t = TempDir::new();
    let ud = t.user_data();
    let o = run(
        &["--user-data", ud.to_str().unwrap(), "--fetch"],
        &t.fixtures(&json!({})),
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("doesn't exist yet"), "{}", stderr(&o));
}
