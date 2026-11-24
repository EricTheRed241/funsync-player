//! `<userData>/creator-scout-state.json` (this CLI's own bookkeeping: when
//! each creator's pictures were fetched, creators the judge named without a
//! verifiable account, people and creators looked up without enough found,
//! posts waiting for their details, fetch tries, Instagram follows, posts
//! without a picture to fill, EroScripts posts without a script to time)
//! and `creator-swipes.json` (written by the Creators site, read here).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

pub const FILE_NAME: &str = "creator-scout-state.json";
pub const BACKUP_NAME: &str = "creator-scout-state.json.bak";
pub const SWIPES_NAME: &str = "creator-swipes.json";
pub const VERSION: u64 = 1;
/// Fetch attempts per post before it is given up.
pub const MAX_TRIES: u32 = 5;
/// Hours between fetch tries of a post: 5 tries span about two days.
pub const RETRY_HOURS: i64 = 12;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepRun {
    pub at: String,
    pub code: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Unverified {
    pub name: String,
    pub at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchRecord {
    #[serde(default)]
    pub tries: u32,
    #[serde(default)]
    pub last_try: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    /// Set once the post is in the library.
    #[serde(default)]
    pub done_at: Option<String>,
    /// An EroScripts post: set once `eroscripts:scout --add` put it on the waiting list, from
    /// where the hourly `eroscripts:scout --fetch --auto` downloads it (no more tries here).
    #[serde(default)]
    pub handed_at: Option<String>,
}

/// A follow of a creator's Instagram account (`--follow`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Follow {
    pub handle: String,
    pub at: String,
    /// following / requested (a private account) / failed.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    #[serde(default = "version")]
    pub version: u64,
    /// Step name → its last run.
    #[serde(default)]
    pub last_runs: BTreeMap<String, StepRun>,
    /// Creator slug → when her Instagram pictures were last fetched.
    #[serde(default)]
    pub pictures: BTreeMap<String, String>,
    /// Creator slug → named by the judge, no account found (not searched
    /// again for 30 days).
    #[serde(default)]
    pub unverified: BTreeMap<String, Unverified>,
    /// EroScripts topic id → creator slug: posts added from the feed whose
    /// details (hosts, image) couldn't be read yet.
    #[serde(default)]
    pub pending_details: BTreeMap<String, String>,
    /// Post key (`eroscripts:41415`) → fetch attempts.
    #[serde(default)]
    pub fetch: BTreeMap<String, FetchRecord>,
    /// Person or creator slug → when `--profiles` last found nothing
    /// confirmed hers, or still too few pictures (looked up again after 30
    /// days).
    #[serde(default)]
    pub profile_misses: BTreeMap<String, String>,
    /// Creator slug → her Instagram follow (`--follow`; never undone).
    #[serde(default)]
    pub follows: BTreeMap<String, Follow>,
    /// Post key (`faptap:21`) → when `--post-images` last found no picture
    /// for it or couldn't download it (tried again after 7 days).
    #[serde(default)]
    pub post_image_misses: BTreeMap<String, String>,
    /// Post key (`eroscripts:41415`) → when `--runtimes` last found no
    /// readable stroke script for it (tried again after 7 days).
    #[serde(default)]
    pub runtime_misses: BTreeMap<String, String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn version() -> u64 {
    VERSION
}

pub fn read(user_data: &Path) -> Result<State, String> {
    let path = user_data.join(FILE_NAME);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(State {
                version: VERSION,
                ..State::default()
            });
        }
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    serde_json::from_str(&raw).map_err(|e| {
        format!(
            "{} is not a creator-scout state file ({e}); move it aside and re-run",
            path.display()
        )
    })
}

pub fn write(user_data: &Path, state: &State) -> Result<(), String> {
    video_tags::store::write_json_atomic(user_data, FILE_NAME, BACKUP_NAME, state).map(|_| ())
}

/// The Creators site's swipes: creator slug → choice, post key → choice.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Swipes {
    pub creators: BTreeMap<String, String>,
    pub posts: BTreeMap<String, String>,
}

impl Swipes {
    pub fn creator_is_no(&self, slug: &str) -> bool {
        self.creators.get(slug).is_some_and(|c| c == "no")
    }
}

/// `creator-swipes.json`; missing reads as empty.
pub fn read_swipes(user_data: &Path) -> Result<Swipes, String> {
    let path = user_data.join(SWIPES_NAME);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Swipes::default()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let v: Value =
        serde_json::from_str(&raw).map_err(|e| format!("{}: not JSON ({e})", path.display()))?;
    let choices = |key: &str| -> BTreeMap<String, String> {
        v[key]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, e)| Some((k.clone(), e["choice"].as_str()?.to_string())))
            .collect()
    };
    Ok(Swipes {
        creators: choices("creators"),
        posts: choices("posts"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_swipes() {
        let dir = std::env::temp_dir().join(format!("creator-scout-state-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_swipes(&dir).unwrap(), Swipes::default());
        fs::write(
            dir.join(SWIPES_NAME),
            r#"{"version":1,"creators":{"a":{"choice":"no","at":"x"},"b":{"choice":"yes"}},
               "posts":{"eroscripts:1":{"choice":"want","creator":"b"},"faptap:2":{"at":"x"}}}"#,
        )
        .unwrap();
        let s = read_swipes(&dir).unwrap();
        assert!(s.creator_is_no("a"));
        assert!(!s.creator_is_no("b"));
        assert_eq!(s.posts.len(), 1);
        assert_eq!(read(&dir).unwrap().version, VERSION);
        let _ = fs::remove_dir_all(&dir);
    }
}
