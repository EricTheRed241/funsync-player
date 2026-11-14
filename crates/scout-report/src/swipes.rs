//! The decks' verdicts, the only files the CLI writes: one per deck.
//!
//! - `<userData>/eroscripts-scout-swipes.json` (EROSCRIPTS): right = wanted (the scout's
//!   `--fetch --auto` downloads the post whatever its score or axes), left = hidden from the
//!   deck only (the scout ignores it). By forum topic id.
//! - `<userData>/slr-scout-swipes.json` (SLR): the same verdicts on SexLikeReal scenes from
//!   `slr-scout.json`, by SLR scene id (a pick list: nothing downloads from SLR yet).
//!
//! Undo removes the entry.
//!
//! ```json
//! { "version": 1, "swipes": { "343669": { "dir": "right", "at": "2026-11-13T15:00:00.000Z" } } }
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const VERSION: u32 = 1;

/// One deck's swipes file and the previous copy every write keeps
/// (`video_tags::store::write_json_atomic`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Store {
    pub file: &'static str,
    pub backup: &'static str,
}

/// The EroScripts waiting list's swipes, by topic id.
pub const EROSCRIPTS: Store = Store {
    file: "eroscripts-scout-swipes.json",
    backup: "eroscripts-scout-swipes.json.bak",
};

/// The SLR deck's swipes, by SLR scene id.
pub const SLR: Store = Store {
    file: "slr-scout-swipes.json",
    backup: "slr-scout-swipes.json.bak",
};

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
    /// By topic or scene id (a string key in the file).
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

impl Store {
    /// Read the file; a missing one is empty. An unreadable, unparsable or
    /// other-version file is an error naming it (the caller reports it as a
    /// problem and never writes over it).
    pub fn read(self, user_data: &Path) -> Result<Swipes, String> {
        let file = self.file;
        let bytes = match std::fs::read(user_data.join(file)) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Swipes::default()),
            Err(e) => return Err(format!("{file}: {e}")),
        };
        let swipes: Swipes = serde_json::from_slice(&bytes).map_err(|e| format!("{file}: {e}"))?;
        if swipes.version != VERSION {
            return Err(format!(
                "{file}: version {}, this CLI reads version {VERSION}",
                swipes.version
            ));
        }
        Ok(swipes)
    }

    /// Record `dir` for `id` at `at` (replacing an earlier swipe) and write the file.
    pub fn set(self, user_data: &Path, id: u64, dir: Dir, at: String) -> Result<Swipe, String> {
        let mut swipes = self.read(user_data)?;
        let swipe = Swipe { dir, at };
        swipes.swipes.insert(id, swipe.clone());
        self.write(user_data, &swipes)?;
        Ok(swipe)
    }

    /// Remove `id`'s swipe; returns whether there was one (the file is only
    /// written when it changes).
    pub fn undo(self, user_data: &Path, id: u64) -> Result<bool, String> {
        let mut swipes = self.read(user_data)?;
        if swipes.swipes.remove(&id).is_none() {
            return Ok(false);
        }
        self.write(user_data, &swipes)?;
        Ok(true)
    }

    fn write(self, user_data: &Path, swipes: &Swipes) -> Result<(), String> {
        video_tags::store::write_json_atomic(user_data, self.file, self.backup, swipes).map(|_| ())
    }
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

    fn on_disk(dir: &Path, store: Store) -> Value {
        serde_json::from_slice(&std::fs::read(dir.join(store.file)).unwrap()).unwrap()
    }

    #[test]
    fn swipe_and_undo_round_trip() {
        let dir = temp("round-trip");
        let s = EROSCRIPTS;
        assert_eq!(s.read(&dir).unwrap(), Swipes::default());

        s.set(&dir, 343669, Dir::Right, "2026-11-13T15:00:00.000Z".into())
            .unwrap();
        s.set(&dir, 12, Dir::Left, "2026-11-13T15:00:01.000Z".into())
            .unwrap();
        assert_eq!(
            on_disk(&dir, s),
            json!({"version": 1, "swipes": {
                "12": {"dir": "left", "at": "2026-11-13T15:00:01.000Z"},
                "343669": {"dir": "right", "at": "2026-11-13T15:00:00.000Z"}
            }})
        );
        let r = s.read(&dir).unwrap();
        assert_eq!(
            (r.dir(343669), r.dir(12), r.dir(7)),
            (Some(Dir::Right), Some(Dir::Left), None)
        );

        // A second swipe replaces the first.
        s.set(&dir, 12, Dir::Right, "2026-11-13T16:00:00.000Z".into())
            .unwrap();
        assert_eq!(s.read(&dir).unwrap().dir(12), Some(Dir::Right));

        assert!(s.undo(&dir, 12).unwrap());
        assert!(!s.undo(&dir, 12).unwrap());
        assert_eq!(
            on_disk(&dir, s),
            json!({"version": 1, "swipes": {
                "343669": {"dir": "right", "at": "2026-11-13T15:00:00.000Z"}
            }})
        );
        assert!(dir.join(s.backup).is_file());
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_decks_keep_separate_files() {
        let dir = temp("separate");
        // The same number is a forum topic in one deck and an SLR scene in the other.
        EROSCRIPTS.set(&dir, 85666, Dir::Left, "x".into()).unwrap();
        SLR.set(&dir, 85666, Dir::Right, "y".into()).unwrap();
        assert_eq!(EROSCRIPTS.read(&dir).unwrap().dir(85666), Some(Dir::Left));
        assert_eq!(SLR.read(&dir).unwrap().dir(85666), Some(Dir::Right));
        assert!(SLR.undo(&dir, 85666).unwrap());
        assert_eq!(EROSCRIPTS.read(&dir).unwrap().dir(85666), Some(Dir::Left));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_without_a_file_writes_nothing() {
        let dir = temp("undo-empty");
        assert!(!EROSCRIPTS.undo(&dir, 5).unwrap());
        assert!(!dir.join(EROSCRIPTS.file).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_file_is_an_error_and_never_overwritten() {
        let dir = temp("broken");
        let s = EROSCRIPTS;
        for bad in [
            &b"{not json"[..],
            br#"{"version": 2, "swipes": {}}"#,
            br#"{"version": 1, "swipes": {"9": {"dir": "up", "at": "x"}}}"#,
        ] {
            std::fs::write(dir.join(s.file), bad).unwrap();
            let e = s.read(&dir).unwrap_err();
            assert!(e.starts_with(s.file), "{e}");
            assert!(s.set(&dir, 1, Dir::Right, "x".into()).is_err());
            assert!(s.undo(&dir, 9).is_err());
            assert_eq!(std::fs::read(dir.join(s.file)).unwrap(), bad);
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
