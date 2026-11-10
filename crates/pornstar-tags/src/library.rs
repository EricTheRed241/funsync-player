//! Read the library: every video with a `.funscript` beside it, in each
//! library source, with the text that may name its performers. Read only.

use crate::text::{self, Text};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

/// Video extensions the library scan pairs with scripts
/// (`VIDEO_EXTS` in electron/library-scan.js).
pub const VIDEO_EXTS: &[&str] = &[
    "mp4", "m4v", "mkv", "webm", "avi", "mov", "mp3", "wav", "ogg", "flac", "m4a",
];

#[derive(Debug, Clone)]
pub struct Video {
    /// Source path as written in config.json + the on-disk names.
    pub path: String,
    /// What counts as one scene: the video's folder when it sits in a scene
    /// folder (`<source>/<artist>/<scene>/`), else the video itself.
    pub scene: String,
    /// The folder's name (VRBangers scout scenes name their performers).
    pub folder: String,
    pub text: Text,
    /// `faptap.json` performer names.
    pub faptap_performers: Vec<String>,
}

/// Video path → EroScripts topic slug, from `settings.library.scriptTags`
/// (`topicUrl`); keys in NFC so either spelling finds them.
pub fn topic_slugs(config: &Value) -> HashMap<String, String> {
    let settings = config.get("settings").unwrap_or(config);
    let Some(tags) = settings
        .pointer("/library/scriptTags")
        .and_then(Value::as_object)
    else {
        return HashMap::new();
    };
    tags.iter()
        .filter_map(|(path, record)| {
            let url = record.get("topicUrl")?.as_str()?;
            Some((nfc(path), text::topic_slug(url)?.to_string()))
        })
        .collect()
}

fn nfc(s: &str) -> String {
    s.nfc().collect()
}

/// Walk every source. A source that can't be read is an error: its videos
/// would otherwise lose their tags.
pub fn scan(sources: &[PathBuf], slugs: &HashMap<String, String>) -> Result<Vec<Video>, String> {
    let mut videos = Vec::new();
    for source in sources {
        fs::read_dir(source).map_err(|e| {
            format!(
                "library source {} can't be read ({e}); nothing done",
                source.display()
            )
        })?;
        walk(source, &[], slugs, &mut videos);
    }
    Ok(videos)
}

#[derive(Default)]
struct Sidecars {
    titles: Vec<String>,
    performers: Vec<String>,
}

