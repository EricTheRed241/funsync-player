//! End to end through the binary, against a temp userData, with the network
//! faked: `SLR_SCOUT_FAKE_HTTP` points the CLI at canned responses.

use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

const API: &str = "https://api.sexlikereal.com";
const SITE: &str = "https://www.sexlikereal.com";
const FORUM: &str = "https://discuss.eroscripts.com";

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "slr-scout-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("userData")).unwrap();
        TempDir(dir)
    }

    fn user_data(&self) -> PathBuf {
        self.0.join("userData")
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
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_slr-scout"));
    cmd.args(args).env_remove("FUNSYNC_USER_DATA");
    match fixtures {
        Some(f) => cmd.env("SLR_SCOUT_FAKE_HTTP", f),
        // An unreadable fixture file fails the run before any request:
        // these runs never reach the network either.
        None => cmd.env("SLR_SCOUT_FAKE_HTTP", "/nonexistent/fixtures.json"),
    };
    cmd.output().unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn read_file(user_data: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(user_data.join("slr-scout.json")).unwrap()).unwrap()
}

fn ok(body: Value) -> Value {
    json!({ "status": 200, "body": body })
}

fn detail(id: u64, label: &str, title: &str, favorites: u64, extra: Value) -> Value {
    let mut d = json!({
        "id": id, "label": label, "title": title, "isFlatScreenType": false,
        "date": 1777071600, "fullVideoLength": 1629, "fps": 60,
        "passthrough": {"chromaKey": {"enabled": false}, "alpha": {"enabled": false}},
        "thumbnailUrl": format!("https://cdn-vr.sexlikereal.com/{id}.webp"),
        "quantity": {"favorites": favorites}, "studio": {"name": "Star VR"},
        "actors": [{"name": "Test Star"}],
        "scripts": [{"id": 1, "scriptAI": true, "author": "scriptai"}],
        "timestamps": [{"name": "Blow Job"}], "categories": [{"name": "3D"}]
    });
    for (k, v) in extra.as_object().unwrap() {
        d[k] = v.clone();
    }
    ok(json!({ "data": d }))
}

/// A performer with four scenes: a creampie with an SLR hand-crafted
/// script, a creampie matched on the forum, a creampie compilation, a
/// scene without creampie and a flat one.
fn catalogue() -> Value {
    json!({
        format!("GET {API}/v3/search?query=Test%20Star&tab=actors"): ok(json!({"data": [
            {"id": 42, "name": "Test Star", "label": "test-star", "quantities": {"scenes": 5}}]})),
        format!("GET {API}/v3/scenes?models=42&perPage=36&page=1"): ok(json!({
            "data": [
                {"id": 1, "label": "oiled-1", "isFlatScreenType": false},
                {"id": 2, "label": "please-2", "isFlatScreenType": false},
                {"id": 3, "label": "best-3", "isFlatScreenType": false}],
            "meta": {"pagination": {"totalPages": 2}}})),
        format!("GET {API}/v3/scenes?models=42&perPage=36&page=2"): ok(json!({
            "data": [
                {"id": 4, "label": "plain-4", "isFlatScreenType": false},
                {"id": 5, "label": "flat-5", "isFlatScreenType": true}],
            "meta": {"pagination": {"totalPages": 2}}})),
        format!("GET {API}/v3/scenes/oiled-1"): detail(1, "oiled-1", "Oiled Up And Creampied", 1123, json!({
            "scripts": [{"id": 9, "scriptAI": false, "author": "kimera"}],
            "categories": [{"name": "Creampie"}]})),
        format!("GET {API}/v3/scenes/please-2"): detail(2, "please-2", "Please Creampie Me", 300, json!({
            "timestamps": [{"name": "Cowgirl"}, {"name": "Creampie"}]})),
        format!("GET {API}/v3/scenes/best-3"): detail(3, "best-3", "25 Top Babes", 5000, json!({
            "categories": [{"name": "Compilation"}, {"name": "Creampie"}]})),
        format!("GET {API}/v3/scenes/plain-4"): detail(4, "plain-4", "Just A Blowjob", 9000, json!({})),
    })
}

