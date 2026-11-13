//! Which judged posts are in the library now: library videos per EroScripts
//! topic, from the scouts' `eroscripts.json` sidecars in the library sources,
//! the app's `library.scriptTags` (posts linked by `eroscripts:sync` or
//! downloaded in the app) and `eroscripts-links.json`. Only videos that still
//! exist count, so a deleted scene no longer marks its post.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const LINKS_FILE: &str = "eroscripts-links.json";
const SIDECAR: &str = "eroscripts.json";
/// `<source>/<artist>/<scene>/eroscripts.json` is depth 2; a little slack for odd layouts.
const MAX_DEPTH: usize = 4;

/// Library videos per topic id. `problems` gets sources that can't be read
/// (an unplugged drive), since their posts then show as not in the library.
pub fn topic_videos(
    config: &Value,
    links: &Value,
    problems: &mut Vec<String>,
) -> HashMap<u64, usize> {
    let settings = config.get("settings").unwrap_or(config);
    let mut pairs: Vec<(u64, PathBuf)> = Vec::new();
    for source in settings
        .pointer("/library/sources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| s.get("path")?.as_str())
        .filter(|p| !p.is_empty())
    {
        let root = Path::new(source);
        if !root.is_dir() {
            problems.push(format!("library source not found: {source}"));
            continue;
        }
        walk(root, 0, &mut pairs);
    }
    let by_path = |v: Option<&Value>| -> Vec<(u64, PathBuf)> {
        v.and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(path, rec)| Some((topic_of(rec.get("topicId")?)?, PathBuf::from(path))))
            .collect()
    };
    pairs.extend(by_path(settings.pointer("/library/scriptTags")));
    pairs.extend(by_path(links.get("links")));
    count_existing(pairs, |p| p.is_file())
}

/// One count per distinct (topic, video) whose video `exists`.
pub fn count_existing(
    pairs: impl IntoIterator<Item = (u64, PathBuf)>,
    exists: impl Fn(&Path) -> bool,
) -> HashMap<u64, usize> {
    let mut seen = HashSet::new();
    let mut counts = HashMap::new();
    for (topic, path) in pairs {
        if seen.contains(&(topic, path.clone())) || !exists(&path) {
            continue;
        }
        *counts.entry(topic).or_insert(0) += 1;
        seen.insert((topic, path));
    }
    counts
}

fn topic_of(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Sidecars under `dir`: (topic, the scene's video). Dot files, AppleDouble
/// sidecars and the scouts' `_staging` are skipped.
fn walk(dir: &Path, depth: usize, out: &mut Vec<(u64, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') || name == "_staging" {
            continue;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if depth < MAX_DEPTH {
                walk(&path, depth + 1, out);
            }
        } else if name == SIDECAR
            && let Some(pair) = sidecar_video(&path)
        {
            out.push(pair);
        }
    }
}

/// A sidecar's topic and video: its `file`, else the first video beside it.
fn sidecar_video(sidecar: &Path) -> Option<(u64, PathBuf)> {
    let rec: Value = serde_json::from_slice(&fs::read(sidecar).ok()?).ok()?;
    let topic = topic_of(rec.get("topicId")?)?;
    let dir = sidecar.parent()?;
    let file = match rec.get("file").and_then(Value::as_str) {
        Some(f) if !f.is_empty() => f.to_string(),
        _ => fs::read_dir(dir)
            .ok()?
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .find(|n| !n.starts_with("._") && is_video(n))?,
    };
    Some((topic, dir.join(file)))
}

fn is_video(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".mp4", ".mkv", ".webm", ".mov", ".avi", ".m4v", ".wmv"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("scout-report-lib-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn counts_each_existing_video_once_per_topic() {
        let pairs = vec![
            (1, PathBuf::from("/a.mp4")),
            (1, PathBuf::from("/a.mp4")),
            (1, PathBuf::from("/b.mp4")),
            (2, PathBuf::from("/gone.mp4")),
        ];
        let counts = count_existing(pairs, |p| p != Path::new("/gone.mp4"));
        assert_eq!(counts.get(&1), Some(&2));
        assert_eq!(counts.get(&2), None);
    }

    #[test]
    fn reads_sidecars_script_tags_and_links_but_not_deleted_videos() {
        let root = temp_dir("sources");
        let scene = root.join("Artist/Artist - Scene");
        fs::create_dir_all(&scene).unwrap();
        fs::write(scene.join("Artist - Scene.mp4"), b"v").unwrap();
        fs::write(
            scene.join("eroscripts.json"),
            json!({"topicId": 10}).to_string(),
        )
        .unwrap();
        // A sidecar whose video was deleted, one in staging and an AppleDouble one: none count.
        let empty = root.join("Artist/Deleted");
        fs::create_dir_all(&empty).unwrap();
        fs::write(
            empty.join("eroscripts.json"),
            json!({"topicId": 11, "file": "x.mp4"}).to_string(),
        )
        .unwrap();
        let staging = root.join("_staging/S");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("s.mp4"), b"v").unwrap();
        fs::write(
            staging.join("eroscripts.json"),
            json!({"topicId": 12}).to_string(),
        )
        .unwrap();
        fs::write(scene.join("._eroscripts.json"), b"\0").unwrap();
        let linked = root.join("Other/linked.mp4");
        fs::create_dir_all(linked.parent().unwrap()).unwrap();
        fs::write(&linked, b"v").unwrap();
        let config = json!({"settings": {"library": {
            "sources": [{"path": root.to_str().unwrap()}, {"path": "/no/such/source"}],
            "scriptTags": {linked.to_str().unwrap(): {"topicId": 20}, "/gone.mp4": {"topicId": 21}}
        }}});
        let links = json!({"links": {linked.to_str().unwrap(): {"topicId": "30"}}});
        let mut problems = Vec::new();
        let counts = topic_videos(&config, &links, &mut problems);
        assert_eq!(counts.get(&10), Some(&1));
        assert_eq!(counts.get(&20), Some(&1));
        assert_eq!(counts.get(&30), Some(&1));
        for missing in [11, 12, 21] {
            assert_eq!(counts.get(&missing), None, "{missing}");
        }
        assert_eq!(
            problems,
            vec!["library source not found: /no/such/source".to_string()]
        );
        fs::remove_dir_all(&root).unwrap();
    }
}