fn read_sidecars(dir: &Path) -> Sidecars {
    let mut s = Sidecars::default();
    let read = |name: &str| -> Option<Value> {
        serde_json::from_str(&fs::read_to_string(dir.join(name)).ok()?).ok()
    };
    for name in ["eroscripts.json", "faptap.json"] {
        if let Some(title) = read(name)
            .as_ref()
            .and_then(|v| v.get("title")?.as_str().map(str::to_string))
        {
            s.titles.push(title);
        }
    }
    if let Some(v) = read("faptap.json") {
        s.performers = v
            .get("performers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|p| p.get("name")?.as_str())
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .collect();
    }
    s
}

fn walk(dir: &Path, rel: &[String], slugs: &HashMap<String, String>, out: &mut Vec<Video>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut dirs: Vec<(PathBuf, String)> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        // Dotfiles, AppleDouble `._*` sidecars included.
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            dirs.push((dir.join(&name), name));
        } else if kind.is_file() || (kind.is_symlink() && entry.path().is_file()) {
            files.push(name);
        }
    }
    files.sort();
    dirs.sort();

    let ext = |n: &str| {
        Path::new(n)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
    };
    let has_script = files.iter().any(|n| ext(n).as_deref() == Some("funscript"));
    if has_script {
        let sidecars = read_sidecars(dir);
        let folder = rel.last().cloned().unwrap_or_default();
        for name in &files {
            if !ext(name).is_some_and(|e| VIDEO_EXTS.contains(&e.as_str())) {
                continue;
            }
            let Some(path) = dir.join(name).to_str().map(str::to_string) else {
                continue;
            };
            let stem = Path::new(name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(name);
            let mut parts: Vec<&str> = rel.iter().map(String::as_str).collect();
            parts.push(stem);
            parts.extend(sidecars.titles.iter().map(String::as_str));
            parts.extend(sidecars.performers.iter().map(String::as_str));
            if let Some(slug) = slugs.get(&nfc(&path)) {
                parts.push(slug);
            }
            let scene = if rel.len() >= 2 {
                dir.to_string_lossy().into_owned()
            } else {
                path.clone()
            };
            out.push(Video {
                text: Text::new(parts),
                scene,
                folder: folder.clone(),
                faptap_performers: sidecars.performers.clone(),
                path,
            });
        }
    }
    for (path, name) in dirs {
        let mut rel = rel.to_vec();
        rel.push(name);
        walk(&path, &rel, slugs, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Phrase;
    use serde_json::json;

    #[test]
    fn reads_topic_slugs_by_path() {
        let cfg = json!({ "settings": { "library": { "scriptTags": {
            "/V/a.mp4": { "tags": [], "topicUrl": "https://discuss.eroscripts.com/t/nofacegirl-edges-you/42" },
            "/V/b.mp4": { "tags": [], "topicUrl": null },
            "/V/Zoe\u{308}.mp4": { "topicUrl": "https://discuss.eroscripts.com/t/zoe/7" },
        } } } });
        let slugs = topic_slugs(&cfg);
        assert_eq!(slugs.len(), 2);
        assert_eq!(slugs["/V/a.mp4"], "nofacegirl-edges-you");
        assert_eq!(slugs[&nfc("/V/Zoë.mp4")], "zoe");
    }

    #[test]
    fn video_text_has_path_segments_sidecars_and_slug() {
        let tmp = std::env::temp_dir().join(format!("pornstar-tags-lib-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let scene = tmp.join("Artist").join("Scene One");
        fs::create_dir_all(&scene).unwrap();
        for f in [
            "Cut A.mp4",
            "Cut A.funscript",
            "Cut B.mkv",
            "._Cut A.mp4",
            "notes.txt",
        ] {
            fs::write(scene.join(f), b"").unwrap();
        }
        fs::write(
            scene.join("faptap.json"),
            r#"{"title": "Hot Title", "performers": [{"name": "Jane Doe"}]}"#,
        )
        .unwrap();
        fs::write(scene.join("eroscripts.json"), r#"{"title": "Post Title"}"#).unwrap();
        let loose = tmp.join("Loose");
        fs::create_dir_all(&loose).unwrap();
        fs::write(loose.join("x.mp4"), b"").unwrap();
        fs::write(loose.join("x.funscript"), b"").unwrap();
        fs::create_dir_all(tmp.join("NoScript")).unwrap();
        fs::write(tmp.join("NoScript").join("y.mp4"), b"").unwrap();

        let a = format!("{}/Artist/Scene One/Cut A.mp4", tmp.display());
        let slugs = HashMap::from([(a.clone(), "slug-words".to_string())]);
        let videos = scan(std::slice::from_ref(&tmp), &slugs).unwrap();
        let paths: Vec<&str> = videos.iter().map(|v| v.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                a.as_str(),
                &format!("{}/Artist/Scene One/Cut B.mkv", tmp.display()),
                &format!("{}/Loose/x.mp4", tmp.display()),
            ]
        );
        assert_eq!(videos[0].scene, videos[1].scene);
        assert_eq!(videos[2].scene, videos[2].path);
        assert_eq!(videos[0].folder, "Scene One");
        assert_eq!(videos[0].faptap_performers, ["Jane Doe"]);
        for phrase in [
            "Artist",
            "Scene One",
            "Cut A",
            "Hot Title",
            "Post Title",
            "Jane Doe",
            "slug words",
        ] {
            assert!(
                videos[0].text.contains(&Phrase::new(phrase).unwrap()),
                "{phrase}"
            );
        }
        assert!(!videos[1].text.contains(&Phrase::new("slug words").unwrap()));
        assert!(!videos[0].text.contains(&Phrase::new("mp4").unwrap()));
        assert!(scan(&[tmp.join("missing")], &slugs).is_err());
        fs::remove_dir_all(&tmp).unwrap();
    }
}