fn forum() -> Value {
    json!({
        format!("GET {FORUM}/search.json?q=%22Test%20Star%22%20in%3Atitle&page=1"): ok(json!({
            "topics": [
                {"id": 273166, "title": "Test Star - Please Creampie Me [SLR]", "category_id": 15,
                 "tags": [{"name": "vr"}, {"name": "multi-axis"}]},
                {"id": 11, "title": "Test Star request", "category_id": 3}],
            "grouped_search_result": {"more_full_page_results": null}})),
        format!("GET {FORUM}/search.json?q=teststar%20in%3Atitle&page=1"): ok(json!({
            "topics": [], "grouped_search_result": {}})),
    })
}

fn quota_ok() -> Value {
    json!({
        format!("POST {SITE}/api/get-access-token"): {
            "status": 200,
            "headers": [["set-cookie", "refresh_jwt=rotated; Max-Age=2592000; Path=/"]],
            "body": {"accessToken": "tok"}
        },
        format!("GET {API}/users/v1/user/overview"): ok(json!({"data": {"limits": {
            "downloadScript": {"remaining": 2, "total": 3, "usageExpiresAt": 1795000000},
            "downloadAiScript": {"remaining": 3, "total": 3, "usageExpiresAt": null},
            "downloadVideo": {"remaining": 35, "total": 35, "usageExpiresAt": null}}}}))
    })
}

fn merged(parts: &[Value]) -> Value {
    let mut all = serde_json::Map::new();
    for p in parts {
        all.extend(p.as_object().unwrap().clone());
    }
    Value::Object(all)
}

fn write_session(user_data: &Path) {
    fs::write(
        user_data.join("slr-session.json"),
        json!({"refreshJwt": "old", "expires": "2026-12-13T00:00:00.000Z", "savedAt": "x"})
            .to_string(),
    )
    .unwrap();
}

fn write_config(user_data: &Path) {
    fs::write(
        user_data.join("config.json"),
        json!({"settings": {"eroscripts": {"session": {"cookie": "forum-secret"}}}}).to_string(),
    )
    .unwrap();
}

#[test]
fn help_and_bad_arguments() {
    let o = run(&["--help"], None);
    assert_eq!(o.status.code(), Some(0));
    assert!(stdout(&o).contains("Usage:"));
    let o = run(&["--bogus"], None);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("unknown argument --bogus"));
    let o = run(&["--quota", "--actor", "x"], None);
    assert_eq!(o.status.code(), Some(1));
    let o = run(&["--user-data", "/nonexistent/slr-scout"], None);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("is not a folder"));
}

#[test]
fn quota_without_a_session_exits_3_and_keeps_the_old_quota() {
    let tmp = TempDir::new();
    let ud = tmp.user_data();
    let ud_s = ud.to_str().unwrap();
    let empty = tmp.fixtures(&json!({}));
    let o = run(&["--user-data", ud_s, "--quota"], Some(&empty));
    assert_eq!(o.status.code(), Some(3), "{}", stderr(&o));
    let file = read_file(&ud);
    assert_eq!(file["quota"], Value::Null);
    assert_eq!(file["quotaError"], "no session");
    assert_eq!(file["scenes"], json!([]));
    assert!(stdout(&o).contains("--login"));

    // With a session the quota is read; the rotated cookie is saved.
    write_session(&ud);
    let fixtures = tmp.fixtures(&quota_ok());
    let o = run(&["--user-data", ud_s, "--quota"], Some(&fixtures));
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let file = read_file(&ud);
    assert_eq!(file["quotaError"], Value::Null);
    assert_eq!(
        file["quota"]["scripts"],
        json!({"remaining": 2, "total": 3, "usageExpiresAt": 1795000000})
    );
    let session: Value =
        serde_json::from_str(&fs::read_to_string(ud.join("slr-session.json")).unwrap()).unwrap();
    assert_eq!(session["refreshJwt"], "rotated");
    assert!(!stdout(&o).contains("rotated") && !stdout(&o).contains("tok"));

    // An expired session: exit 3, the old quota kept beside the error.
    let fixtures = tmp.fixtures(&json!({
        format!("POST {SITE}/api/get-access-token"): {"status": 401, "body": {}}
    }));
    let o = run(&["--user-data", ud_s, "--quota"], Some(&fixtures));
    assert_eq!(o.status.code(), Some(3));
    let file = read_file(&ud);
    assert_eq!(file["quotaError"], "session expired");
    assert_eq!(file["quota"]["videos"]["total"], 35);
    assert!(ud.join("slr-scout.json.bak").exists());
    assert!(!ud.join("slr-scout.lock").exists());
}

