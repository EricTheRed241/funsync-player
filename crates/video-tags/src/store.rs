//! `<userData>/video-tags.json`: video path → tags. Written only by this CLI
//! (temp file + rename, previous file kept as `video-tags.json.bak`), read
//! by the app on every library scan (`electron/video-tags.js`).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "video-tags.json";
pub const BACKUP_NAME: &str = "video-tags.json.bak";
pub const VERSION: u64 = 1;

/// One video's tags.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub tags: Vec<String>,
    /// When the current tags were set.
    #[serde(default)]
    pub added_at: String,
    /// Who proposed them (the proposals file's `by`), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// Fields this version doesn't know, kept as they are.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The whole file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagFile {
    pub version: u64,
    #[serde(default)]
    pub updated_at: Option<String>,
    /// Keyed by absolute video path in the filesystem's own spelling.
    #[serde(default)]
    pub videos: BTreeMap<String, Entry>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl TagFile {
    pub fn empty() -> Self {
        TagFile {
            version: VERSION,
            updated_at: None,
            videos: BTreeMap::new(),
            extra: Map::new(),
        }
    }
}

pub fn file_path(user_data: &Path) -> PathBuf {
    user_data.join(FILE_NAME)
}

/// Read the file: `Ok(None)` when it doesn't exist; an error when it can't
/// be read, isn't JSON or is another version (never overwrite what we don't
/// understand).
pub fn read(path: &Path) -> Result<Option<TagFile>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let file: TagFile = serde_json::from_str(&raw).map_err(|e| {
        format!(
            "{} is not a video tags file ({e}); move it aside and re-run",
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

/// Write the file atomically: a temp file in the same directory, renamed
/// over the target, so the app never reads half a file. An existing file is
/// copied to `video-tags.json.bak` first. Returns whether a backup was made.
pub fn write_atomic(user_data: &Path, file: &TagFile) -> Result<bool, String> {
    let target = file_path(user_data);
    let tmp = user_data.join(format!("{FILE_NAME}.{}.tmp", std::process::id()));
    let result = (|| -> io::Result<bool> {
        let mut json = serde_json::to_string_pretty(file).map_err(io::Error::other)?;
        json.push('\n');
        let mut out = fs::File::create(&tmp)?;
        out.write_all(json.as_bytes())?;
        out.sync_all()?;
        drop(out);
        let backed_up = match fs::copy(&target, user_data.join(BACKUP_NAME)) {
            Ok(_) => true,
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(e) => return Err(e),
        };
        fs::rename(&tmp, &target)?;
        Ok(backed_up)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|e| format!("writing {}: {e}", target.display()))
}
