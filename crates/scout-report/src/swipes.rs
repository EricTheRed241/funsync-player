//! The deck's verdicts, the only files the CLI writes: one per source.
//!
//! - `<userData>/eroscripts-scout-swipes.json` (EROSCRIPTS): right = wanted (the scout's
//!   `--fetch --auto` downloads the post whatever its score or axes), left = hidden from the
//!   deck only (the scout ignores it). By forum topic id.
//! - `slr-scout-swipes.json` (SLR), `vrb-scout-swipes.json` (VRB), `pmvhaven-scout-swipes.json`
//!   (PMVHAVEN) and `faptap-scout-swipes.json` (FAPTAP): the same verdicts on the scenes of that
//!   scout's file, by the site's own scene id; each scout's `--fetch` downloads its right swipes.
//!
//! Ids are string keys here, as in the file: SLR and topic ids are numbers, VRBangers and
//! PMVHaven ids hex strings, FapTap ids too long for a JavaScript number. Undo removes the entry.
//!
//! ```json
//! { "version": 1, "swipes": { "343669": { "dir": "right", "at": "2026-11-13T15:00:00.000Z" } } }
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

pub const VERSION: u32 = 1;
/// Longest scene id accepted (a FapTap id is 19 digits, a VRBangers one 24 hex digits).
pub const MAX_ID_LEN: usize = 64;

/// One source's swipes file and the previous copy every write keeps
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

/// slr-scout.json's scenes, by SLR scene id.
pub const SLR: Store = Store {
    file: "slr-scout-swipes.json",
    backup: "slr-scout-swipes.json.bak",
};

/// vrb-scout.json's scenes (VRBangers and VRConk), by the site's scene id.
pub const VRB: Store = Store {
    file: "vrb-scout-swipes.json",
    backup: "vrb-scout-swipes.json.bak",
};

/// pmvhaven-scout.json's videos, by PMVHaven video id.
pub const PMVHAVEN: Store = Store {
    file: "pmvhaven-scout-swipes.json",
    backup: "pmvhaven-scout-swipes.json.bak",
};

/// faptap-scout.json's candidates, by FapTap video id.
pub const FAPTAP: Store = Store {
    file: "faptap-scout-swipes.json",
    backup: "faptap-scout-swipes.json.bak",
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
    /// By topic or scene id.
    #[serde(default)]
    pub swipes: BTreeMap<String, Swipe>,
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
    pub fn dir(&self, id: &str) -> Option<Dir> {
        self.swipes.get(id).map(|s| s.dir)
    }
}

/// A scene id as its swipes key: a whole number in plain digits, or a string of 1 to
/// MAX_ID_LEN letters, digits, `-` and `_` (never `:` or `,`, which the deck's `exclude`
/// list uses as separators). Anything else is no id.
pub fn id_key(v: &Value) -> Option<String> {
    match v {
        Value::Number(n) => n.as_u64().map(|n| n.to_string()),
        Value::String(s) => is_id(s).then(|| s.clone()),
        _ => None,
    }
}