#[test]
fn full_run_scores_and_writes_the_scenes() {
    let tmp = TempDir::new();
    let ud = tmp.user_data();
    let ud_s = ud.to_str().unwrap();
    write_session(&ud);
    write_config(&ud);
    let fixtures = tmp.fixtures(&merged(&[catalogue(), forum(), quota_ok()]));
    let o = run(
        &["--user-data", ud_s, "--actor", "Test Star"],
        Some(&fixtures),
    );
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let file = read_file(&ud);
    assert_eq!(
        file["actors"],
        json!([{"name": "Test Star", "id": 42, "label": "test-star", "scenes": 5}])
    );
    let scenes = file["scenes"].as_array().unwrap();
    let ids: Vec<u64> = scenes.iter().map(|s| s["id"].as_u64().unwrap()).collect();
    // 2: VR 3 + likes 0.2 + creampie 2 + hand-made 4 + multi-axis 4;
    // 3: VR 3 + likes 3 (capped) + creampie 2 + compilation tag 2 + bonus 2;
    // 1: VR 3 + likes 0.75 + creampie 2 + hand-made 4; 4: VR 3 + likes 3.
    // The flat scene (5) is left out.
    assert_eq!(ids, [2, 3, 1, 4]);
    assert_eq!(scenes[0]["score"], 13.2);
    assert_eq!(
        scenes[0]["scoreParts"],
        json!({"handmadeScript": 4, "likes": 0.2, "multiAxis": 4, "tag:creampie": 2, "vr": 3})
    );
    assert_eq!(
        scenes[0]["handmade"],
        json!([{"source": "eroscripts", "topicId": 273166,
                "title": "Test Star - Please Creampie Me [SLR]", "free": false, "multiAxis": true}])
    );
    // SLR's 3D category means stereoscopic: not the animation tag.
    assert_eq!(scenes[0]["tags"], json!(["creampie"]));
    assert_eq!(scenes[0]["tagScores"], json!({"creampie": 2}));
    // A paid post's script isn't free: SLR's AI script.
    assert_eq!(scenes[0]["scriptSource"], "slr-ai");
    assert_eq!(scenes[1]["score"], 12);
    assert_eq!(scenes[1]["compilation"], true);
    assert_eq!(scenes[2]["score"], 9.75);
    assert_eq!(
        scenes[2]["handmade"],
        json!([{"source": "slr", "author": "kimera"}])
    );
    assert_eq!(scenes[2]["scriptSource"], "slr-handcrafted");
    assert_eq!(scenes[2]["url"], format!("{SITE}/scenes/oiled-1"));
    assert_eq!(scenes[2]["date"], "2026-04-24");
    assert_eq!(scenes[3]["score"], 6);
    assert_eq!(scenes[3]["tags"], json!([]));
    assert!(scenes.iter().all(|s| s["fetchedAt"].is_null()));
    assert!(scenes.iter().all(|s| s["favourite"].is_null()));
    assert_eq!(file["quota"]["aiScripts"]["remaining"], 3);
    assert_eq!(file["quotaError"], Value::Null);
    let out = stdout(&o);
    assert!(out.contains("4 scene(s) (VR) from 1 performer(s) on SLR:"));
    assert!(out.contains("Test Star: 5 on SLR, 4 kept"));
    assert!(out.contains("Please Creampie Me"));
    assert!(out.contains("es#273166 paid"));
    assert!(out.contains("Scripts: 0 from EroScripts, 1 SLR hand-crafted, 3 SLR AI, 0 none."));
    assert!(out.contains("Quota: hand-crafted scripts 2/3, AI scripts 3/3, videos 35/35 left"));
    assert!(!out.contains("forum-secret") && !stderr(&o).contains("forum-secret"));

    // Without the forum or a session: earlier forum matches are kept, the
    // quota too, fetchedAt as well, and the run exits 3.
    let mut file = read_file(&ud);
    file["scenes"][2]["fetchedAt"] = json!("2026-11-13T10:00:00.000Z");
    fs::write(ud.join("slr-scout.json"), file.to_string()).unwrap();
    fs::remove_file(ud.join("slr-session.json")).unwrap();
    let fixtures = tmp.fixtures(&catalogue());
    let o = run(
        &["--user-data", ud_s, "--actor", "Test Star", "--no-forum"],
        Some(&fixtures),
    );
    assert_eq!(o.status.code(), Some(3), "{}", stderr(&o));
    let file = read_file(&ud);
    assert_eq!(file["scenes"][0]["handmade"][0]["topicId"], 273166);
    assert_eq!(file["scenes"][0]["scoreParts"]["multiAxis"], 4);
    assert_eq!(file["scenes"].as_array().unwrap().len(), 4);
    assert_eq!(file["scenes"][2]["id"], 1);
    assert_eq!(file["scenes"][2]["fetchedAt"], "2026-11-13T10:00:00.000Z");
    assert_eq!(file["quotaError"], "no session");
    assert_eq!(file["quota"]["videos"]["remaining"], 35);
    assert!(stderr(&o).contains("EroScripts not searched (--no-forum)"));
}

