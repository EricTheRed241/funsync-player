//! Room on the library drive for a download (the scouts' `--fetch`): the drive must keep
//! `swipeDeck.minFreeGb` (shared/content-preferences.json, `scout_report::drive`) free
//! after the download, so `free − size ≥ minFree`; a size that isn't known needs `free ≥
//! minFree`. These fetches download one video at a time, so nothing else of the run is in
//! flight when a download starts. A drive `df` can't measure lets the download go. A video
//! that doesn't fit is left for later (the fetch goes on with the next, which may be smaller,
//! and exits 2); scripts are too small to check. GB are 10^9 bytes, as on the swipe page.

use std::io::Write;
use std::path::Path;

/// Canned-network runs (tests) take the free bytes from this variable instead of `df`
/// (unset: not measured); see [`gate`].
pub const FAKE_FREE_ENV: &str = "FUNSYNC_FAKE_FREE_BYTES";

/// Whether a download of `bytes` (None: size unknown) into `dir` leaves the library drive
/// its minimum free space; the reason when it doesn't. `dir` must exist to be measured.
pub fn room(dir: &Path, bytes: Option<u64>) -> Result<(), String> {
    let free = scout_report::free_space::drives(&[dir.to_path_buf()])
        .into_iter()
        .map(|d| d.free)
        .min();
    room_with(free, scout_report::drive::min_free_bytes()?, bytes)
}

/// The check a fetch makes before a video download into `dir`, under the item's header
/// line: whether to download it. Not enough room prints `  skipped: not enough disk space
/// (…); left for later` and gives false; a dry run prints the result either way and gives
/// true. With canned network (`fake`, tests) the free space comes from [`FAKE_FREE_ENV`]
/// rather than the real drive.
pub fn gate(
    dir: &Path,
    bytes: Option<u64>,
    fake: bool,
    dry_run: bool,
    out: &mut dyn Write,
) -> std::io::Result<bool> {
    let verdict = if fake {
        let free = std::env::var(FAKE_FREE_ENV)
            .ok()
            .and_then(|v| v.trim().parse().ok());
        scout_report::drive::min_free_bytes().and_then(|min| room_with(free, min, bytes))
    } else {
        room(dir, bytes)
    };
    match (verdict, dry_run) {
        (Ok(()), false) => Ok(true),
        (Ok(()), true) => writeln!(out, "  disk space: enough").map(|()| true),
        (Err(why), true) => {
            writeln!(out, "  disk space: {why}; a real run leaves it for later").map(|()| true)
        }
        (Err(why), false) => writeln!(out, "  skipped: {why}; left for later").map(|()| false),
    }
}

/// The rule itself: `free` bytes on the drive (None: not measured, allowed), `min` to keep
/// free, `bytes` the download's size (None: unknown).
pub fn room_with(free: Option<u64>, min: u64, bytes: Option<u64>) -> Result<(), String> {
    let Some(free) = free else {
        return Ok(());
    };
    let need = min.saturating_add(bytes.unwrap_or(0));
    if free >= need {
        return Ok(());
    }
    Err(match bytes {
        Some(_) => format!(
            "not enough disk space ({} free, {} needed to keep {} free)",
            gb(free),
            gb(need),
            gb(min)
        ),
        None => format!(
            "not enough disk space ({} free, size unknown, under the {} to keep free)",
            gb(free),
            gb(min)
        ),
    })
}

/// `48.3 GB`, `50 GB`.
fn gb(bytes: u64) -> String {
    let s = format!("{:.1}", bytes as f64 / 1e9);
    format!("{} GB", s.strip_suffix(".0").unwrap_or(&s))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 50_000_000_000;

    #[test]
    fn a_download_fits_when_the_minimum_stays_free() {
        let size = 3_000_000_000;
        // Exactly the minimum left after it.
        assert_eq!(room_with(Some(MIN + size), MIN, Some(size)), Ok(()));
        // One byte short.
        assert_eq!(
            room_with(Some(MIN + size - 1), MIN, Some(size)),
            Err("not enough disk space (53 GB free, 53 GB needed to keep 50 GB free)".into())
        );
        assert_eq!(
            room_with(Some(48_300_000_000), MIN, Some(size)),
            Err("not enough disk space (48.3 GB free, 53 GB needed to keep 50 GB free)".into())
        );
        // A smaller one may still fit.
        assert_eq!(room_with(Some(MIN + size - 1), MIN, Some(size / 2)), Ok(()));
    }

    #[test]
    fn an_unknown_size_needs_the_minimum_free() {
        assert_eq!(room_with(Some(MIN), MIN, None), Ok(()));
        assert_eq!(
            room_with(Some(MIN - 1), MIN, None),
            Err(
                "not enough disk space (50 GB free, size unknown, under the 50 GB to keep free)"
                    .into()
            )
        );
    }

    #[test]
    fn an_unmeasured_drive_lets_it_go() {
        assert_eq!(room_with(None, MIN, Some(u64::MAX)), Ok(()));
        assert_eq!(room_with(None, MIN, None), Ok(()));
        // A folder df can't measure (missing) is allowed too.
        assert_eq!(
            room(Path::new("/nonexistent/slr-scout-disk"), Some(1)),
            Ok(())
        );
    }

    #[test]
    fn a_huge_size_does_not_overflow() {
        assert!(room_with(Some(u64::MAX - 1), MIN, Some(u64::MAX)).is_err());
    }
}
