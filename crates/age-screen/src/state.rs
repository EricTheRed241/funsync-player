//! `<userData>/age-screen.json`: every screened video (by path) with the
//! file size it had, the judge's `p_minor` and reason, and what was done.
//! Only this CLI writes it (temp file + rename, the previous file kept as
//! `age-screen.json.bak`). Plus `age-screen.lock`, so runs never overlap.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use video_tags::host::{self, LockError, PidLock};

pub const FILE_NAME: &str = "age-screen.json";
pub const BACKUP_NAME: &str = "age-screen.json.bak";
pub const LOCK_NAME: &str = "age-screen.lock";
pub const VERSION: u64 = 1;

/// What the screener did with a video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// Rated 1★: the desktop deletes it.
    Purged,
    /// A favourite at the purge level: in the review playlist (1★ never
    /// deletes favourites).
    ReviewFavourite,
    /// In the review playlist.
    Review,
    /// Nothing to do.
    Clear,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Purged => "purged",
            Action::ReviewFavourite => "review-favourite",
            Action::Review => "review",
            Action::Clear => "clear",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// File size in bytes when screened; another size means screen again.
    pub size: u64,
    pub screened_at: String,
    pub p_minor: f64,
    #[serde(default)]
    pub reason: String,
    pub action: Action,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateFile {
    pub version: u64,
    /// When the last complete run finished (nothing left); `--weekly` counts
    /// from it.
    #[serde(default)]
    pub last_run: Option<String>,
    /// Keyed by absolute video path, as the backend lists it.
    #[serde(default)]
    pub videos: BTreeMap<String, Entry>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl StateFile {
    pub fn empty() -> Self {
        StateFile {
            version: VERSION,
            last_run: None,
            videos: BTreeMap::new(),
            extra: Map::new(),
        }
    }
}

pub fn file_path(user_data: &Path) -> PathBuf {
    user_data.join(FILE_NAME)
}

/// Read the state: `Ok(None)` when it doesn't exist; an error when it can't
/// be read, isn't JSON or is another version (never overwrite what we don't
/// understand).
pub fn read(path: &Path) -> Result<Option<StateFile>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let file: StateFile = serde_json::from_str(&raw).map_err(|e| {
        format!(
            "{} is not an age-screen state file ({e}); move it aside and re-run",
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

/// Write the state atomically (temp file in the same folder, renamed over
/// the target); an existing file is copied to `age-screen.json.bak` first.
pub fn write_atomic(user_data: &Path, file: &StateFile) -> Result<(), String> {
    let target = file_path(user_data);
    let tmp = user_data.join(format!("{FILE_NAME}.{}.tmp", std::process::id()));
    let result = (|| -> io::Result<()> {
        let mut json = serde_json::to_string_pretty(file).map_err(io::Error::other)?;
        json.push('\n');
        let mut out = fs::File::create(&tmp)?;
        out.write_all(json.as_bytes())?;
        out.sync_all()?;
        drop(out);
        match fs::copy(&target, user_data.join(BACKUP_NAME)) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|e| format!("writing {}: {e}", target.display()))
}

/// Whether `--weekly` should run: never ran, an unreadable `lastRun`, or
/// the last complete run at least `interval_days` ago.
pub fn is_due(last_run: Option<&str>, now_ms: i64, interval_days: f64) -> bool {
    let Some(last) = last_run.and_then(crate::timefmt::parse_iso) else {
        return true;
    };
    (now_ms - last) as f64 >= interval_days * 86_400_000.0
}

/// Take `age-screen.lock` (`<pid>@<host>`, removed on drop). A lock whose
/// run has ended (this host: its pid is gone; another host: older than
/// `FOREIGN_LOCK_MAX_AGE`) or that is unreadable is stale and taken over;
/// a live one is an error.
pub fn lock(user_data: &Path) -> Result<PidLock, String> {
    let path = user_data.join(LOCK_NAME);
    host::take_lock(&path).map_err(|e| match e {
        LockError::Held(holder) => {
            format!("another age-screen run ({holder}) holds {}", path.display())
        }
        LockError::Io(m) => m,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::{Duration, SystemTime};
    use video_tags::host::LockHolder;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("age-screen-state-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn weekly_is_due_after_the_interval() {
        let day = 86_400_000;
        let last = crate::timefmt::iso_utc(10 * day);
        assert!(is_due(None, 0, 7.0));
        assert!(is_due(Some("garbage"), 0, 7.0));
        assert!(!is_due(Some(&last), 10 * day, 7.0));
        assert!(!is_due(Some(&last), 17 * day - 1, 7.0));
        assert!(is_due(Some(&last), 17 * day, 7.0));
        assert!(is_due(Some(&last), 30 * day, 7.0));
    }

    #[test]
    fn a_stale_lock_is_taken_over_and_a_live_one_refused() {
        let dir = temp("lock");
        let lock_path = dir.join(LOCK_NAME);
        // A pid that has exited, in the old format (no host): this host's.
        let mut child = Command::new("true").spawn().unwrap();
        let dead = child.id();
        child.wait().unwrap();
        fs::write(&lock_path, dead.to_string()).unwrap();
        let held = lock(&dir).unwrap();
        assert_eq!(
            fs::read_to_string(&lock_path).unwrap(),
            LockHolder::me().to_text()
        );
        // Held by a live process (this one): refused, left in place.
        assert!(lock(&dir).unwrap_err().contains("another age-screen run"));
        drop(held);
        assert!(!lock_path.exists());
        // Garbage in the file counts as stale.
        fs::write(&lock_path, "not a pid").unwrap();
        drop(lock(&dir).unwrap());
        // Another host's (a pod's, the Mac's): its pid is never checked here;
        // held for 12 hours, then stale.
        fs::write(&lock_path, format!("{dead}@some-other-host")).unwrap();
        let err = lock(&dir).unwrap_err();
        assert!(
            err.contains(&format!("pid {dead} on some-other-host")),
            "{err}"
        );
        fs::OpenOptions::new()
            .write(true)
            .open(&lock_path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(13 * 3600))
            .unwrap();
        drop(lock(&dir).unwrap());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round_trips_and_keeps_a_backup() {
        let dir = temp("store");
        let mut s = StateFile::empty();
        s.videos.insert(
            "/V/a.mp4".into(),
            Entry {
                size: 5,
                screened_at: "2026-11-04T12:00:00.000Z".into(),
                p_minor: 0.6,
                reason: "r".into(),
                action: Action::ReviewFavourite,
                extra: Map::new(),
            },
        );
        write_atomic(&dir, &s).unwrap();
        let raw = fs::read_to_string(file_path(&dir)).unwrap();
        assert!(raw.contains("\"action\": \"review-favourite\""));
        assert!(raw.contains("\"pMinor\": 0.6"));
        assert!(!dir.join(BACKUP_NAME).exists());
        s.last_run = Some("2026-11-04T12:00:00.000Z".into());
        write_atomic(&dir, &s).unwrap();
        assert!(dir.join(BACKUP_NAME).exists());
        assert_eq!(read(&file_path(&dir)).unwrap(), Some(s));
        fs::write(file_path(&dir), r#"{"version": 2}"#).unwrap();
        assert!(read(&file_path(&dir)).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
