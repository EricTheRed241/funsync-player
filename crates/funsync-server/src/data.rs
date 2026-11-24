//! The Rust app's own data folder (`~/Library/Application Support/funsync-rust`)
//! and its `library.json`. Never the old app's folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const LIBRARY_FILE: &str = "library.json";
pub const SCAN_FILE: &str = "scan.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceEntry {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub path: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// What the server owns. The imported parts of the old app are kept as
/// they were (`playlists`, `categories`, `videoCategories`, `oldLibrary` =
/// the old `settings.library`) until phase 3 gives them a model.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub imported_from: Option<String>,
    #[serde(default)]
    pub imported_at: Option<u64>,
    #[serde(default)]
    pub sources: Vec<SourceEntry>,
    #[serde(default)]
    pub playlists: Value,
    #[serde(default)]
    pub categories: Value,
    #[serde(default)]
    pub video_categories: Value,
    #[serde(default)]
    pub old_library: Value,
}

pub fn default_data_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("FUNSYNC_RUST_DATA").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join("Library/Application Support/funsync-rust"))
}

/// The old app's data folder (read-only source of the import).
pub fn default_old_user_data() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("FUNSYNC_USER_DATA").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join("Library/Application Support/funsync-player"))
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Write via a temp file and rename, so a reader never sees half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

pub fn load_library(dir: &Path) -> Result<Library, String> {
    let p = dir.join(LIBRARY_FILE);
    match std::fs::read(&p) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| format!("{}: {e}", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Library::default()),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

pub fn save_library(dir: &Path, lib: &Library) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(lib).map_err(|e| e.to_string())?;
    write_atomic(&dir.join(LIBRARY_FILE), &bytes).map_err(|e| e.to_string())
}
