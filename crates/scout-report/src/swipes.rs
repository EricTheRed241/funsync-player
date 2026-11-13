//! `<userData>/eroscripts-scout-swipes.json`: the deck's verdicts, the only
//! file the CLI writes. Right = wanted (the scout's `--fetch --auto` downloads
//! the post whatever its score or axes), left = hidden from the deck only (the
//! scout ignores it). Undo removes the entry.
//!
//! ```json
//! { "version": 1, "swipes": { "343669": { "dir": "right", "at": "2026-11-13T15:00:00.000Z" } } }
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const FILE: &str = "eroscripts-scout-swipes.json";
/// The previous file, kept by every write (`video_tags::store::write_json_atomic`).
pub const BACKUP: &str = "eroscripts-scout-swipes.json.bak";
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Right,
    Left,
}

impl Dir {
    pub fn parse(s: &str) -> Option<Dir> {
        match s {
            "right" => Some(Dir::Right),
            "left" => Some(Dir::Left),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Swipe {
    pub dir: Dir,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Swipes {
    pub version: u32,
    /// By topic id (a string key in the file).
    #[serde(default)]
    pub swipes: BTreeMap<u64, Swipe>,
}

impl Default for Swipes {
    fn default() -> Self {
        Swipes {
            version: VERSION,
            swipes: BTreeMap::new(),
        }
    }
}

impl Swipes {
    pub fn dir(&self, id: u64) -> Option<Dir> {
        self.swipes.get(&id).map(|s| s.dir)
    }
}

/// Read the file; a missing one is empty. An unreadable, unparsable or
/// other-version file is an error naming it (the caller reports it as a
/// problem and never writes over it).
pub fn read(user_data: &Path) -> Result<Swipes, String> {
    let bytes = match std::fs::read(user_data.join(FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Swipes::default()),
        Err(e) => return Err(format!("{FILE}: {e}")),
    };
    let swipes: Swipes = serde_json::from_slice(&bytes).map_err(|e| format!("{FILE}: {e}"))?;
    if swipes.version != VERSION {
        return Err(format!(
            "{FILE}: version {}, this CLI reads version {VERSION}",
            swipes.version
        ));
    }
    Ok(swipes)
}

/// Record `dir` for `id` at `at` (replacing an earlier swipe) and write the file.
pub fn set(user_data: &Path, id: u64, dir: Dir, at: String) -> Result<Swipe, String> {
    let mut swipes = read(user_data)?;
    let swipe = Swipe { dir, at };
    swipes.swipes.insert(id, swipe.clone());
    write(user_data, &swipes)?;
    Ok(swipe)
}

/// Remove `id`'s swipe; returns whether there was one (the file is only
/// written when it changes).
pub fn undo(user_data: &Path, id: u64) -> Result<bool, String> {
    let mut swipes = read(user_data)?;
    if swipes.swipes.remove(&id).is_none() {
        return Ok(false);
    }
    write(user_data, &swipes)?;
    Ok(true)
}

fn write(user_data: &Path, swipes: &Swipes) -> Result<(), String> {
    video_tags::store::write_json_atomic(user_data, FILE, BACKUP, swipes).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn temp(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("scout-report-swipes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn on_disk(dir: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(dir.join(FILE)).unwrap()).unwrap()
    }

    #[test]
    fn swipe_and_undo_round_trip() {
        let dir = temp("round-trip");
        assert_eq!(read(&dir).unwrap(), Swipes::default());

        set(&dir, 343669, Dir::Right, "2026-11-13T15:00:00.000Z".into()).unwrap();
        set(&dir, 12, Dir::Left, "2026-11-13T15:00:01.000Z".into()).unwrap();
        assert_eq!(
            on_disk(&dir),
            json!({"version": 1, "swipes": {
                "12": {"dir": "left", "at": "2026-11-13T15:00:01.000Z"},
                "343669": {"dir": "right", "at": "2026-11-13T15:00:00.000Z"}
            }})
        );
        let s = read(&dir).unwrap();
        assert_eq!(
            (s.dir(343669), s.dir(12), s.dir(7)),
            (Some(Dir::Right), Some(Dir::Left), None)
        );

        // A second swipe replaces the first.
        set(&dir, 12, Dir::Right, "2026-11-13T16:00:00.000Z".into()).unwrap();
        assert_eq!(read(&dir).unwrap().dir(12), Some(Dir::Right));

        assert!(undo(&dir, 12).unwrap());
        assert!(!undo(&dir, 12).unwrap());
        assert_eq!(
            on_disk(&dir),
            json!({"version": 1, "swipes": {
                "343669": {"dir": "right", "at": "2026-11-13T15:00:00.000Z"}
            }})
        );
        assert!(dir.join(BACKUP).is_file());
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_without_a_file_writes_nothing() {
        let dir = temp("undo-empty");
        assert!(!undo(&dir, 5).unwrap());
        assert!(!dir.join(FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_file_is_an_error_and_never_overwritten() {
        let dir = temp("broken");
        for bad in [
            &b"{not json"[..],
            br#"{"version": 2, "swipes": {}}"#,
            br#"{"version": 1, "swipes": {"9": {"dir": "up", "at": "x"}}}"#,
        ] {
            std::fs::write(dir.join(FILE), bad).unwrap();
            let e = read(&dir).unwrap_err();
            assert!(e.starts_with(FILE), "{e}");
            assert!(set(&dir, 1, Dir::Right, "x".into()).is_err());
            assert!(undo(&dir, 9).is_err());
            assert_eq!(std::fs::read(dir.join(FILE)).unwrap(), bad);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_directions() {
        assert_eq!(Dir::parse("right"), Some(Dir::Right));
        assert_eq!(Dir::parse("left"), Some(Dir::Left));
        assert_eq!(Dir::parse("Right"), None);
        assert_eq!(Dir::parse(""), None);
    }
}