#[test]
fn unknown_performers_are_skipped_and_listed() {
    let tmp = TempDir::new();
    let ud = tmp.user_data();
    let mut fixtures = catalogue();
    fixtures[format!("GET {API}/v3/search?query=Nobody%20Here&tab=actors")] =
        ok(json!({"data": [{"id": 1, "name": "Nobody Else"}]}));
    let fixtures = tmp.fixtures(&fixtures);
    let o = run(
        &[
            "--user-data",
            ud.to_str().unwrap(),
            "--actor",
            "Nobody Here",
            "--actor",
            "Test Star",
            "--no-forum",
        ],
        Some(&fixtures),
    );
    assert_eq!(o.status.code(), Some(3), "{}", stderr(&o));
    assert!(stdout(&o).contains("Not on SLR (skipped): Nobody Here"));
    assert!(stderr(&o).contains("[1/2] Nobody Here: not on SLR"));
    assert_eq!(read_file(&ud)["scenes"].as_array().unwrap().len(), 4);
}

#[test]
fn rate_limited_catalogue_exits_2_and_writes_nothing() {
    let tmp = TempDir::new();
    let ud = tmp.user_data();
    let fixtures = tmp.fixtures(&json!({
        format!("GET {API}/v3/search?query=Test%20Star&tab=actors"): {"status": 429, "body": ""}
    }));
    let o = run(
        &["--user-data", ud.to_str().unwrap(), "--actor", "Test Star"],
        Some(&fixtures),
    );
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(!ud.join("slr-scout.json").exists());
}

#[test]
fn rate_limited_forum_keeps_the_scenes_and_exits_2() {
    let tmp = TempDir::new();
    let ud = tmp.user_data();
    write_session(&ud);
    write_config(&ud);
    let mut fixtures = merged(&[catalogue(), quota_ok()]);
    fixtures[format!("GET {FORUM}/search.json?q=%22Test%20Star%22%20in%3Atitle&page=1")] =
        json!({"status": 429, "body": ""});
    let fixtures = tmp.fixtures(&fixtures);
    let o = run(
        &["--user-data", ud.to_str().unwrap(), "--actor", "Test Star"],
        Some(&fixtures),
    );
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    let file = read_file(&ud);
    assert_eq!(file["scenes"].as_array().unwrap().len(), 4);
    assert_eq!(file["quota"]["scripts"]["remaining"], 2);
    assert!(stderr(&o).contains("rate limited"));
}

// ── --fetch ─────────────────────────────────────────────────────────────

/// A saved scene (slr-scout.json) for the fetch tests.
fn saved(id: u64, title: &str, score: f64, extra: Value) -> Value {
    let mut s = json!({
        "id": id, "label": format!("s-{id}"), "url": format!("{SITE}/scenes/s-{id}"),
        "title": title, "studio": "Star VR", "actors": ["Test Star", "Co Star"],
        "date": "2026-04-24", "durationS": 1629, "thumbnail": null, "vr": true, "fps": 60,
        "passthrough": false, "creampie": true, "handmade": [], "aiScript": false,
        "pmv": false, "compilation": false, "favorites": 100, "tags": ["creampie"],
        "tagScores": {"creampie": 2}, "favourite": null, "fetchedAt": null,
        "scriptSource": null, "score": score, "scoreParts": {"vr": 3}
    });
    for (k, v) in extra.as_object().unwrap() {
        s[k] = v.clone();
    }
    s
}

