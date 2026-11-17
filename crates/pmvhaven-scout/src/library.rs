//! Which PMVHaven videos the library already has (the deck hides them,
//! `--fetch` skips them), cheaply: no video is opened.
//!
//! - by id: a `pmvhaven.json` sidecar (this scout's downloads) or an
//!   `eroscripts.json` sidecar's `pmvhaven.id` (EroScripts posts that link a
//!   PMVHaven video), found walking the enabled sources four levels deep;
//! - by title and length: a library video whose file name holds the title's
//!   words (whole words, folded) and whose length in the app's
//!   `library.durationCache` (seconds by path) is within 1.5 s.

use serde_json::Value;
use slr_scout::library::{self, Source};
use slr_scout::text;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub const SIDECAR: &str = "pmvhaven.json";
pub const EROSCRIPTS_SIDECAR: &str = "eroscripts.json";
/// How far a library video's length may be from PMVHaven's.
pub const LENGTH_SLACK_S: f64 = 1.5;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Known {
    pub ids: HashSet<String>,
    /// Folded file stem and length in seconds of each video with a known length.
    pub lengths: Vec<(String, f64)>,
}

impl Known {
    pub fn has(&self, id: &str, title: &str, duration_s: Option<u64>) -> bool {
        if self.ids.contains(id) {
            return true;
        }
        let (Some(d), title) = (duration_s, text::normalise(title)) else {
            return false;
        };
        !title.is_empty()
            && self.lengths.iter().any(|(stem, len)| {
                (len - d as f64).abs() <= LENGTH_SLACK_S && text::contains_words(stem, &title)
            })
    }
}

/// The ids of PMVHaven sidecars under the enabled sources.
pub fn sidecar_ids(sources: &[Source]) -> HashSet<String> {
    fn walk(dir: &Path, depth: u32, out: &mut HashSet<String>) {
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
                    walk(&e.path(), depth + 1, out);
                }
                continue;
            }
            let pointer = match name.as_str() {
                SIDECAR => "/id",
                EROSCRIPTS_SIDECAR => "/pmvhaven/id",
                _ => continue,
            };
            let id = fs::read_to_string(e.path())
                .ok()
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                .and_then(|v| {
                    v.pointer(pointer)
                        .and_then(Value::as_str)
                        .map(str::to_string)
                });
            if let Some(id) = id {
                out.insert(id);
            }
        }
    }
    let mut out = HashSet::new();
    for s in library::enabled(sources) {
        walk(&s.path, 0, &mut out);
    }
    out
}

/// `library.durationCache` as folded file stems and lengths.
pub fn lengths(config: &Value) -> Vec<(String, f64)> {
    let settings = config.get("settings").unwrap_or(config);
    settings
        .pointer("/library/durationCache")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(path, secs)| {
            let stem = Path::new(path).file_stem()?.to_str()?;
            Some((text::normalise(stem), secs.as_f64()?))
        })
        .collect()
}

pub fn known(config: &Value, sources: &[Source]) -> Known {
    Known {
        ids: sidecar_ids(sources),
        lengths: lengths(config),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_videos_by_sidecar() {
        let root = std::env::temp_dir().join(format!("pmvhaven-library-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let other = root.join("Other/Cowgirlfan101/Cowgirlfan101 - X (PMVHaven)");
        let es = root.join("EroScripts/Artist/Scene");
        fs::create_dir_all(&other).unwrap();
        fs::create_dir_all(&es).unwrap();
        fs::write(other.join(SIDECAR), r#"{"site": "pmvhaven", "id": "aaa"}"#).unwrap();
        fs::write(other.join("._pmvhaven.json"), "junk").unwrap();
        fs::write(
            es.join(EROSCRIPTS_SIDECAR),
            r#"{"topicId": 1, "pmvhaven": {"id": "bbb", "url": "https://pmvhaven.com/video/x_bbb"}}"#,
        )
        .unwrap();
        let src = |name: &str, enabled| Source {
            name: Some(name.into()),
            path: root.join(name),
            enabled,
        };
        let ids = sidecar_ids(&[src("Other", true), src("EroScripts", true)]);
        assert_eq!(ids, HashSet::from(["aaa".to_string(), "bbb".to_string()]));
        assert!(sidecar_ids(&[src("Other", false)]).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn finds_videos_by_title_and_length() {
        let config = json!({"settings": {"library": {"durationCache": {
            "/V/Other/Vassago/Vassago - Shrooms (PMVHaven)/Vassago - Shrooms (PMVHaven).mp4": 293.4,
            "/V/Other/x/Neurotica remix.mp4": 100.0
        }}}});
        let known = Known {
            ids: HashSet::from(["id1".to_string()]),
            lengths: lengths(&config),
        };
        assert!(known.has("id1", "anything", None));
        assert!(known.has("z", "Shrooms", Some(294)));
        assert!(known.has("z", "shrooms!", Some(292)));
        assert!(!known.has("z", "Shrooms", Some(296)));
        assert!(!known.has("z", "Shroom", Some(293)));
        assert!(!known.has("z", "Shrooms", None));
        assert!(!known.has("z", "!!", Some(100)));
    }
}
