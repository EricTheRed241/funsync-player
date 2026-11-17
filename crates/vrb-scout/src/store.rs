//! `<userData>/vrb-scout.json`: the scored scenes, read by the `scout:report`
//! swipe page (the shared scout-scene shape), written atomically (temp file
//! + rename; the previous file kept as `vrb-scout.json.bak`).
//!
//! Each scene also keeps what the next scan needs to skip its detail request
//! (`slug`, `shortName`, `qualities`, `categories`, `updatedAt`, `listKey`).
//! Screened-out scenes go to `excluded` with their reason, so a rescan
//! re-screens them from the same cache instead of asking again.

use serde::{Deserialize, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "vrb-scout.json";
pub const BACKUP_NAME: &str = "vrb-scout.json.bak";
pub const VERSION: u64 = 1;

/// The scene's script: the public toy script (stroke only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Script {
    pub url: String,
    /// Axes besides stroke; the toy scripts have none.
    #[serde(default)]
    pub axes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    /// The site's own id (hex).
    pub id: String,
    /// `sites::Site::key`.
    pub site: String,
    pub url: String,
    pub title: String,
    pub studio: String,
    pub actors: Vec<String>,
    /// `YYYY-MM-DD` (UTC) of publication.
    pub date: Option<String>,
    pub duration_s: Option<u64>,
    pub thumbnail: Option<String>,
    /// Gallery stills after the thumbnail (at most `api::IMAGES`).
    #[serde(default)]
    pub images: Vec<String>,
    pub vr: bool,
    #[serde(serialize_with = "opt_number")]
    pub fps: Option<f64>,
    pub passthrough: bool,
    pub creampie: bool,
    pub pmv: bool,
    pub compilation: bool,
    /// The site's view count (the score's popularity).
    pub likes: u64,
    /// Category names as tags (`score::scene_tags`), animation tags dropped.
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, serialize_with = "numbers")]
    pub tag_scores: BTreeMap<String, f64>,
    #[serde(default)]
    pub favourite: Option<String>,
    pub script: Option<Script>,
    /// Hand-made scripts (the shared shape); none known for these scenes.
    #[serde(default)]
    pub handmade: Vec<serde_json::Value>,
    /// Already in the library (the deck hides it; `--fetch` skips it).
    #[serde(default)]
    pub in_library: bool,
    /// When `--fetch` put the scene in the library (kept across scans).
    #[serde(default)]
    pub fetched_at: Option<String>,
    #[serde(serialize_with = "number")]
    pub score: f64,
    #[serde(serialize_with = "numbers")]
    pub score_parts: BTreeMap<String, f64>,
    // The scan's cache.
    pub slug: String,
    #[serde(default)]
    pub short_name: Option<String>,
    /// The download qualities (`8K_180x180_3dh` …).
    #[serde(default)]
    pub qualities: Vec<String>,
    /// The site's category names, as given.
    #[serde(default)]
    pub categories: Vec<String>,
    /// The detail's `updatedAt` (epoch seconds); None when the detail
    /// couldn't be read (asked again next scan).
    #[serde(default)]
    pub updated_at: Option<i64>,
    /// `api::fingerprint` of the listing entry the detail belongs to.
    #[serde(default)]
    pub list_key: String,
    /// Why the screen turned it down (only in `excluded`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excluded: Option<String>,
}

impl Scene {
    /// The projection of the best listed quality (`180x180_3dh`).
    pub fn projection(&self) -> Option<&str> {
        self.qualities
            .iter()
            .filter_map(|q| q.split_once('_'))
            .max_by_key(|(k, _)| k.trim_end_matches(['K', 'k']).parse::<u32>().unwrap_or(0))
            .map(|(_, p)| p)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoutFile {
    pub version: u64,
    pub generated_at: String,
    pub scenes: Vec<Scene>,
    #[serde(default)]
    pub excluded: Vec<Scene>,
}

impl ScoutFile {
    /// A scene of either list by site and id.
    pub fn find(&self, site: &str, id: &str) -> Option<&Scene> {
        self.scenes
            .iter()
            .chain(&self.excluded)
            .find(|s| s.site == site && s.id == id)
    }
}

/// Whole numbers as integers (`4`, not `4.0`), the way the app writes them.
fn number<S: Serializer>(x: &f64, s: S) -> Result<S::Ok, S::Error> {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        s.serialize_i64(*x as i64)
    } else {
        s.serialize_f64(*x)
    }
}

fn opt_number<S: Serializer>(x: &Option<f64>, s: S) -> Result<S::Ok, S::Error> {
    match x {
        Some(x) => number(x, s),
        None => s.serialize_none(),
    }
}

fn numbers<S: Serializer>(map: &BTreeMap<String, f64>, s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    struct N(f64);
    impl Serialize for N {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            number(&self.0, s)
        }
    }
    let mut m = s.serialize_map(Some(map.len()))?;
    for (k, v) in map {
        m.serialize_entry(k, &N(*v))?;
    }
    m.end()
}

