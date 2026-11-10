//! `<userData>/pornstar-tags.json`, owned by this CLI: every performer's
//! scene count and whether her tag is applied, and video path → applied
//! tags. Recomputed from the library on every run; written atomically
//! (previous file kept as `pornstar-tags.json.bak`) only when its content
//! changes. Read by the app on every library scan (electron/video-tags.js).
//! `pornstar-tags.last-run` records the last `--apply` run, for `--daily`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use video_tags::store::write_json_atomic;

pub const FILE_NAME: &str = "pornstar-tags.json";
pub const BACKUP_NAME: &str = "pornstar-tags.json.bak";
pub const LAST_RUN_NAME: &str = "pornstar-tags.last-run";
pub const VERSION: u64 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pornstar {
    pub name: String,
    /// Distinct scene folders that show her.
    pub scenes: usize,
    /// `scenes` reached `minScenes`: her tag is on her videos.
    pub applied: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PornstarFile {
    pub version: u64,
    #[serde(default)]
    pub updated_at: Option<String>,
    pub min_scenes: usize,
    /// Keyed by tag.
    #[serde(default)]
    pub pornstars: BTreeMap<String, Pornstar>,
    /// Video path (source as in config.json + on-disk names) → applied tags.
    #[serde(default)]
    pub videos: BTreeMap<String, Vec<String>>,
}

impl PornstarFile {
    pub fn new(
        min_scenes: usize,
        pornstars: BTreeMap<String, Pornstar>,
        videos: BTreeMap<String, Vec<String>>,
    ) -> Self {
        PornstarFile {
            version: VERSION,
            updated_at: None,
            min_scenes,
            pornstars,
            videos,
        }
    }

    /// Equal apart from `updatedAt`.
    pub fn same_content(&self, other: &PornstarFile) -> bool {
        self.version == other.version
            && self.min_scenes == other.min_scenes
            && self.pornstars == other.pornstars
            && self.videos == other.videos
    }
}

pub fn file_path(user_data: &Path) -> PathBuf {
    user_data.join(FILE_NAME)
}

/// `Ok(None)` when the file doesn't exist; an error when it can't be read or
/// isn't a version 1 pornstar tags file (never overwrite what we don't
/// understand).
pub fn read(path: &Path) -> Result<Option<PornstarFile>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let file: PornstarFile = serde_json::from_str(&raw).map_err(|e| {
        format!(
            "{} is not a pornstar tags file ({e}); move it aside and re-run",
            path.display()
        )
    })?;
    if file.version != VERSION {
        return Err(format!(
            "{} is version {}, this CLI writes version {VERSION}; move it aside and re-run",
            path.display(),
            file.version
        ));
    }
    Ok(Some(file))
}

/// Write atomically; returns whether the previous file was kept as `.bak`.
pub fn write(user_data: &Path, file: &PornstarFile) -> Result<bool, String> {
    write_json_atomic(user_data, FILE_NAME, BACKUP_NAME, file)
}

/// The last `--apply` run's time, if recorded.
pub fn last_run(user_data: &Path) -> Option<String> {
    let raw = fs::read_to_string(user_data.join(LAST_RUN_NAME)).ok()?;
    let t = raw.trim();
    (!t.is_empty()).then(|| t.to_string())
}

pub fn record_run(user_data: &Path, now: &str) -> Result<(), String> {
    let path = user_data.join(LAST_RUN_NAME);
    fs::write(&path, format!("{now}\n")).map_err(|e| format!("writing {}: {e}", path.display()))
}

/// Whether one of the ISO times falls on `now`'s (UTC) day.
pub fn ran_today(times: &[Option<&str>], now: &str) -> bool {
    let day = |t: &str| t.get(..10).map(str::to_string);
    let today = day(now);
    today.is_some() && times.iter().flatten().any(|t| day(t) == today)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_content_ignores_updated_at() {
        let mut a = PornstarFile::new(2, BTreeMap::new(), BTreeMap::new());
        let mut b = a.clone();
        a.updated_at = Some("2026-11-09T10:00:00.000Z".into());
        assert!(a.same_content(&b));
        b.videos.insert("/a.mp4".into(), vec!["x".into()]);
        assert!(!a.same_content(&b));
    }

    #[test]
    fn daily_skips_only_on_the_same_day() {
        let now = "2026-11-10T08:00:00.000Z";
        assert!(ran_today(&[None, Some("2026-11-10T00:00:01.000Z")], now));
        assert!(ran_today(&[Some("2026-11-10T23:59:59.000Z"), None], now));
        assert!(!ran_today(&[Some("2026-11-09T23:59:59.999Z")], now));
        assert!(!ran_today(&[None, None], now));
        assert!(!ran_today(&[Some("garbage")], now));
    }

    #[test]
    fn round_trips_and_keeps_a_backup() {
        let dir = std::env::temp_dir().join(format!("pornstar-tags-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(read(&file_path(&dir)), Ok(None));
        let mut f = PornstarFile::new(
            2,
            BTreeMap::from([(
                "candy-love".into(),
                Pornstar {
                    name: "Candy Love".into(),
                    scenes: 3,
                    applied: true,
                },
            )]),
            BTreeMap::from([("/a.mp4".into(), vec!["candy-love".into()])]),
        );
        f.updated_at = Some("T".into());
        assert_eq!(write(&dir, &f), Ok(false));
        assert_eq!(write(&dir, &f), Ok(true));
        assert_eq!(read(&file_path(&dir)).unwrap(), Some(f));
        let raw = fs::read_to_string(file_path(&dir)).unwrap();
        assert!(raw.contains("\"minScenes\": 2") && raw.contains("\"updatedAt\""));
        fs::write(file_path(&dir), r#"{"version": 2, "minScenes": 2}"#).unwrap();
        assert!(read(&file_path(&dir)).is_err());
        assert_eq!(last_run(&dir), None);
        record_run(&dir, "2026-11-10T08:00:00.000Z").unwrap();
        assert_eq!(last_run(&dir).as_deref(), Some("2026-11-10T08:00:00.000Z"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