/// Six swiped scenes: a free EroScripts post's (11), SLR hand-crafted (12),
/// no script (13), SLR AI (14), already fetched (15), swiped left (16);
/// and a right swipe for a scene the file no longer has (99).
fn fetch_setup(tmp: &TempDir, video_quota: i64, held: usize) -> (PathBuf, PathBuf) {
    let ud = tmp.user_data();
    let lib = tmp.0.join("lib");
    let other = lib.join("Other");
    let es = lib.join("EroScripts");
    fs::create_dir_all(&other).unwrap();
    fs::create_dir_all(es.join("Artist")).unwrap();
    for i in 0..held {
        fs::write(es.join(format!("Artist/v{i}.mp4")), "").unwrap();
    }
    fs::write(
        ud.join("config.json"),
        json!({"settings": {
            "eroscripts": {"session": {"cookie": "forum-secret"}},
            "library": {"sources": [
                {"name": "EroScripts", "path": es, "enabled": true},
                {"name": "Other", "path": other, "enabled": true}]}}})
        .to_string(),
    )
    .unwrap();
    write_session(&ud);
    let free = json!([{"source": "eroscripts", "topicId": 500, "title": "Test Star - A Title",
                       "free": true, "multiAxis": true}]);
    let file = json!({
        "generatedAt": "2026-11-13T00:00:00.000Z", "actors": [], "quota": null, "quotaError": null,
        "scenes": [
            saved(11, "A Title", 20.0, json!({"handmade": free, "scriptSource": "eroscripts"})),
            saved(12, "B Title", 19.0, json!({"handmade": [{"source": "slr", "author": "k"}],
                                               "scriptSource": "slr-handcrafted"})),
            saved(13, "C Title", 18.0, json!({})),
            saved(14, "D Title", 17.0, json!({"aiScript": true, "scriptSource": "slr-ai"})),
            saved(15, "E Title", 16.0, json!({"fetchedAt": "2026-11-01T00:00:00.000Z"})),
            saved(16, "F Title", 15.0, json!({})),
        ]
    });
    fs::write(ud.join("slr-scout.json"), file.to_string()).unwrap();
    let mut swipes = serde_json::Map::new();
    for id in [11, 12, 13, 14, 15, 99] {
        swipes.insert(id.to_string(), json!({"dir": "right", "at": "x"}));
    }
    swipes.insert("16".into(), json!({"dir": "left", "at": "x"}));
    fs::write(
        ud.join("slr-scout-swipes.json"),
        json!({"version": 1, "swipes": swipes}).to_string(),
    )
    .unwrap();

    let detail = |id: u64, scripts: Value| {
        ok(
            json!({"data": {"id": id, "label": format!("s-{id}"), "isFlatScreenType": false,
            "fps": 60, "screenType": 0, "viewAngle": 180, "stereomode": "sbs2l", "scripts": scripts}}),
        )
    };
    let options = ok(json!({"data": {"isVideoPurchased": false, "options": [
        {"type": "trailer", "height": 1080, "size": 10},
        {"type": "maxResolution", "height": 4096, "width": 8192, "size": 7_000_000_000u64, "codec": "h265"},
        {"type": "optimalResolution", "height": 2880, "size": 3_000_000_000u64, "codec": "h265"}]}}));
    let mut f = quota_ok();
    f[format!("GET {API}/users/v1/user/overview")] = ok(json!({"data": {"limits": {
        "downloadScript": {"remaining": 1, "total": 3},
        "downloadAiScript": {"remaining": 3, "total": 3},
        "downloadVideo": {"remaining": video_quota, "total": 35}}}}));
    let more = json!({
        format!("GET {API}/v3/scenes/s-11"): detail(11, json!([])),
        format!("GET {API}/v3/scenes/s-12"): detail(12, json!([{"id": 77, "scriptAI": false}])),
        format!("GET {API}/v3/scenes/s-13"): detail(13, json!([])),
        format!("GET {API}/v3/scenes/s-14"): detail(14, json!([{"id": 78, "scriptAI": true}])),
        format!("GET {FORUM}/t/500.json"): ok(json!({"post_stream": {"posts": [{"cooked":
            "<p><a class=\"attachment\" href=\"/uploads/short-url/main.funscript\">Test Star - A Title.funscript</a> (9 KB)<br>\
             <a class=\"attachment\" href=\"/uploads/short-url/tw.funscript\">Test Star - A Title.twist.funscript</a>\
             <a href=\"https://mega.nz/file/x\">video</a></p>"}]}})),
        format!("GET {FORUM}/uploads/short-url/main.funscript"): {"status": 302,
            "headers": [["location", "https://cdn.forum.test/main.funscript"]], "body": ""},
        format!("GET {FORUM}/uploads/short-url/tw.funscript"): {"status": 302,
            "headers": [["location", "https://cdn.forum.test/tw.funscript"]], "body": ""},
        "GET https://cdn.forum.test/main.funscript": ok(json!({"actions": [{"at": 0, "pos": 10}]})),
        "GET https://cdn.forum.test/tw.funscript": ok(json!({"actions": [{"at": 0, "pos": 50}]})),
        format!("GET {API}/content/v1/videos/11/download-options"): options.clone(),
        format!("GET {API}/content/v1/videos/12/download-options"): options.clone(),
        format!("GET {API}/content/v1/videos/14/download-options"): options,
        format!("GET {API}/content/v1/videos/11/download?type=maxResolution"):
            ok(json!({"data": {"url": "https://cdn.slr.test/11.mp4", "availableDownloadsCount": 1}})),
        format!("GET {API}/content/v1/videos/12/download?type=maxResolution"):
            ok(json!({"data": {"url": "https://cdn.slr.test/12.mp4", "availableDownloadsCount": 0}})),
        "GET https://cdn.slr.test/11.mp4": {"status": 200, "body": "VIDEO-11"},
        "GET https://cdn.slr.test/12.mp4": {"status": 200, "body": "VIDEO-12"},
        format!("GET {API}/content/v1/scripts/77/download"):
            ok(json!({"data": {"link": "https://cdn.slr.test/77.funscript",
                               "scriptDownloadLimits": {"availableDownloadsCount": 0}}})),
        "GET https://cdn.slr.test/77.funscript": ok(json!({"actions": [{"at": 5, "pos": 90}]})),
    });
    f.as_object_mut()
        .unwrap()
        .extend(more.as_object().unwrap().clone());
    (ud, tmp.fixtures(&f))
}

