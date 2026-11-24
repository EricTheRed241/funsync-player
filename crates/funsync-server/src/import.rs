//! Seed the Rust app's library from the old app's `config.json`, read-only.

use std::path::Path;

use serde_json::Value;

use crate::data::{Library, SourceEntry, load_library, now_ms, save_library};

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
    let lib_settings = cfg
        .pointer("/settings/library")
        .cloned()
        .unwrap_or(Value::Null);
    let sources: Vec<SourceEntry> = lib_settings
        .get("sources")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| format!("settings.library.sources: {e}"))?
        .unwrap_or_default();
    let count = |v: Option<&Value>| match v {
        Some(Value::Array(a)) => a.len(),
        Some(Value::Object(o)) => o.len(),
        _ => 0,
    };
    let lib = Library {
        version: 1,
        imported_from: Some(cfg_path.to_string_lossy().into_owned()),
        imported_at: Some(now_ms()),
        sources,
        playlists: cfg
            .get("playlists")
            .cloned()
            .unwrap_or(Value::Array(vec![])),
        categories: cfg
            .get("categories")
            .cloned()
            .unwrap_or(Value::Array(vec![])),
        video_categories: cfg.get("videoCategories").cloned().unwrap_or(Value::Null),
        old_library: lib_settings,
    };
    save_library(data_dir, &lib)?;
    Ok(ImportSummary {
        sources: lib.sources.len(),
        playlists: count(Some(&lib.playlists)),
        categories: count(Some(&lib.categories)),
        ratings: count(lib.old_library.get("ratings")),
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
        assert!(!lib.sources[0].enabled);
        let written = std::fs::read_to_string(new.path().join("library.json")).unwrap();
        assert!(
            !written.contains("secret"),
            "the forum session must not be copied"
        );
        assert!(import(old.path(), new.path(), false).is_err());
        assert!(import(old.path(), new.path(), true).is_ok());
    }
}
