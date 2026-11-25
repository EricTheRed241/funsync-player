//! The library's `config.json` in the shared userData folder: seeded from
//! it (`import`), kept in step with it (every save writes the parts this
//! server owns back, `write_through`; a `config.json` changed while the
//! server was down, e.g. by a CLI, is read again at start, `newer_config`),
//! so the CLIs and scouts keep reading the library state from there.

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::data::{Library, load_library, now_ms, save_library, write_atomic};

/// The `settings` sections copied. Left out: `eroscripts` (the forum
/// session), `security`, `handy` (connection keys) and window state.
const SETTINGS: [&str; 6] = [
    "library",
    "player",
    "tcode",
    "buttplug",
    "vr",
    "knownDevices",
];
const TOP: [&str; 3] = ["playlists", "categories", "videoCategories"];

#[derive(Debug)]
pub struct ImportSummary {
    pub sources: usize,
    pub playlists: usize,
    pub categories: usize,
    pub ratings: usize,
}

/// Reads `<old>/config.json` (only reads it) and writes `<data>/library.json`.
/// Refuses to replace an earlier import unless `force`.
pub fn import(old_user_data: &Path, data_dir: &Path, force: bool) -> Result<ImportSummary, String> {
    let existing = load_library(data_dir)?;
    if existing.imported_at.is_some() && !force {
        return Err(format!(
            "{} already holds an import (from {}); --force replaces it",
            data_dir.display(),
            existing.imported_from.as_deref().unwrap_or("?")
        ));
    }
    let cfg_path = old_user_data.join("config.json");
    let config = pick_config(&read_json(&cfg_path)?);
    let lib = Library {
        version: 1,
        imported_from: Some(cfg_path.to_string_lossy().into_owned()),
        imported_at: Some(now_ms()),
        config,
    };
    save_library(data_dir, &lib)?;
    // The old backend's frame check results (same file and shape as
    // crate::stereo's), so a fresh import doesn't look at every video again.
    // Entries are keyed by size + mtime, so stale ones are redone anyway.
    let stereo = old_user_data.join("vr-stereo-cache.json");
    if stereo.is_file() && !data_dir.join("vr-stereo-cache.json").exists() {
        std::fs::copy(&stereo, data_dir.join("vr-stereo-cache.json")).map_err(|e| e.to_string())?;
    }
    let count = |v: &Value| match v {
        Value::Array(a) => a.len(),
        Value::Object(o) => o.len(),
        _ => 0,
    };
    Ok(ImportSummary {
        sources: lib.sources().len(),
        playlists: count(&lib.config["playlists"]),
        categories: count(&lib.config["categories"]),
        ratings: count(&lib.lib()["ratings"]),
    })
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The parts of a `config.json` this server keeps.
pub fn pick_config(cfg: &Value) -> Value {
    let pick = |from: &Value, keys: &[&str]| -> Value {
        Value::Object(
            keys.iter()
                .filter_map(|k| from.get(*k).map(|v| (k.to_string(), v.clone())))
                .collect::<Map<_, _>>(),
        )
    };
    let mut config = pick(cfg, &TOP);
    config["settings"] = pick(&cfg["settings"], &SETTINGS);
    config
}

/// Write this server's parts of `config` into `cfg_path`, keeping every other
/// key there (the forum session, …), with `settings.backend.port` = `port` so
/// the CLIs reach this server.
pub fn write_through(config: &Value, cfg_path: &Path, port: u16) -> Result<(), String> {
    let mut out = read_json(cfg_path).unwrap_or_else(|_| json!({}));
    if !out.is_object() {
        out = json!({});
    }
    for k in TOP {
        if let Some(v) = config.get(k) {
            out[k] = v.clone();
        }
    }
    if !out["settings"].is_object() {
        out["settings"] = json!({});
    }
    for k in SETTINGS {
        if let Some(v) = config["settings"].get(k) {
            out["settings"][k] = v.clone();
        }
    }
    if !out["settings"]["backend"].is_object() {
        out["settings"]["backend"] = json!({});
    }
    out["settings"]["backend"]["port"] = json!(port);
    let bytes = serde_json::to_vec_pretty(&out).map_err(|e| e.to_string())?;
    write_atomic(cfg_path, &bytes).map_err(|e| format!("{}: {e}", cfg_path.display()))
}

/// `config.json`'s parts when it changed after `library.json` was last
/// written (someone else edited it while this server was down).
pub fn newer_config(lib: &Library, data_dir: &Path) -> Option<Value> {
    let cfg_path = Path::new(lib.imported_from.as_deref()?);
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    (mtime(cfg_path)? > mtime(&data_dir.join(crate::data::LIBRARY_FILE))?)
        .then(|| read_json(cfg_path).ok().map(|c| pick_config(&c)))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_without_touching_the_old_folder_and_refuses_twice() {
        let old = tempfile::tempdir().unwrap();
        let new = tempfile::tempdir().unwrap();
        let cfg = r#"{"settings":{"library":{"sources":[{"id":"a","name":"Ero","path":"/x","enabled":false}],
            "ratings":{"/x/a.mp4":{"rating":4}}},"eroscripts":{"session":{"cookie":"secret"}},
            "tcode":{"autoConnect":true,"port":"/dev/tty.x"}},
            "playlists":[{"name":"P","videoPaths":[]}],"categories":[]}"#;
        std::fs::write(old.path().join("config.json"), cfg).unwrap();
        std::fs::write(
            old.path().join("vr-stereo-cache.json"),
            r#"{"version":1,"entries":{}}"#,
        )
        .unwrap();
        let before = std::fs::read_dir(old.path()).unwrap().count();
        let s = import(old.path(), new.path(), false).unwrap();
        assert_eq!((s.sources, s.playlists, s.ratings), (1, 1, 1));
        assert_eq!(std::fs::read_dir(old.path()).unwrap().count(), before);
        let lib = load_library(new.path()).unwrap();
        assert!(!lib.sources()[0].enabled);
        assert_eq!(lib.config["settings"]["tcode"]["autoConnect"], true);
        assert_eq!(lib.config["settings"]["tcode"]["port"], "/dev/tty.x");
        let written = std::fs::read_to_string(new.path().join("library.json")).unwrap();
        assert!(
            !written.contains("secret"),
            "the forum session must not be copied"
        );
        assert!(import(old.path(), new.path(), false).is_err());
        assert!(import(old.path(), new.path(), true).is_ok());
        assert!(new.path().join("vr-stereo-cache.json").is_file());
    }

    #[test]
    fn write_through_keeps_other_keys_and_sets_the_port() {
        let old = tempfile::tempdir().unwrap();
        let cfg = old.path().join("config.json");
        std::fs::write(
            &cfg,
            r#"{"settings":{"eroscripts":{"session":"s"},"library":{"ratings":{}}},"x":1}"#,
        )
        .unwrap();
        let config = json!({"playlists": [{"name": "P"}], "settings": {"library": {"ratings": {"/a": {"rating": 5}}}}});
        write_through(&config, &cfg, 5300).unwrap();
        let out: Value = serde_json::from_slice(&std::fs::read(&cfg).unwrap()).unwrap();
        assert_eq!(out["settings"]["eroscripts"]["session"], "s");
        assert_eq!(out["x"], 1);
        assert_eq!(out["playlists"][0]["name"], "P");
        assert_eq!(out["settings"]["library"]["ratings"]["/a"]["rating"], 5);
        assert_eq!(out["settings"]["backend"]["port"], 5300);
        assert_eq!(pick_config(&out)["settings"].get("eroscripts"), None);
    }
}