const SCENE_A: &str = "Test Star/Test Star, Co Star - A Title (Star VR)_180_LR";
const SCENE_B: &str = "Test Star/Test Star, Co Star - B Title (Star VR)_180_LR";

#[test]
fn fetch_dry_run_prints_the_plan_and_downloads_nothing() {
    let tmp = TempDir::new();
    let (ud, fixtures) = fetch_setup(&tmp, 35, 3);
    let before = fs::read_to_string(ud.join("slr-scout.json")).unwrap();
    let o = run(
        &["--user-data", ud.to_str().unwrap(), "--fetch", "--dry-run"],
        Some(&fixtures),
    );
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains(
        "6 scene(s) swiped right: 4 to fetch, 1 already fetched, 1 no longer in slr-scout.json."
    ));
    let other = tmp.0.join("lib/Other");
    assert!(out.contains(&format!(
        "video: maxResolution 4096p 6.52 GB h265 → {}",
        other
            .join(SCENE_A)
            .join("Test Star, Co Star - A Title (Star VR)_180_LR.mp4")
            .display()
    )));
    assert!(out.contains("script: EroScripts #500 \"Test Star - A Title.funscript\" + twist"));
    assert!(out.contains("script: SLR hand-crafted script #77"));
    assert!(out.contains("#13 C Title\n  skipped: no script source"));
    assert!(out.contains("script: SLR AI script #78"));
    assert!(out.contains("Dry run: nothing downloaded."));
    assert!(!out.contains("forum-secret"));
    // Nothing written: no scene folder, the file as it was.
    assert!(!other.join("Test Star").exists());
    assert!(!tmp.0.join("lib/_staging").exists());
    assert_eq!(
        fs::read_to_string(ud.join("slr-scout.json")).unwrap(),
        before
    );
}

