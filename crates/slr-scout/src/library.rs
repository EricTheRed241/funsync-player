//! The user's side of the preferences and the library, read only from
//! `config.json` and the library folders: tag score changes
//! (`library.tagScores`), favourite artists (FAVOURITES with
//! `library.favouriteArtists` on top) and the favourite artist bonus
//! (`library.favouriteArtistScore`), the library sources, their artist
//! folders and how many videos they hold (LIBRARY_MAX_VIDEOS is checked
//! against it, counted like the scouts: scripts/discovery-io.mjs
//! libraryVideoCount).

use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

/// Videos the scouts count (`isVideoName` in scripts/eroscripts-scout-lib.mjs).
pub const VIDEO_EXTS: &[&str] = &["mp4", "m4v", "mkv", "webm", "mov"];

#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub name: Option<String>,
    pub path: PathBuf,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UserConfig {
    /// `library.tagScores`, raw (normalised by the scorer).
    pub tag_scores: BTreeMap<String, Value>,
    /// `library.favouriteArtists`: `true` adds, `false` removes.
    pub favourite_changes: Vec<(String, bool)>,
    /// `library.favouriteArtistScore`, raw.
    pub favourite_score: Value,
    pub sources: Vec<Source>,
}

/// Settings live under `settings` (electron-conf); a bare settings object is
/// accepted too.
pub fn from_json(config: &Value) -> UserConfig {
    let settings = config.get("settings").unwrap_or(config);
    let library = settings.get("library").unwrap_or(&Value::Null);
    let tag_scores = library
        .get("tagScores")
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    let favourite_changes = library
        .get("favouriteArtists")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_bool()?)))
                .collect()
        })
        .unwrap_or_default();
    let sources = library
        .get("sources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let path = s.get("path")?.as_str().filter(|p| !p.is_empty())?;
            Some(Source {
                name: s.get("name").and_then(Value::as_str).map(str::to_string),
                path: PathBuf::from(path),
                enabled: s.get("enabled").and_then(Value::as_bool) != Some(false),
            })
        })
        .collect();
    UserConfig {
        tag_scores,
        favourite_changes,
        favourite_score: library
            .get("favouriteArtistScore")
            .cloned()
            .unwrap_or(Value::Null),
        sources,
    }
}

/// How names compare as artists: NFC, trimmed, lower case
/// (renderer/js/favourite-artists.js; ARTIST_ALIASES aren't shared with
/// Rust, so an alias counts under its own spelling).
pub fn fold(name: &str) -> String {
    name.trim().nfc().collect::<String>().to_lowercase()
}

/// The favourite artists: `builtin` (FAVOURITES) not set false in
/// `changes`, plus the names set true (effectiveFavourites).
pub fn effective_favourites(builtin: &[String], changes: &[(String, bool)]) -> Vec<String> {
    let mut out: Vec<(String, String)> = builtin
        .iter()
        .map(|n| (fold(n), n.trim().to_string()))
        .collect();
    for (name, favourite) in changes {
        let key = fold(name);
        if key.is_empty() {
            continue;
        }
        if !favourite {
            out.retain(|(k, _)| *k != key);
        } else if !out.iter().any(|(k, _)| *k == key) {
            out.push((key, name.trim().to_string()));
        }
    }
    out.into_iter().map(|(_, n)| n).collect()
}

/// The enabled sources.
pub fn enabled(sources: &[Source]) -> impl Iterator<Item = &Source> {
    sources.iter().filter(|s| s.enabled)
}

/// Artist folders (each enabled source's top-level folders), folded.
pub fn creators(sources: &[Source]) -> HashSet<String> {
    let mut out = HashSet::new();
    for s in enabled(sources) {
        let Ok(entries) = fs::read_dir(&s.path) else {
            continue;
        };
        for e in entries.flatten() {
            let Ok(name) = e.file_name().into_string() else {
                continue;
            };
            if !name.starts_with('.') && e.path().is_dir() {
                out.insert(fold(&name));
            }
        }
    }
    out
}

pub fn is_video_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTS.iter().any(|v| e.eq_ignore_ascii_case(v)))
}

