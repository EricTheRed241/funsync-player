//! The library drive's free space. A right swipe queues a download, so the download tabs stop
//! swiping while the drive holding the library sources (config.json) has less than
//! `swipeDeck.minFreeGb` free (the shared preferences, `renderer/js/content-preferences.json`,
//! embedded at compile time). Measured with `df` (crate::free_space::drives) on every request;
//! with sources on several drives, the one with the least free space counts. The Free space
//! tab's marks are never refused: deleting is how the drive gets room again. GB here are 10^9
//! bytes, as everywhere on the page.

use crate::free_space::{self, Drive};
use serde::{Deserialize, Serialize};
use std::path::Path;

const SHARED: &str = include_str!("../../../renderer/js/content-preferences.json");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Shared {
    swipe_deck: SwipeDeck,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwipeDeck {
    min_free_gb: f64,
}

/// `swipeDeck.minFreeGb` in bytes.
pub fn min_free_bytes() -> Result<u64, String> {
    let shared: Shared = serde_json::from_str(SHARED)
        .map_err(|e| format!("renderer/js/content-preferences.json: {e}"))?;
    let gb = shared.swipe_deck.min_free_gb;
    if !(gb.is_finite() && gb >= 0.0) {
        return Err("content-preferences.json: swipeDeck.minFreeGb must be 0 or more".into());
    }
    Ok((gb * 1e9) as u64)
}

/// What the page shows and the swipe routes obey (`GET /api/drive`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// The library drive with the least free space; None when no source is mounted (then a
    /// line in `problems`).
    pub drive: Option<Drive>,
    /// Swiping stops below this (`swipeDeck.minFreeGb`); None when the preferences can't be read.
    pub min_free_bytes: Option<u64>,
    /// The drive has less than `min_free_bytes` free: post and scene swipes are refused.
    pub full: bool,
    pub problems: Vec<String>,
}

/// Read config.json from `user_data` and measure the drives of its library sources.
pub fn status(user_data: &Path) -> Status {
    let mut problems = Vec::new();
    let drives = match video_tags::config::read(user_data) {
        Ok(config) => free_space::drives(&config.sources),
        Err(e) => {
            problems.push(e);
            Vec::new()
        }
    };
    if drives.is_empty() && problems.is_empty() {
        problems.push("no library source is mounted".into());
    }
    let min_free = min_free_bytes().map_err(|e| problems.push(e)).ok();
    judge(drives, min_free, problems)
}

/// The tightest of `drives`, and whether it's below `min_free` (never full without a limit).
pub fn judge(drives: Vec<Drive>, min_free: Option<u64>, problems: Vec<String>) -> Status {
    let drive = drives.into_iter().min_by_key(|d| d.free);
    let full = matches!((&drive, min_free), (Some(d), Some(min)) if d.free < min);
    Status {
        drive,
        min_free_bytes: min_free,
        full,
        problems,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1_000_000_000;

    #[test]
    fn the_tightest_drive_decides_and_full_is_strictly_below_the_limit() {
        let d = |mount: &str, free| Drive {
            mount: mount.into(),
            free,
            total: 2000 * GB,
        };
        let s = judge(
            vec![d("/a", 80 * GB), d("/b", 30 * GB)],
            Some(50 * GB),
            vec![],
        );
        assert_eq!((s.drive.unwrap().mount.as_str(), s.full), ("/b", true));
        assert!(!judge(vec![d("/a", 50 * GB)], Some(50 * GB), vec![]).full);
        assert!(judge(vec![d("/a", 50 * GB - 1)], Some(50 * GB), vec![]).full);
        // No drive measured, or no limit: swiping stays on.
        assert!(!judge(vec![], Some(50 * GB), vec![]).full);
        assert!(!judge(vec![d("/a", 0)], None, vec![]).full);
    }

    #[test]
    fn measures_the_sources_in_config() {
        let dir = std::env::temp_dir().join(format!("scout-report-drive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = dir.join("Library Source");
        std::fs::create_dir_all(&lib).unwrap();
        let config = serde_json::json!({"settings": {"library": {"sources": [
            {"path": lib.to_str().unwrap()}, {"path": dir.join("unplugged").to_str().unwrap()}
        ]}}});
        std::fs::write(dir.join("config.json"), config.to_string()).unwrap();
        let s = status(&dir);
        let drive = s.drive.expect("the temp dir's drive");
        assert!(drive.total > 0 && drive.free <= drive.total);
        assert_eq!(s.problems, Vec::<String>::new());

        std::fs::remove_dir_all(&lib).unwrap();
        let s = status(&dir);
        assert_eq!((s.drive.is_none(), s.full), (true, false));
        assert_eq!(s.problems, ["no library source is mounted"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
