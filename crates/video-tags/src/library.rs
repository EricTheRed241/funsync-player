//! How many videos the library holds, counted the way every scout checks it
//! against `libraryMaxVideos` (scripts/discovery-io.mjs libraryVideoCount):
//! the enabled sources of `config.json`, up to four folder levels down, dot
//! names (AppleDouble `._*` included) skipped. Shared by the scouts and the
//! Downloads page, so the page's "library full" is the scouts' own stop.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// Videos the scouts count (`isVideoName` in scripts/eroscripts-scout-lib.mjs).
pub const VIDEO_EXTS: &[&str] = &["mp4", "m4v", "mkv", "webm", "mov"];

pub fn is_video_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTS.iter().any(|v| e.eq_ignore_ascii_case(v)))
}

/// The enabled library sources of `config.json` (`settings.library.sources`;
/// a source counts unless `enabled` is false).
pub fn enabled_sources(config: &Value) -> Vec<PathBuf> {
    let settings = config.get("settings").unwrap_or(config);
    settings
        .pointer("/library/sources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|s| s.get("enabled").and_then(Value::as_bool) != Some(false))
        .filter_map(|s| s.get("path")?.as_str().filter(|p| !p.is_empty()))
        .map(PathBuf::from)
        .collect()
}

/// Videos under `dirs`, up to four levels down, dot names skipped.
pub fn video_count<'a>(dirs: impl IntoIterator<Item = &'a Path>) -> usize {
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
    for d in dirs {
        walk(d, 0, &mut n);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_like_the_scouts() {
        let root = std::env::temp_dir().join(format!("video-tags-library-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let on = root.join("On");
        fs::create_dir_all(on.join("A/B/C/D")).unwrap();
        fs::create_dir_all(on.join(".hidden")).unwrap();
        fs::create_dir_all(root.join("Off")).unwrap();
        for f in [
            "x.mp4",
            "._x.mp4",
            "x.funscript",
            "A/B/C/y.MKV",
            "A/B/C/D/too-deep.mp4",
            ".hidden/z.mp4",
        ] {
            fs::write(on.join(f), "").unwrap();
        }
        fs::write(root.join("Off/o.mp4"), "").unwrap();
        let config = json!({"settings": {"library": {"sources": [
            {"path": on}, {"path": root.join("Off"), "enabled": false}, {"path": ""}]}}});
        let sources = enabled_sources(&config);
        assert_eq!(sources, std::slice::from_ref(&on));
        assert_eq!(video_count(sources.iter().map(PathBuf::as_path)), 2);
        let _ = fs::remove_dir_all(&root);
    }
}