pub fn file_path(user_data: &Path) -> PathBuf {
    user_data.join(FILE_NAME)
}

/// The previous file: `Ok(None)` when there is none; an error when it can't
/// be read or isn't this CLI's file.
pub fn read(path: &Path) -> Result<Option<ScoutFile>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    serde_json::from_str(&raw).map(Some).map_err(|e| {
        format!(
            "{} is not a vrb-scout file ({e}); move it aside and re-run",
            path.display()
        )
    })
}

/// Write the file atomically; returns whether the previous one was backed up.
pub fn write(user_data: &Path, file: &ScoutFile) -> Result<bool, String> {
    video_tags::store::write_json_atomic(user_data, FILE_NAME, BACKUP_NAME, file)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use serde_json::{Value, json};

    pub fn scene(id: &str) -> Scene {
        Scene {
            id: id.into(),
            site: "vrbangers".into(),
            url: "https://vrbangers.com/video/wet/".into(),
            title: "Wet & Wild".into(),
            studio: "VRBangers".into(),
            actors: vec!["Angel Youngs".into()],
            date: Some("2026-10-14".into()),
            duration_s: Some(4041),
            thumbnail: Some("https://content.vrbangers.com/p.jpg".into()),
            images: vec![],
            vr: true,
            fps: Some(60.0),
            passthrough: false,
            creampie: false,
            pmv: false,
            compilation: false,
            likes: 3233,
            tags: vec!["babe".into()],
            tag_scores: BTreeMap::new(),
            favourite: None,
            script: Some(Script {
                url: "https://mov.vrbangers.com/vrb_wet/MISC/VRBANGERS_wet_toy.funscript".into(),
                axes: vec![],
            }),
            handmade: vec![],
            in_library: false,
            fetched_at: None,
            score: 7.5,
            score_parts: BTreeMap::from([("vr".into(), 3.0), ("likes".into(), 0.11)]),
            slug: "wet".into(),
            short_name: Some("wet".into()),
            qualities: vec!["2K_180x180_3dh".into(), "8K_180x180_3dh".into()],
            categories: vec!["Babe".into()],
            updated_at: Some(1),
            list_key: "k".into(),
            excluded: None,
        }
    }

    #[test]
    fn serialises_in_the_contract_shape() {
        let v = serde_json::to_value(scene("6accfc2cff316f0c39086f52")).unwrap();
        assert_eq!(v["id"], "6accfc2cff316f0c39086f52");
        assert_eq!(v["durationS"], 4041);
        assert_eq!(v["fps"], 60);
        assert_eq!(v["score"], 7.5);
        assert_eq!(v["scoreParts"], json!({"vr": 3, "likes": 0.11}));
        assert_eq!(
            v["script"],
            json!({"url": "https://mov.vrbangers.com/vrb_wet/MISC/VRBANGERS_wet_toy.funscript", "axes": []})
        );
        assert_eq!(v["inLibrary"], false);
        assert_eq!(v["fetchedAt"], Value::Null);
        assert_eq!(v["handmade"], json!([]));
        assert!(v.get("excluded").is_none());
        let back: Scene = serde_json::from_value(v).unwrap();
        assert_eq!(back, scene("6accfc2cff316f0c39086f52"));
    }

    #[test]
    fn projection_of_the_best_quality() {
        let mut s = scene("a");
        assert_eq!(s.projection(), Some("180x180_3dh"));
        s.qualities = vec!["8K_360x180_3dh".into(), "10K_180x180_3dh".into()];
        assert_eq!(s.projection(), Some("180x180_3dh"));
        s.qualities.clear();
        assert_eq!(s.projection(), None);
    }

    #[test]
    fn reads_and_writes() {
        let dir = std::env::temp_dir().join(format!("vrb-scout-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(read(&file_path(&dir)).unwrap(), None);
        let file = ScoutFile {
            version: VERSION,
            generated_at: "now".into(),
            scenes: vec![scene("a")],
            excluded: vec![],
        };
        write(&dir, &file).unwrap();
        let back = read(&file_path(&dir)).unwrap().unwrap();
        assert_eq!(back, file);
        assert!(back.find("vrbangers", "a").is_some());
        assert!(back.find("vrconk", "a").is_none());
        fs::write(file_path(&dir), "[]").unwrap();
        assert!(read(&file_path(&dir)).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
