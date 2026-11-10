//! What the CLI needs from the app's `config.json`, read only: the library
//! sources (`settings.library.sources[].path`) and the backend port. Errors
//! never quote the file: it holds the EroScripts session cookie.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_BACKEND_PORT: u16 = 5123;

#[derive(Debug, PartialEq)]
pub struct AppConfig {
    pub sources: Vec<PathBuf>,
    pub backend_port: u16,
}

pub fn read(user_data: &Path) -> Result<AppConfig, String> {
    from_json(user_data, &read_json(user_data)?)
}

/// `config.json` as JSON, for callers that need more of it than
/// [`AppConfig`] (pass it to [`from_json`] for the sources and port).
pub fn read_json(user_data: &Path) -> Result<Value, String> {
    let path = user_data.join("config.json");
    let raw = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| {
        format!(
            "{} is not valid JSON (line {}, column {})",
            path.display(),
            e.line(),
            e.column()
        )
    })
}

/// The sources and port of a `config.json` read with [`read_json`].
pub fn from_json(user_data: &Path, v: &Value) -> Result<AppConfig, String> {
    parse(v).ok_or_else(|| {
        format!(
            "{} lists no library sources (settings.library.sources)",
            user_data.join("config.json").display()
        )
    })
}

/// Settings live under `settings` (electron-conf); a bare settings object is
/// accepted too.
fn parse(v: &Value) -> Option<AppConfig> {
    let settings = v.get("settings").unwrap_or(v);
    let sources: Vec<PathBuf> = settings
        .pointer("/library/sources")?
        .as_array()?
        .iter()
        .filter_map(|s| s.get("path")?.as_str())
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .collect();
    if sources.is_empty() {
        return None;
    }
    let backend_port = settings
        .pointer("/backend/port")
        .and_then(Value::as_u64)
        .and_then(|p| u16::try_from(p).ok())
        .filter(|&p| p != 0)
        .unwrap_or(DEFAULT_BACKEND_PORT);
    Some(AppConfig {
        sources,
        backend_port,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_sources_and_port() {
        let v = json!({ "settings": {
            "library": { "sources": [{ "path": "/V/EroScripts", "enabled": true }, { "path": "" }, { "name": "x" }] },
            "backend": { "port": 5200 },
            "eroscripts": { "session": { "cookie": "secret" } },
        } });
        assert_eq!(
            parse(&v),
            Some(AppConfig {
                sources: vec![PathBuf::from("/V/EroScripts")],
                backend_port: 5200
            })
        );
    }

    #[test]
    fn defaults_the_port_and_accepts_bare_settings() {
        let v = json!({ "library": { "sources": [{ "path": "/V/Other" }] } });
        assert_eq!(parse(&v).unwrap().backend_port, DEFAULT_BACKEND_PORT);
        assert_eq!(parse(&json!({ "settings": { "library": {} } })), None);
    }
}