/// Whether `s` may be a scene id (id_key's string rule).
pub fn is_id(s: &str) -> bool {
    (1..=MAX_ID_LEN).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
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
    pub fn set(self, user_data: &Path, id: &str, dir: Dir, at: String) -> Result<Swipe, String> {
        let mut swipes = self.read(user_data)?;
        let swipe = Swipe { dir, at };
        swipes.swipes.insert(id.to_string(), swipe.clone());
        self.write(user_data, &swipes)?;
        Ok(swipe)
    }

    /// Remove `id`'s swipe; returns whether there was one (the file is only
    /// written when it changes).
    pub fn undo(self, user_data: &Path, id: &str) -> Result<bool, String> {
        let mut swipes = self.read(user_data)?;
        if swipes.swipes.remove(id).is_none() {
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
    use serde_json::json;

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

        s.set(
            &dir,
            "343669",
            Dir::Right,
            "2026-11-13T15:00:00.000Z".into(),
        )
        .unwrap();
        s.set(&dir, "12", Dir::Left, "2026-11-13T15:00:01.000Z".into())
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
            (r.dir("343669"), r.dir("12"), r.dir("7")),
            (Some(Dir::Right), Some(Dir::Left), None)
        );

        // A second swipe replaces the first.
        s.set(&dir, "12", Dir::Right, "2026-11-13T16:00:00.000Z".into())
            .unwrap();
        assert_eq!(s.read(&dir).unwrap().dir("12"), Some(Dir::Right));

        assert!(s.undo(&dir, "12").unwrap());
        assert!(!s.undo(&dir, "12").unwrap());
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
    fn an_existing_numeric_file_round_trips_with_string_ids_added() {
        // An slr-scout-swipes.json written when ids were numbers: the same keys come back,
        // and a string id lands beside them in the same format.
        let dir = temp("numeric");
        let before = json!({"version": 1, "swipes": {
            "66249": {"dir": "right", "at": "2026-11-13T15:00:00.000Z"},
            "9": {"dir": "left", "at": "2026-11-13T15:00:01.000Z"}
        }});
        std::fs::write(dir.join(SLR.file), before.to_string()).unwrap();
        let r = SLR.read(&dir).unwrap();
        assert_eq!(
            (r.dir("66249"), r.dir("9"), r.dir("09")),
            (Some(Dir::Right), Some(Dir::Left), None)
        );
        SLR.set(&dir, "6accfc2cff316f0c39086f52", Dir::Right, "x".into())
            .unwrap();
        assert!(SLR.undo(&dir, "6accfc2cff316f0c39086f52").unwrap());
        assert_eq!(on_disk(&dir, SLR), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_sources_keep_separate_files() {
        let dir = temp("separate");
        // The same number is a forum topic in one file and a scene in the others.
        EROSCRIPTS
            .set(&dir, "85666", Dir::Left, "x".into())
            .unwrap();
        SLR.set(&dir, "85666", Dir::Right, "y".into()).unwrap();
        FAPTAP.set(&dir, "85666", Dir::Left, "z".into()).unwrap();
        assert_eq!(EROSCRIPTS.read(&dir).unwrap().dir("85666"), Some(Dir::Left));
        assert_eq!(SLR.read(&dir).unwrap().dir("85666"), Some(Dir::Right));
        assert!(SLR.undo(&dir, "85666").unwrap());
        assert_eq!(EROSCRIPTS.read(&dir).unwrap().dir("85666"), Some(Dir::Left));
        assert_eq!(FAPTAP.read(&dir).unwrap().dir("85666"), Some(Dir::Left));
        assert!(VRB.read(&dir).unwrap().swipes.is_empty());
        assert!(PMVHAVEN.read(&dir).unwrap().swipes.is_empty());
        let files: std::collections::HashSet<_> = [EROSCRIPTS, SLR, VRB, PMVHAVEN, FAPTAP]
            .iter()
            .map(|s| s.file)
            .collect();
        assert_eq!(files.len(), 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_without_a_file_writes_nothing() {
        let dir = temp("undo-empty");
        assert!(!EROSCRIPTS.undo(&dir, "5").unwrap());
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
            assert!(s.set(&dir, "1", Dir::Right, "x".into()).is_err());
            assert!(s.undo(&dir, "9").is_err());
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

    #[test]
    fn scene_ids_as_keys() {
        assert_eq!(id_key(&json!(66249)), Some("66249".into()));
        assert_eq!(
            id_key(&json!("6accfc2cff316f0c39086f52")),
            Some("6accfc2cff316f0c39086f52".into())
        );
        assert_eq!(
            id_key(&json!("1957860091331874817")),
            Some("1957860091331874817".into())
        );
        assert_eq!(id_key(&json!("a_b-C")), Some("a_b-C".into()));
        for bad in [
            json!(-1),
            json!(1.5),
            json!(""),
            json!("a:b"),
            json!("a,b"),
            json!("a b"),
            json!("../x"),
            json!("x".repeat(MAX_ID_LEN + 1)),
            json!(null),
            json!(true),
            json!([1]),
        ] {
            assert_eq!(id_key(&bad), None, "{bad}");
        }
        assert!(is_id(&"x".repeat(MAX_ID_LEN)));
    }
}
