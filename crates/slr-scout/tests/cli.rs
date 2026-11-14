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
        format!("GET {FORUM}/search.json?q=%22Test%20Star%22&page=1"): ok(json!({
            "topics": [
                {"id": 273166, "title": "Test Star - Please Creampie Me [SLR]", "category_id": 15,
                 "tags": [{"name": "vr"}, {"name": "multi-axis"}]},
                {"id": 11, "title": "Test Star request", "category_id": 3}],
            "grouped_search_result": {"more_full_page_results": null}})),
        format!("GET {FORUM}/search.json?q=teststar&page=1"): ok(json!({
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
    // 1: 4 + 1.12; 2: 4 + 0.3; 3: 2 + 2 (capped); no creampie (4) or flat (5).
    assert_eq!(ids, [1, 2, 3]);
    assert_eq!(scenes[0]["score"], 5.12);
    assert_eq!(
        scenes[0]["scoreParts"],
        json!({"handmadeScript": 4, "popularity": 1.12})
    );
    assert_eq!(
        scenes[0]["handmade"],
        json!([{"source": "slr", "author": "kimera"}])
    );
    assert_eq!(scenes[0]["url"], format!("{SITE}/scenes/oiled-1"));
    assert_eq!(scenes[0]["date"], "2026-04-24");
    assert_eq!(scenes[0]["aiScript"], false);
    assert_eq!(scenes[1]["score"], 4.3);
    assert_eq!(
        scenes[1]["handmade"],
        json!([{"source": "eroscripts", "topicId": 273166,
                "title": "Test Star - Please Creampie Me [SLR]", "free": false, "multiAxis": true}])
    );
    assert_eq!(scenes[1]["aiScript"], true);
    assert_eq!(scenes[2]["compilation"], true);
    assert_eq!(scenes[2]["score"], 4);
    assert_eq!(
        scenes[2]["scoreParts"],
        json!({"compilation": 2, "popularity": 2})
    );
    assert_eq!(file["quota"]["aiScripts"]["remaining"], 3);
    assert_eq!(file["quotaError"], Value::Null);
    let out = stdout(&o);
    assert!(out.contains("Please Creampie Me"));
    assert!(out.contains("es#273166 paid"));
    assert!(out.contains("Quota: hand-crafted scripts 2/3, AI scripts 3/3, videos 35/35 left"));
    assert!(!out.contains("forum-secret") && !stderr(&o).contains("forum-secret"));

    // Without the forum or a session: earlier forum matches are kept, the
    // quota too, and the run exits 3.
    fs::remove_file(ud.join("slr-session.json")).unwrap();
    let fixtures = tmp.fixtures(&catalogue());
    let o = run(
        &["--user-data", ud_s, "--actor", "Test Star", "--no-forum"],
        Some(&fixtures),
    );
    assert_eq!(o.status.code(), Some(3), "{}", stderr(&o));
    let file = read_file(&ud);
    assert_eq!(file["scenes"][1]["handmade"][0]["topicId"], 273166);
    assert_eq!(file["scenes"].as_array().unwrap().len(), 3);
    assert_eq!(file["quotaError"], "no session");
    assert_eq!(file["quota"]["videos"]["remaining"], 35);
    assert!(stderr(&o).contains("EroScripts not searched (--no-forum)"));
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
    fixtures[format!("GET {FORUM}/search.json?q=%22Test%20Star%22&page=1")] =
        json!({"status": 429, "body": ""});
    let fixtures = tmp.fixtures(&fixtures);
    let o = run(
        &["--user-data", ud.to_str().unwrap(), "--actor", "Test Star"],
        Some(&fixtures),
    );
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    let file = read_file(&ud);
    assert_eq!(file["scenes"].as_array().unwrap().len(), 3);
    assert_eq!(file["quota"]["scripts"]["remaining"], 2);
    assert!(stderr(&o).contains("rate limited"));
}
