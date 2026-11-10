//! End to end through the binary, against temp folders: a userData whose
//! config.json points at two temp library sources with fake scenes.

use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "pornstar-tags-test-{}-{}",
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

struct Env {
    _tmp: TempDir,
    user_data: PathBuf,
    eroscripts: PathBuf,
    faptap: PathBuf,
}

/// A scene folder with a script and the given videos.
fn scene(dir: &Path, videos: &[&str]) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("script.funscript"), b"{}").unwrap();
    for v in videos {
        fs::write(dir.join(v), b"not really a video").unwrap();
    }
}

fn s(p: &Path) -> String {
    p.to_str().unwrap().to_string()
}

fn env() -> Env {
    let tmp = TempDir::new();
    let user_data = tmp.0.join("userData");
    let eroscripts = tmp.0.join("GVP").join("EroScripts");
    let faptap = tmp.0.join("GVP").join("FapTap");
    fs::create_dir_all(&user_data).unwrap();

    // Candy Love: two cuts of one scene, plus a FapTap scene naming her only
    // as a performer → 2 scenes.
    let candy = eroscripts.join("Candy Love").join("Candy Love - One");
    scene(&candy, &["One.mp4", "One 4K.mp4", "._One.mp4"]);
    let gorgeous = faptap.join("Somebody").join("Gorgeous brunette");
    scene(&gorgeous, &["v.mp4"]);
    fs::write(
        gorgeous.join("faptap.json"),
        json!({ "title": "Gorgeous brunette", "performers": [{ "name": "Candy Love" }, { "name": "Mercy" }] })
            .to_string(),
    )
    .unwrap();
    // A video without a script doesn't count.
    let unscripted = eroscripts.join("Candy Love").join("Candy Love - Three");
    fs::create_dir_all(&unscripted).unwrap();
    fs::write(unscripted.join("x.mp4"), b"").unwrap();
    // NoFaceGirl: once by folder name, once only through the topic slug.
    scene(
        &eroscripts.join("NoFaceGirl").join("NoFaceGirl - Edges"),
        &["a.mp4"],
    );
    let slugged = eroscripts.join("scripter").join("Untitled 7");
    scene(&slugged, &["a.mp4"]);
    // Riley Reid: one VRBangers scene → pending.
    scene(
        &eroscripts
            .join("Riley Reid")
            .join("Riley Reid - Anal Virginity (VRBangers)_180x180_3dh"),
        &["v.mp4"],
    );
    // Jane Doe isn't listed: the VRBangers scout's folders name her.
    scene(
        &eroscripts
            .join("Jane Doe")
            .join("Jane Doe - First (VRBangers)_180x180_3dh"),
        &["v.mp4"],
    );
    scene(
        &eroscripts
            .join("Mia Malkova")
            .join("Mia Malkova, Jane Doe - Second (VRBangers)"),
        &["v.mp4"],
    );

    let config = json!({ "settings": {
        "library": {
            "sources": [
                { "id": "1", "name": "EroScripts", "path": eroscripts, "enabled": true },
                { "id": "2", "name": "FapTap", "path": faptap, "enabled": true },
            ],
            "scriptTags": {
                s(&slugged.join("a.mp4")): {
                    "tags": ["pov"],
                    "topicId": 7,
                    "topicUrl": "https://discuss.eroscripts.com/t/nofacegirl-creampied-after-she-cums/7",
                },
            },
        },
        "backend": { "port": 1 },
        "eroscripts": { "session": { "cookie": "SECRET" } },
    } });
    fs::write(user_data.join("config.json"), config.to_string()).unwrap();
    Env {
        _tmp: tmp,
        user_data,
        eroscripts,
        faptap,
    }
}

fn run(env: &Env, args: &[&str]) -> (i32, String) {
    let out: Output = Command::new(env!("CARGO_BIN_EXE_pornstar-tags"))
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
    (out.status.code().unwrap_or(-1), all)
}

fn read_file(env: &Env) -> Value {
    serde_json::from_str(&fs::read_to_string(env.user_data.join("pornstar-tags.json")).unwrap())
        .unwrap()
}