/// Videos in the enabled sources, up to four levels down, dot names
/// skipped (AppleDouble `._*` included).
pub fn video_count(sources: &[Source]) -> usize {
    fn walk(dir: &Path, depth: u32, n: &mut usize) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let Ok(name) = e.file_name().into_string() else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = e.file_type() else { continue };
            if kind.is_dir() {
                if depth < 3 {
                    walk(&e.path(), depth + 1, n);
                }
            } else if is_video_name(&name) {
                *n += 1;
            }
        }
    }
    let mut n = 0;
    for s in enabled(sources) {
        walk(&s.path, 0, &mut n);
    }
    n
}

/// Where SLR scenes go: the library source called `name`, else `<name>`
/// beside the first enabled source (scripts/discovery-io.mjs downloadFolder).
/// The bool says whether it is a library source.
pub fn download_folder(sources: &[Source], name: &str) -> Option<(PathBuf, bool)> {
    if let Some(own) = sources.iter().find(|s| s.name.as_deref() == Some(name)) {
        return Some((own.path.clone(), true));
    }
    let first = enabled(sources).next()?;
    Some((first.path.parent()?.join(name), false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("slr-scout-library-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_the_user_side() {
        let c = from_json(&json!({"settings": {"library": {
            "tagScores": {"pov": 2.5, "anal": -1},
            "favouriteArtists": {"Miss Lexa": true, "JJULEZ": false, "x": "junk"},
            "favouriteArtistScore": 6,
            "sources": [{"name": "Other", "path": "/V/Other", "enabled": true},
                        {"name": "Off", "path": "/V/Off", "enabled": false}, {"path": ""}]}}}));
        assert_eq!(c.tag_scores["pov"], json!(2.5));
        assert_eq!(
            c.favourite_changes,
            [
                ("JJULEZ".to_string(), false),
                ("Miss Lexa".to_string(), true)
            ]
        );
        assert_eq!(c.favourite_score, json!(6));
        assert_eq!(c.sources.len(), 2);
        assert!(c.sources[0].enabled && !c.sources[1].enabled);
        assert_eq!(from_json(&json!({})), UserConfig::default());
    }

    #[test]
    fn applies_favourite_changes() {
        let builtin = vec!["Candy Love".to_string(), "JJULEZ".to_string()];
        let changes = vec![
            ("jjulez".to_string(), false),
            ("Miss Lexa".to_string(), true),
            ("candy love".to_string(), true),
        ];
        assert_eq!(
            effective_favourites(&builtin, &changes),
            ["Candy Love", "Miss Lexa"]
        );
    }

    #[test]
    fn walks_the_library() {
        let root = temp("walk");
        let other = root.join("Other");
        fs::create_dir_all(other.join("Miss Lexa/Scene A")).unwrap();
        fs::create_dir_all(other.join(".hidden")).unwrap();
        fs::create_dir_all(root.join("Off/Artist")).unwrap();
        fs::write(other.join("Miss Lexa/Scene A/a.mp4"), "").unwrap();
        fs::write(other.join("Miss Lexa/Scene A/._a.mp4"), "").unwrap();
        fs::write(other.join("Miss Lexa/Scene A/a.funscript"), "").unwrap();
        fs::write(other.join("loose.MKV"), "").unwrap();
        fs::write(root.join("Off/Artist/b.mp4"), "").unwrap();
        let sources = vec![
            Source {
                name: Some("Other".into()),
                path: other.clone(),
                enabled: true,
            },
            Source {
                name: Some("Off".into()),
                path: root.join("Off"),
                enabled: false,
            },
        ];
        assert_eq!(video_count(&sources), 2);
        assert_eq!(creators(&sources), HashSet::from(["miss lexa".to_string()]));
        assert_eq!(
            download_folder(&sources, "Other"),
            Some((other.clone(), true))
        );
        assert_eq!(
            download_folder(&sources[..1], "SLR"),
            Some((root.join("SLR"), false))
        );
        assert_eq!(download_folder(&[], "Other"), None);
        let _ = fs::remove_dir_all(&root);
    }
}
