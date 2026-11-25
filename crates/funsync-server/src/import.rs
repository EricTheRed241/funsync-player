//! Seed the Rust app's library from the old app's `config.json`, read-only.

use std::path::Path;

use serde_json::{Map, Value};

use crate::data::{Library, load_library, now_ms, save_library};

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
    let text = std::fs::read(&cfg_path).map_err(|e| format!("{}: {e}", cfg_path.display()))?;
    let cfg: Value =
        serde_json::from_slice(&text).map_err(|e| format!("{}: {e}", cfg_path.display()))?;
    let pick = |from: &Value, keys: &[&str]| -> Value {
        Value::Object(
            keys.iter()
                .filter_map(|k| from.get(*k).map(|v| (k.to_string(), v.clone())))
                .collect::<Map<_, _>>(),
        )
    };
    let mut config = pick(&cfg, &TOP);
    config["settings"] = pick(&cfg["settings"], &SETTINGS);
    let lib = Library {
        version: 1,
        imported_from: Some(cfg_path.to_string_lossy().into_owned()),
        imported_at: Some(now_ms()),
        config,
    };
    save_library(data_dir, &lib)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_without_touching_the_old_folder_and_refuses_twice() {
        let old = tempfile::tempdir().unwrap();
        let new = tempfile::tempdir().unwrap();
        let cfg = r#"{"settings":{"library":{"sources":[{"id":"a","name":"Ero","path":"/x","enabled":false}],
            "ratings":{"/x/a.mp4":{"rating":4}}},"eroscripts":{"session":{"cookie":"secret"}}},
            "playlists":[{"name":"P","videoPaths":[]}],"categories":[]}"#;
        std::fs::write(old.path().join("config.json"), cfg).unwrap();
        let before = std::fs::read_dir(old.path()).unwrap().count();
        let s = import(old.path(), new.path(), false).unwrap();
        assert_eq!((s.sources, s.playlists, s.ratings), (1, 1, 1));
        assert_eq!(std::fs::read_dir(old.path()).unwrap().count(), before);
        let lib = load_library(new.path()).unwrap();
        assert!(!lib.sources()[0].enabled);
        let written = std::fs::read_to_string(new.path().join("library.json")).unwrap();
        assert!(
            !written.contains("secret"),
            "the forum session must not be copied"
        );
        assert!(import(old.path(), new.path(), false).is_err());
        assert!(import(old.path(), new.path(), true).is_ok());
    }
}
