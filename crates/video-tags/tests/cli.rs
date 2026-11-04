//! End to end through the binary, against temp folders: a userData with a
//! sources-only config.json and a library source with fake videos.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "video-tags-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // The temp dir itself may sit behind a symlink (/var → /private/var
        // on macOS); paths are compared in their on-disk spelling, which
        // follows the listing, not the link target, so keep it as is.
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Env {
    _tmp: TempDir,
    user_data: PathBuf,
    library: PathBuf,
    outside: PathBuf,
}

const COOKIE: &str = "SECRET-COOKIE-VALUE";

fn env() -> Env {
    let tmp = TempDir::new();
    let user_data = tmp.0.join("userData");
    let library = tmp.0.join("GVP").join("Other");
    let outside = tmp.0.join("Elsewhere");
    fs::create_dir_all(&user_data).unwrap();
    fs::create_dir_all(library.join("Artist").join("Scene")).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let config = serde_json::json!({ "settings": {
        "library": { "sources": [{ "id": "1", "name": "Other", "path": library, "enabled": true }] },
        "eroscripts": { "session": { "cookie": COOKIE } },
    } });
    fs::write(
        user_data.join("config.json"),
        serde_json::to_string(&config).unwrap(),
    )
    .unwrap();
    Env {
        _tmp: tmp,
        user_data,
        library,
        outside,
    }
}

fn video(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, b"not really a video").unwrap();
    p
}

fn run(env: &Env, args: &[&str]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_video-tags"))
        .arg("--user-data")
        .arg(&env.user_data)
        .args(args)
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!all.contains(COOKIE), "printed the session cookie");
    out
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn proposals(env: &Env, json: serde_json::Value) -> String {
    let p = env.user_data.parent().unwrap().join("proposals.json");
    fs::write(&p, json.to_string()).unwrap();
    p.to_str().unwrap().to_string()
}