#[test]
fn fetch_downloads_until_the_video_quota_runs_out() {
    let tmp = TempDir::new();
    let (ud, fixtures) = fetch_setup(&tmp, 2, 3);
    let o = run(
        &["--user-data", ud.to_str().unwrap(), "--fetch"],
        Some(&fixtures),
    );
    let out = stdout(&o);
    // 11 and 12 fetched (SLR says no video download is left after 12), so
    // the run stops at 13 before any request.
    assert_eq!(o.status.code(), Some(2), "{out}\n{}", stderr(&o));
    assert!(out.contains("#13 C Title\n  stopped: the weekly video download quota is used up"));
    assert!(!out.contains("#14"));
    assert!(out.contains("Fetched 2, 0 failed, 0 skipped."));
    let other = tmp.0.join("lib/Other");
    let a = other.join(SCENE_A);
    let stem = "Test Star, Co Star - A Title (Star VR)_180_LR";
    assert_eq!(
        fs::read_to_string(a.join(format!("{stem}.mp4"))).unwrap(),
        "VIDEO-11"
    );
    assert!(
        fs::read_to_string(a.join(format!("{stem}.funscript")))
            .unwrap()
            .contains("\"pos\":10")
    );
    assert!(a.join(format!("{stem}.twist.funscript")).exists());
    let sidecar: Value =
        serde_json::from_str(&fs::read_to_string(a.join("slr.json")).unwrap()).unwrap();
    assert_eq!(sidecar["sceneId"], 11);
    assert_eq!(sidecar["url"], format!("{SITE}/scenes/s-11"));
    assert_eq!(sidecar["performers"], json!(["Test Star", "Co Star"]));
    assert_eq!(sidecar["studio"], "Star VR");
    assert_eq!(sidecar["scriptSource"], "eroscripts");
    assert_eq!(sidecar["script"]["topicId"], 500);
    assert_eq!(sidecar["video"]["type"], "maxResolution");
    let b = other.join(SCENE_B);
    let stem_b = "Test Star, Co Star - B Title (Star VR)_180_LR";
    assert!(
        fs::read_to_string(b.join(format!("{stem_b}.funscript")))
            .unwrap()
            .contains("\"pos\":90")
    );
    let sidecar: Value =
        serde_json::from_str(&fs::read_to_string(b.join("slr.json")).unwrap()).unwrap();
    assert_eq!(sidecar["scriptSource"], "slr-handcrafted");
    assert_eq!(sidecar["script"]["scriptId"], 77);
    // Nothing left in staging; fetchedAt set for the two only.
    let staging: Vec<_> = fs::read_dir(tmp.0.join("lib/_staging"))
        .unwrap()
        .flatten()
        .collect();
    assert!(staging.is_empty());
    let file = read_file(&ud);
    let fetched: Vec<u64> = file["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| !s["fetchedAt"].is_null())
        .map(|s| s["id"].as_u64().unwrap())
        .collect();
    assert_eq!(fetched, [11, 12, 15]);
    assert!(!stdout(&o).contains("forum-secret") && !stderr(&o).contains("forum-secret"));

    // A second run has nothing new but 13 and 14; 14 is still over quota.
    let o = run(
        &["--user-data", ud.to_str().unwrap(), "--fetch"],
        Some(&fixtures),
    );
    assert!(stdout(&o).contains("6 scene(s) swiped right: 2 to fetch, 3 already fetched"));
}

#[test]
fn fetch_stops_at_the_library_cap_and_without_a_session() {
    let tmp = TempDir::new();
    let cap = 1000; // libraryMaxVideos
    let (ud, fixtures) = fetch_setup(&tmp, 35, cap);
    let o = run(
        &["--user-data", ud.to_str().unwrap(), "--fetch", "--dry-run"],
        Some(&fixtures),
    );
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(stdout(&o).contains("Library holds 1000 videos, the cap is 1000"));
    assert!(!stdout(&o).contains("#11"));

    fs::remove_file(ud.join("slr-session.json")).unwrap();
    let o = run(
        &["--user-data", ud.to_str().unwrap(), "--fetch"],
        Some(&fixtures),
    );
    assert_eq!(o.status.code(), Some(3), "{}", stderr(&o));
    assert!(
        stdout(&o).contains(
            "Nothing fetched: SLR no session; sign in with npm run slr:scout -- --login."
        )
    );
}
