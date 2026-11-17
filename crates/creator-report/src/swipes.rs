//! `<userData>/creator-swipes.json`, the only file the site writes (creator-scout
//! reads it: it downloads the `want` posts). Every change reads the file, changes
//! one entry and writes it back atomically (video_tags::store::write_json_atomic:
//! temp + rename, the previous copy kept as `creator-swipes.json.bak`); the server
//! answers one request at a time, so changes never interleave. Undo removes the entry.
//!
//! ```json
//! { "version": 1,
//!   "creators": { "sweetiefox": { "choice": "yes", "at": "…" } },
//!   "posts": { "eroscripts:41415": { "choice": "want", "at": "…", "creator": "sweetiefox" } } }
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const FILE: &str = "creator-swipes.json";
pub const BACKUP: &str = "creator-swipes.json.bak";
pub const VERSION: u32 = 1;

/// A creator's swipe: right = yes, left = no.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Choice {
    Yes,
    No,
}

/// A post's swipe: right = want (creator-scout downloads it), left = hide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PostChoice {
    Want,
    Hide,
}

impl Choice {
    pub fn parse(s: &str) -> Option<Choice> {
        match s {
            "yes" => Some(Choice::Yes),
            "no" => Some(Choice::No),
            _ => None,
        }
    }
}

impl PostChoice {
    pub fn parse(s: &str) -> Option<PostChoice> {
        match s {
            "want" => Some(PostChoice::Want),
            "hide" => Some(PostChoice::Hide),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatorSwipe {
    pub choice: Choice,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostSwipe {
    pub choice: PostChoice,
    pub at: String,
    /// The creator's slug.
    pub creator: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Swipes {
    pub version: u32,
    /// By creator slug.
    #[serde(default)]
    pub creators: BTreeMap<String, CreatorSwipe>,
    /// By `<kind>:<id>`.
    #[serde(default)]
    pub posts: BTreeMap<String, PostSwipe>,
}

impl Default for Swipes {
    fn default() -> Self {
        Swipes {
            version: VERSION,
            creators: BTreeMap::new(),
            posts: BTreeMap::new(),
        }
    }
}

impl Swipes {
    pub fn creator(&self, slug: &str) -> Option<Choice> {
        self.creators.get(slug).map(|s| s.choice)
    }
}

/// Read the file; a missing one is empty. An unreadable, unparsable or other-version
/// file is an error naming it (shown as a problem, and never written over).
pub fn read(user_data: &Path) -> Result<Swipes, String> {
    let bytes = match std::fs::read(user_data.join(FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Swipes::default()),
        Err(e) => return Err(format!("{FILE}: {e}")),
    };
    let swipes: Swipes = serde_json::from_slice(&bytes).map_err(|e| format!("{FILE}: {e}"))?;
    if swipes.version != VERSION {
        return Err(format!(
            "{FILE}: version {}, this site reads version {VERSION}",
            swipes.version
        ));
    }
    Ok(swipes)
}

/// Record `choice` for creator `slug` (replacing an earlier one) and write the file.
pub fn set_creator(
    user_data: &Path,
    slug: &str,
    choice: Choice,
    at: String,
) -> Result<CreatorSwipe, String> {
    let mut swipes = read(user_data)?;
    let swipe = CreatorSwipe { choice, at };
    swipes.creators.insert(slug.to_string(), swipe.clone());
    write(user_data, &swipes)?;
    Ok(swipe)
}

/// Record `choice` for post `key` of `creator` and write the file.
pub fn set_post(
    user_data: &Path,
    key: &str,
    choice: PostChoice,
    creator: &str,
    at: String,
) -> Result<PostSwipe, String> {
    let mut swipes = read(user_data)?;
    let swipe = PostSwipe {
        choice,
        at,
        creator: creator.to_string(),
    };
    swipes.posts.insert(key.to_string(), swipe.clone());
    write(user_data, &swipes)?;
    Ok(swipe)
}

/// Forget creator `slug`'s swipe; whether there was one (the file is written only then).
pub fn undo_creator(user_data: &Path, slug: &str) -> Result<bool, String> {
    let mut swipes = read(user_data)?;
    if swipes.creators.remove(slug).is_none() {
        return Ok(false);
    }
    write(user_data, &swipes)?;
    Ok(true)
}

/// Forget post `key`'s swipe; whether there was one.
pub fn undo_post(user_data: &Path, key: &str) -> Result<bool, String> {
    let mut swipes = read(user_data)?;
    if swipes.posts.remove(key).is_none() {
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
        let dir = std::env::temp_dir().join(format!(
            "creator-report-swipes-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn on_disk(dir: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(dir.join(FILE)).unwrap()).unwrap()
    }

    #[test]
    fn records_and_undoes_swipes_in_the_contract_shape() {
        let dir = temp("roundtrip");
        assert_eq!(read(&dir).unwrap(), Swipes::default());
        set_creator(&dir, "sweetiefox", Choice::Yes, "t1".into()).unwrap();
        set_post(
            &dir,
            "eroscripts:41415",
            PostChoice::Want,
            "sweetiefox",
            "t2".into(),
        )
        .unwrap();
        set_post(
            &dir,
            "faptap:9",
            PostChoice::Hide,
            "sweetiefox",
            "t3".into(),
        )
        .unwrap();
        assert_eq!(
            on_disk(&dir),
            json!({"version": 1,
                   "creators": {"sweetiefox": {"choice": "yes", "at": "t1"}},
                   "posts": {"eroscripts:41415": {"choice": "want", "at": "t2", "creator": "sweetiefox"},
                             "faptap:9": {"choice": "hide", "at": "t3", "creator": "sweetiefox"}}})
        );
        // A later swipe replaces the earlier; the previous copy is kept.
        set_creator(&dir, "sweetiefox", Choice::No, "t4".into()).unwrap();
        assert_eq!(read(&dir).unwrap().creator("sweetiefox"), Some(Choice::No));
        assert!(dir.join(BACKUP).is_file());

        assert!(undo_creator(&dir, "sweetiefox").unwrap());
        assert!(!undo_creator(&dir, "sweetiefox").unwrap());
        assert!(undo_post(&dir, "faptap:9").unwrap());
        assert!(!undo_post(&dir, "faptap:9").unwrap());
        let s = read(&dir).unwrap();
        assert!(s.creators.is_empty());
        assert_eq!(s.posts.keys().collect::<Vec<_>>(), ["eroscripts:41415"]);
        // No temp file left behind.
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(names.iter().all(|n| n == FILE || n == BACKUP), "{names:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_broken_or_newer_file_is_never_written_over() {
        let dir = temp("broken");
        std::fs::write(dir.join(FILE), "{oops").unwrap();
        assert!(read(&dir).unwrap_err().starts_with(FILE));
        assert!(set_creator(&dir, "a", Choice::Yes, "t".into()).is_err());
        assert_eq!(std::fs::read(dir.join(FILE)).unwrap(), b"{oops");
        std::fs::write(dir.join(FILE), r#"{"version":2}"#).unwrap();
        assert!(read(&dir).unwrap_err().contains("version 2"));
        assert!(undo_post(&dir, "x").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_choices() {
        assert_eq!(Choice::parse("yes"), Some(Choice::Yes));
        assert_eq!(Choice::parse("no"), Some(Choice::No));
        assert_eq!(Choice::parse("want"), None);
        assert_eq!(PostChoice::parse("want"), Some(PostChoice::Want));
        assert_eq!(PostChoice::parse("hide"), Some(PostChoice::Hide));
        assert_eq!(PostChoice::parse("yes"), None);
    }
}