fn stored(env: &Env) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(env.user_data.join("video-tags.json")).unwrap())
        .unwrap()
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn dry_run_writes_nothing_and_apply_writes_atomically_with_a_backup() {
    let env = env();
    let a = video(&env.library.join("Artist").join("Scene"), "a.mp4");
    let p = proposals(
        &env,
        serde_json::json!({ "by": "tagger", "videos": { s(&a): { "tags": ["POV", "creampie"], "confidence": 0.9 } } }),
    );

    let o = run(&env, &["--proposals", &p]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(
        stdout(&o).contains(&format!("new        {}: + pov + creampie", s(&a))),
        "{}",
        stdout(&o)
    );
    assert!(stdout(&o).contains("Dry run: nothing written"));
    assert_eq!(
        fs::read_dir(&env.user_data).unwrap().count(),
        1,
        "only config.json"
    );

    let config_before = fs::read(env.user_data.join("config.json")).unwrap();
    let o = run(&env, &["--proposals", &p, "--apply", "--no-rescan"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let file = stored(&env);
    assert_eq!(file["version"], 1);
    assert_eq!(
        file["videos"][s(&a)]["tags"],
        serde_json::json!(["pov", "creampie"])
    );
    assert_eq!(file["videos"][s(&a)]["by"], "tagger");
    assert!(file["videos"][s(&a)].get("confidence").is_none());
    assert!(
        !env.user_data.join("video-tags.json.bak").exists(),
        "no previous file to keep"
    );

    // Union: a second proposal adds; the previous file becomes the .bak.
    let first = fs::read_to_string(env.user_data.join("video-tags.json")).unwrap();
    let p2 = proposals(
        &env,
        serde_json::json!([{ "path": s(&a), "tags": ["riding", "pov"] }]),
    );
    let o = run(&env, &["--proposals", &p2, "--apply", "--no-rescan"]);
    assert!(
        stdout(&o).contains("+ riding (kept pov, creampie)"),
        "{}",
        stdout(&o)
    );
    assert_eq!(
        stored(&env)["videos"][s(&a)]["tags"],
        serde_json::json!(["pov", "creampie", "riding"])
    );
    assert_eq!(
        fs::read_to_string(env.user_data.join("video-tags.json.bak")).unwrap(),
        first
    );
    let mut names: Vec<String> = fs::read_dir(&env.user_data)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["config.json", "video-tags.json", "video-tags.json.bak"],
        "no temp file left"
    );
    assert_eq!(
        fs::read(env.user_data.join("config.json")).unwrap(),
        config_before,
        "config.json untouched"
    );

    // Same again: nothing to change, nothing written.
    let o = run(&env, &["--proposals", &p2, "--apply", "--no-rescan"]);
    assert!(stdout(&o).contains("Nothing to change."));

    // Replace sets exactly; an empty list removes.
    let p3 = proposals(&env, serde_json::json!([{ "path": s(&a), "tags": ["vr"] }]));
    let o = run(
        &env,
        &["--proposals", &p3, "--replace", "--apply", "--no-rescan"],
    );
    assert!(
        stdout(&o).contains("+ vr - pov - creampie - riding"),
        "{}",
        stdout(&o)
    );
    assert_eq!(
        stored(&env)["videos"][s(&a)]["tags"],
        serde_json::json!(["vr"])
    );
    let p4 = proposals(&env, serde_json::json!([{ "path": s(&a), "tags": [] }]));
    run(
        &env,
        &["--proposals", &p4, "--replace", "--apply", "--no-rescan"],
    );
    assert_eq!(stored(&env)["videos"], serde_json::json!({}));
}

#[test]
fn remove_and_list() {
    let env = env();
    let a = video(&env.library, "a.mp4");
    let b = video(&env.library, "b.mp4");
    let p = proposals(
        &env,
        serde_json::json!([{ "path": s(&a), "tags": ["pov"] }, { "path": s(&b), "tags": ["vr"] }]),
    );
    assert_eq!(
        run(&env, &["--proposals", &p, "--apply", "--no-rescan"])
            .status
            .code(),
        Some(0)
    );

    let o = run(&env, &["--list"]);
    assert!(stdout(&o).contains("2 video(s)"));
    assert!(
        stdout(&o).contains(&format!("  {}\n      pov  (added ", s(&a))),
        "{}",
        stdout(&o)
    );

    // A dry run by default; an unknown path fails the whole removal.
    let o = run(&env, &["--remove", s(&a)]);
    assert!(stdout(&o).contains(&format!("removed    {}: - pov", s(&a))));
    assert_eq!(stored(&env)["videos"].as_object().unwrap().len(), 2);
    let o = run(
        &env,
        &[
            "--remove",
            s(&a),
            "--remove",
            "/no/such.mp4",
            "--apply",
            "--no-rescan",
        ],
    );
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(stored(&env)["videos"].as_object().unwrap().len(), 2);

    // A video already deleted from disk can still be removed.
    fs::remove_file(&a).unwrap();
    let o = run(&env, &["--remove", s(&a), "--apply", "--no-rescan"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let videos = stored(&env)["videos"].clone();
    assert_eq!(
        videos.as_object().unwrap().keys().collect::<Vec<_>>(),
        vec![s(&b)]
    );
}

#[test]
fn invalid_entries_write_nothing() {
    let env = env();
    let good = video(&env.library, "good.mp4");
    let apple = video(&env.library, "._good.mp4");
    let outside = video(&env.outside, "x.mp4");
    let cases = [
        (
            serde_json::json!([{ "path": s(&good), "tags": ["big tits"] }]),
            "invalid tag \"big tits\"",
        ),
        (
            serde_json::json!([{ "path": s(&apple), "tags": ["pov"] }]),
            "AppleDouble",
        ),
        (
            serde_json::json!([{ "path": s(&outside), "tags": ["pov"] }]),
            "not inside a library source",
        ),
        (
            serde_json::json!([{ "path": "relative/x.mp4", "tags": ["pov"] }]),
            "not an absolute path",
        ),
        (
            serde_json::json!([{ "path": s(&env.library.join("missing.mp4")), "tags": ["pov"] }]),
            "doesn't exist",
        ),
        (
            serde_json::json!([{ "path": s(&env.library), "tags": ["pov"] }]),
            "not a regular file",
        ),
        (
            serde_json::json!([{ "path": s(&good), "tags": "pov" }]),
            "no \"tags\" array",
        ),
    ];
    for (json, reason) in cases {
        // One bad entry next to a good one: nothing is written.
        let mut list = json.as_array().unwrap().clone();
        list.push(serde_json::json!({ "path": s(&good), "tags": ["pov"] }));
        let p = proposals(&env, serde_json::Value::Array(list));
        let o = run(&env, &["--proposals", &p, "--apply", "--no-rescan"]);
        assert_eq!(o.status.code(), Some(1), "{reason}");
        assert!(stderr(&o).contains(reason), "{reason}: {}", stderr(&o));
        assert!(stderr(&o).contains("nothing written"));
        assert!(!env.user_data.join("video-tags.json").exists(), "{reason}");
    }
}

#[test]
fn stores_the_on_disk_unicode_spelling() {
    let env = env();
    let nfd = "Rene\u{301}e";
    let nfc = "Ren\u{e9}e";
    let dir = env.library.join(nfd);
    fs::create_dir_all(&dir).unwrap();
    video(&dir, "clip.mp4");
    let listed: Vec<String> = fs::read_dir(&env.library)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    if !listed.iter().any(|n| n == nfd) {
        // The filesystem normalised the name on create; the segment matching
        // itself is covered by spelling.rs's unit tests.
        eprintln!("filesystem doesn't keep NFD names; skipping");
        return;
    }
    let proposed = env.library.join(nfc).join("clip.mp4");
    let p = proposals(
        &env,
        serde_json::json!([{ "path": s(&proposed), "tags": ["pov"] }]),
    );
    let o = run(&env, &["--proposals", &p, "--apply", "--no-rescan"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(
        stdout(&o).contains("stored in the on-disk spelling (NFD; proposed NFC)"),
        "{}",
        stdout(&o)
    );
    let on_disk = dir.join("clip.mp4");
    let file = stored(&env);
    let keys: Vec<&String> = file["videos"].as_object().unwrap().keys().collect();
    assert_eq!(keys, vec![s(&on_disk)]);
}

#[test]
fn help_and_bad_flags() {
    let env = env();
    let o = run(&env, &["--help"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(stdout(&o).contains("Exit codes: 0 ok, 1 error"));
    let o = run(&env, &["--bogus"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("unknown argument --bogus"));
}