#[test]
fn dry_run_apply_heal_and_daily() {
    let env = env();
    let config_before = fs::read(env.user_data.join("config.json")).unwrap();

    let (code, out) = run(&env, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Dry run: nothing written"), "{out}");
    assert!(!env.user_data.join("pornstar-tags.json").exists());
    assert!(out.contains("Mercy  (1 scene)"), "{out}");

    let (code, out) = run(&env, &["--apply", "--no-rescan"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("No rescan requested"), "{out}");
    let file = read_file(&env);
    assert_eq!(file["version"], 1);
    assert_eq!(file["minScenes"], 2);
    let ps = &file["pornstars"];
    assert_eq!(
        ps["candy-love"],
        json!({ "name": "Candy Love", "scenes": 2, "applied": true })
    );
    assert_eq!(ps["nofacegirl"]["scenes"], 2);
    assert_eq!(
        ps["jane-doe"],
        json!({ "name": "Jane Doe", "scenes": 2, "applied": true })
    );
    assert_eq!(
        ps["riley-reid"],
        json!({ "name": "Riley Reid", "scenes": 1, "applied": false })
    );
    assert_eq!(ps["mia-malkova"]["applied"], false);
    assert_eq!(ps["eva-elfie"]["scenes"], 0);
    assert!(ps.get("mercy").is_none());

    let candy = env.eroscripts.join("Candy Love").join("Candy Love - One");
    let mut expected = serde_json::Map::new();
    for (path, tags) in [
        (candy.join("One 4K.mp4"), json!(["candy-love"])),
        (candy.join("One.mp4"), json!(["candy-love"])),
        (
            env.faptap
                .join("Somebody")
                .join("Gorgeous brunette")
                .join("v.mp4"),
            json!(["candy-love"]),
        ),
        (
            env.eroscripts
                .join("NoFaceGirl")
                .join("NoFaceGirl - Edges")
                .join("a.mp4"),
            json!(["nofacegirl"]),
        ),
        (
            env.eroscripts
                .join("scripter")
                .join("Untitled 7")
                .join("a.mp4"),
            json!(["nofacegirl"]),
        ),
        (
            env.eroscripts
                .join("Jane Doe")
                .join("Jane Doe - First (VRBangers)_180x180_3dh")
                .join("v.mp4"),
            json!(["jane-doe"]),
        ),
        (
            env.eroscripts
                .join("Mia Malkova")
                .join("Mia Malkova, Jane Doe - Second (VRBangers)")
                .join("v.mp4"),
            json!(["jane-doe"]),
        ),
    ] {
        expected.insert(s(&path), tags);
    }
    assert_eq!(file["videos"], Value::Object(expected));

    // Nothing changed: nothing written, no backup.
    let (code, out) = run(&env, &["--apply", "--no-rescan"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Nothing to change"), "{out}");
    assert!(!env.user_data.join("pornstar-tags.json.bak").exists());

    // A second Riley Reid scene: her tag applies to both; the previous file
    // is kept. Renaming a scene heals on the run.
    scene(
        &env.eroscripts
            .join("Mia Malkova")
            .join("Mia Malkova, Riley Reid - One Last Bang (VRBangers)_180x180_3dh"),
        &["v.mp4"],
    );
    fs::rename(&candy, candy.with_file_name("Candy Love - One renamed")).unwrap();
    let (code, out) = run(&env, &["--apply", "--no-rescan"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("+ riley-reid"), "{out}");
    assert!(out.contains("- candy-love"), "{out}");
    assert!(out.contains("(was pending)"), "{out}");
    assert!(env.user_data.join("pornstar-tags.json.bak").exists());
    let file = read_file(&env);
    assert_eq!(file["pornstars"]["riley-reid"]["applied"], true);
    assert_eq!(file["pornstars"]["mia-malkova"]["scenes"], 2);
    assert_eq!(
        file["videos"][s(&candy
            .with_file_name("Candy Love - One renamed")
            .join("One.mp4"))],
        json!(["candy-love"])
    );
    assert!(file["videos"].get(s(&candy.join("One.mp4"))).is_none());

    // --daily: already ran today.
    let (code, out) = run(&env, &["--daily", "--no-rescan"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Already ran today"), "{out}");
    // No stamp and an old file: it runs (and finds nothing to change).
    fs::remove_file(env.user_data.join("pornstar-tags.last-run")).unwrap();
    let mut old = read_file(&env);
    old["updatedAt"] = json!("2001-01-01T00:00:00.000Z");
    fs::write(
        env.user_data.join("pornstar-tags.json"),
        serde_json::to_string_pretty(&old).unwrap(),
    )
    .unwrap();
    let (code, out) = run(&env, &["--daily", "--no-rescan"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Nothing to change"), "{out}");
    assert!(env.user_data.join("pornstar-tags.last-run").exists());
    let (_, out) = run(&env, &["--daily"]);
    assert!(out.contains("Already ran today"), "{out}");

    let (code, out) = run(&env, &["--list"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("applied  candy-love"), "{out}");
    assert!(out.contains("pending  eva-elfie"), "{out}");
    assert!(out.contains("Videos (9):"), "{out}");

    // config.json is never written, and the cookie never printed.
    assert_eq!(
        fs::read(env.user_data.join("config.json")).unwrap(),
        config_before
    );
    assert!(!out.contains("SECRET"));
}

#[test]
fn help_errors_and_missing_sources() {
    let env = env();
    let (code, out) = run(&env, &["--help"]);
    assert_eq!(code, 0);
    assert!(out.contains("Usage:"));
    let (code, out) = run(&env, &["--bogus"]);
    assert_eq!(code, 1, "{out}");
    let (code, out) = run(&env, &["--list"]);
    assert_eq!(code, 0);
    assert!(out.contains("No pornstar tags yet"), "{out}");

    // An unreadable source: an error, nothing written.
    fs::rename(&env.faptap, env.faptap.with_file_name("FapTap-gone")).unwrap();
    let (code, out) = run(&env, &["--apply", "--no-rescan"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("can't be read"), "{out}");
    assert!(!env.user_data.join("pornstar-tags.json").exists());
}
