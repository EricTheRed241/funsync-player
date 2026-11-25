//! The Rust app's own data folder (`~/Library/Application Support/funsync-rust`)
//! and its `library.json`. Never the old app's folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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

/// What the server owns. `config` has the old app's `config.json` shape
/// (`settings.library`, `settings.player`, device settings, `playlists`,
/// `categories`, `videoCategories`), so every rule ported from the old app
/// reads the same keys, and a takeover is a copy.
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
    pub config: Value,
}

impl Library {
    /// `settings.library`, the old app's library state.
    pub fn lib(&self) -> &Value {
        &self.config["settings"]["library"]
    }

    /// Mutable `settings.library.<key>`, created as `{}` when missing.
    pub fn lib_mut(&mut self, key: &str) -> &mut Value {
        self.section_mut("library")
            .entry(key)
            .or_insert_with(|| json!({}))
    }

    /// Mutable `settings.<name>` object, created when missing.
    pub fn section_mut(&mut self, name: &str) -> &mut serde_json::Map<String, Value> {
        ensure_object(&mut self.config);
        let settings = self
            .config
            .as_object_mut()
            .unwrap()
            .entry("settings")
            .or_insert_with(|| json!({}));
        ensure_object(settings);
        let s = settings
            .as_object_mut()
            .unwrap()
            .entry(name)
            .or_insert_with(|| json!({}));
        ensure_object(s);
        s.as_object_mut().unwrap()
    }

    /// A top-level array (`playlists`, `categories`), created when missing.
    pub fn top_array_mut(&mut self, key: &str) -> &mut Vec<Value> {
        ensure_object(&mut self.config);
        let v = self
            .config
            .as_object_mut()
            .unwrap()
            .entry(key)
            .or_insert_with(|| json!([]));
        if !v.is_array() {
            *v = json!([]);
        }
        v.as_array_mut().unwrap()
    }

    pub fn sources(&self) -> Vec<SourceEntry> {
        serde_json::from_value(self.lib()["sources"].clone()).unwrap_or_default()
    }
}

fn ensure_object(v: &mut Value) {
    if !v.is_object() {
        *v = json!({});
    }
}

/// Whether this server may change the library files (triage and duplicate
/// deletes, script downloads): yes, unless `FUNSYNC_OWNS_LIBRARY=0`.
pub fn owns_library() -> bool {
    std::env::var("FUNSYNC_OWNS_LIBRARY").map_or(true, |v| v != "0")
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
/// Each write has its own temp file: concurrent saves must not rename
/// each other's.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp-{}-{n}", std::process::id()));
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
