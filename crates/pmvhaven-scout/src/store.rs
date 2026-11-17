//! `<userData>/pmvhaven-scout.json`: the scored videos, read by the
//! `scout:report` swipe page (the shared scene-file contract of the Rust
//! scouts), and `<userData>/pmvhaven-scout-cache.json`: each video's
//! details, so a scan reads only new and stale ones. Both written
//! atomically (temp file + rename; the previous file kept as `.bak`).

use crate::api::Detail;
use crate::script::Script;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "pmvhaven-scout.json";
pub const BACKUP_NAME: &str = "pmvhaven-scout.json.bak";
pub const CACHE_NAME: &str = "pmvhaven-scout-cache.json";
pub const CACHE_BACKUP_NAME: &str = "pmvhaven-scout-cache.json.bak";
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    /// PMVHaven's `_id`.
    pub id: String,
    /// Always `pmvhaven`.
    pub site: String,
    pub url: String,
    pub title: String,
    /// Who made the edit: `originalCreator`, else the uploader.
    pub studio: Option<String>,
    /// The performers (`starsTags`).
    pub actors: Vec<String>,
    pub date: Option<String>,
    pub duration_s: Option<u64>,
    pub thumbnail: Option<String>,
    /// Timeline stills, at most 4.
    pub images: Vec<String>,
    pub vr: bool,
    /// The layout's file-name suffix (`classify::vr_suffix`; empty when flat).
    #[serde(default)]
    pub vr_suffix: String,
    /// PMVHaven doesn't say.
    pub fps: Option<f64>,
    pub passthrough: bool,
    pub creampie: bool,
    pub pmv: bool,
    pub compilation: bool,
    /// PMVHaven likes (what the score's likes part used).
    pub likes: u64,
    #[serde(default)]
    pub views: u64,
    pub tags: Vec<String>,
    #[serde(serialize_with = "numbers")]
    pub tag_scores: BTreeMap<String, f64>,
    pub favourite: Option<String>,
    pub script: Option<Script>,
    #[serde(default)]
    pub handmade: Vec<serde_json::Value>,
    pub in_library: bool,
    pub fetched_at: Option<String>,
    /// The mp4 `--fetch` downloads.
    pub video_url: String,
    #[serde(default)]
    pub file_size: Option<u64>,
    pub uploader: String,
    #[serde(serialize_with = "number")]
    pub score: f64,
    #[serde(serialize_with = "numbers")]
    pub score_parts: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoutFile {
    pub version: u32,
    pub generated_at: String,
    pub scenes: Vec<Scene>,
}

impl ScoutFile {
    pub fn scene(&self, id: &str) -> Option<&Scene> {
        self.scenes.iter().find(|s| s.id == id)
    }
}

/// A video's details as last read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheEntry {
    pub checked_at: String,
    #[serde(flatten)]
    pub detail: Detail,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Cache {
    pub version: u32,
    pub videos: BTreeMap<String, CacheEntry>,
}

/// Whole numbers as integers (`4`, not `4.0`), the way the app writes them.
fn number<S: Serializer>(x: &f64, s: S) -> Result<S::Ok, S::Error> {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        s.serialize_i64(*x as i64)
    } else {
        s.serialize_f64(*x)
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

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    serde_json::from_str(&raw).map(Some).map_err(|e| {
        format!(
            "{} is not a pmvhaven-scout file ({e}); move it aside and re-run",
            path.display()
        )
    })
}

/// The previous scene file, `Ok(None)` when there is none.
pub fn read(user_data: &Path) -> Result<Option<ScoutFile>, String> {
    read_json(&file_path(user_data))
}

pub fn write(user_data: &Path, file: &ScoutFile) -> Result<bool, String> {
    video_tags::store::write_json_atomic(user_data, FILE_NAME, BACKUP_NAME, file)
}

/// The detail cache (empty when there is none).
pub fn read_cache(user_data: &Path) -> Result<Cache, String> {
    Ok(read_json(&user_data.join(CACHE_NAME))?.unwrap_or(Cache {
        version: VERSION,
        videos: BTreeMap::new(),
    }))
}

pub fn write_cache(user_data: &Path, cache: &Cache) -> Result<bool, String> {
    video_tags::store::write_json_atomic(user_data, CACHE_NAME, CACHE_BACKUP_NAME, cache)
}

/// Days since 1970-01-01 of an ISO date (`YYYY-MM-DD…`).
pub fn epoch_days(iso: &str) -> Option<i64> {
    let y: i64 = iso.get(0..4)?.parse().ok()?;
    let m: i64 = iso.get(5..7)?.parse().ok()?;
    let d: i64 = iso.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// A cache entry read within `max_days` of `now` (ISO).
pub fn fresh(entry: &CacheEntry, now: &str, max_days: u64) -> bool {
    match (epoch_days(&entry.checked_at), epoch_days(now)) {
        (Some(then), Some(now)) => now - then < max_days as i64,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_days() {
        assert_eq!(epoch_days("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(epoch_days("2026-11-17T18:00:00.000Z"), Some(20_774));
        assert_eq!(epoch_days("2024-03-01"), Some(19_783));
        assert_eq!(epoch_days("nope"), None);
        let entry = CacheEntry {
            checked_at: "2026-10-20T00:00:00Z".into(),
            detail: Detail::default(),
        };
        assert!(fresh(&entry, "2026-11-17T00:00:00Z", 30));
        assert!(!fresh(&entry, "2026-11-19T00:00:00Z", 30));
    }

    #[test]
    fn writes_whole_numbers_as_integers() {
        let mut parts = BTreeMap::new();
        parts.insert("vr".to_string(), 3.0);
        parts.insert("likes".to_string(), 0.42);
        let v = serde_json::to_value(TestParts(parts)).unwrap();
        assert_eq!(v.to_string(), r#"{"likes":0.42,"vr":3}"#);
    }

    struct TestParts(BTreeMap<String, f64>);
    impl Serialize for TestParts {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            numbers(&self.0, s)
        }
    }
}
