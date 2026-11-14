//! Preview frames the scouts took from the videos (25, 50 and 75 % in), so posts and scenes
//! without pictures still show some: `<userData>/scout-frames/eroscripts/<topic id>/` (by
//! eroscripts:scout, listed in the state record's `frames.files`) and
//! `<userData>/scout-frames/slr/<scene id>/` (by slr:scout, in the scene's `frames.files` in
//! slr-scout.json). Served at `/frames/<source>/<id>/<25|50|75>.jpg`, that exact shape only;
//! a card lists the files of its record that are on disk after its own images.

use crate::deck::MAX_IMAGES;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const DIR: &str = "scout-frames";
/// The only file names a frames folder holds.
pub const NAMES: [&str; 3] = ["25.jpg", "50.jpg", "75.jpg"];

/// Whose frames: an EroScripts post's (by topic id) or an SLR scene's (by scene id).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Eroscripts,
    Slr,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Eroscripts => "eroscripts",
            Source::Slr => "slr",
        }
    }

    fn parse(s: &str) -> Option<Source> {
        match s {
            "eroscripts" => Some(Source::Eroscripts),
            "slr" => Some(Source::Slr),
            _ => None,
        }
    }
}

/// The file a request path names: `/frames/<eroscripts|slr>/<id>/<25|50|75>.jpg` with the id
/// in plain digits (no sign, no leading zero), else `None`.
pub fn file_for(user_data: &Path, path: &str) -> Option<PathBuf> {
    let mut parts = path.strip_prefix("/frames/")?.split('/');
    let (source, id, name) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let source = Source::parse(source)?;
    let id: u64 = id
        .bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| id.parse().ok())
        .flatten()
        .filter(|n: &u64| n.to_string() == id)?;
    let name = NAMES.into_iter().find(|n| *n == name)?;
    Some(folder(user_data, source, id).join(name))
}

fn folder(user_data: &Path, source: Source, id: u64) -> PathBuf {
    user_data
        .join(DIR)
        .join(source.as_str())
        .join(id.to_string())
}

/// The frame URLs of record `rec` (`frames.files`, known names only, each once) whose file is
/// on disk, in the record's order.
pub fn urls(user_data: &Path, source: Source, id: u64, rec: &Value) -> Vec<String> {
    let dir = folder(user_data, source, id);
    let mut out: Vec<String> = Vec::new();
    for name in rec
        .pointer("/frames/files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let url = format!("/frames/{}/{id}/{name}", source.as_str());
        if NAMES.contains(&name) && !out.contains(&url) && dir.join(name).is_file() {
            out.push(url);
        }
    }
    out
}

/// `images` followed by `frames`, at most MAX_IMAGES in all.
pub fn append(images: &mut Vec<String>, frames: Vec<String>) {
    let room = MAX_IMAGES.saturating_sub(images.len());
    images.extend(frames.into_iter().take(room));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names_only_frame_files() {
        let ud = Path::new("/ud");
        assert_eq!(
            file_for(ud, "/frames/eroscripts/12/25.jpg"),
            Some(PathBuf::from("/ud/scout-frames/eroscripts/12/25.jpg"))
        );
        assert_eq!(
            file_for(ud, "/frames/slr/66249/75.jpg"),
            Some(PathBuf::from("/ud/scout-frames/slr/66249/75.jpg"))
        );
        for bad in [
            "/frames/eroscripts/12/26.jpg",
            "/frames/eroscripts/12/25.png",
            "/frames/eroscripts/12/25.jpg/",
            "/frames/eroscripts/12/../25.jpg",
            "/frames/eroscripts/../12/25.jpg",
            "/frames/../eroscripts/12/25.jpg",
            "/frames/eroscripts/12/..%2f25.jpg",
            "/frames/eroscripts/%31%32/25.jpg",
            "/frames/x/12/25.jpg",
            "/frames/Eroscripts/12/25.jpg",
            "/frames/eroscripts/abc/25.jpg",
            "/frames/eroscripts/-1/25.jpg",
            "/frames/eroscripts/+1/25.jpg",
            "/frames/eroscripts/012/25.jpg",
            "/frames/eroscripts/1.5/25.jpg",
            "/frames/eroscripts//25.jpg",
            "/frames/eroscripts/99999999999999999999999/25.jpg",
            "/frames/eroscripts/12",
            "/frames/eroscripts/12/",
            "/frames/eroscripts/12/a/25.jpg",
            "/frames//eroscripts/12/25.jpg",
            "/frames/",
            "/frames",
            "//frames/eroscripts/12/25.jpg",
            "/x/frames/eroscripts/12/25.jpg",
        ] {
            assert_eq!(file_for(ud, bad), None, "{bad}");
        }
    }

    #[test]
    fn lists_the_frames_on_disk() {
        let ud = std::env::temp_dir().join(format!("scout-report-frames-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ud);
        let dir = ud.join("scout-frames/eroscripts/7");
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["25.jpg", "75.jpg", "other.jpg"] {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
        let rec = json!({"frames": {"at": "2026-11-14T00:00:00.000Z", "source": "MEGA: a.mp4",
            "files": ["75.jpg", "50.jpg", "25.jpg", "75.jpg", "other.jpg", "../x.jpg", 3]}});
        // 50 isn't on disk; unknown names never count.
        assert_eq!(
            urls(&ud, Source::Eroscripts, 7, &rec),
            ["/frames/eroscripts/7/75.jpg", "/frames/eroscripts/7/25.jpg"]
        );
        assert!(urls(&ud, Source::Slr, 7, &rec).is_empty());
        assert!(urls(&ud, Source::Eroscripts, 8, &rec).is_empty());
        assert!(urls(&ud, Source::Eroscripts, 7, &json!({"frames": null})).is_empty());
        assert!(urls(&ud, Source::Eroscripts, 7, &Value::Null).is_empty());
        let _ = std::fs::remove_dir_all(&ud);
    }

    #[test]
    fn frames_follow_the_images_up_to_the_cap() {
        let s = |list: &[&str]| list.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let mut images = s(&["https://a", "https://b"]);
        append(&mut images, s(&["/f/25", "/f/50", "/f/75"]));
        assert_eq!(
            images,
            s(&["https://a", "https://b", "/f/25", "/f/50", "/f/75"])
        );
        let mut images = s(&["1", "2", "3", "4"]);
        append(&mut images, s(&["/f/25", "/f/50"]));
        assert_eq!(images, s(&["1", "2", "3", "4", "/f/25"]));
        let mut images = s(&["1", "2", "3", "4", "5"]);
        append(&mut images, s(&["/f/25"]));
        assert_eq!(images.len(), MAX_IMAGES);
        let mut images = Vec::new();
        append(&mut images, s(&["/f/25"]));
        assert_eq!(images, s(&["/f/25"]));
    }
}
