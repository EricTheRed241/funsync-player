//! Read only, the library sources from the app's `config.json`
//! (`settings.library.sources[].path`), used to name videos by their path
//! inside a source. Errors never quote the file: it holds the EroScripts
//! session cookie. (The userData default: `video_tags::host`.)

use serde_json::Value;
use std::fs;
use std::path::Path;

/// The library source folders; empty when `config.json` is missing or
/// lists none (titles then fall back to the last path segments).
pub fn sources(user_data: &Path) -> Result<Vec<String>, String> {
    let path = user_data.join("config.json");
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let v: Value = serde_json::from_str(&raw).map_err(|e| {
        format!(
            "{} is not valid JSON (line {}, column {})",
            path.display(),
            e.line(),
            e.column()
        )
    })?;
    Ok(parse_sources(&v))
}

/// Settings live under `settings` (electron-conf); a bare settings object is
/// accepted too.
fn parse_sources(v: &Value) -> Vec<String> {
    let settings = v.get("settings").unwrap_or(v);
    settings
        .pointer("/library/sources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| s.get("path")?.as_str())
        .filter(|p| !p.is_empty())
        .map(|p| p.trim_end_matches('/').to_string())
        .collect()
}

/// A video's name for the judge: its path inside the library source that
/// holds it (`EroScripts/<creator>/<scene>/<file>` minus the source), or the
/// last three path segments when no source holds it.
pub fn title(path: &str, sources: &[String]) -> String {
    let inside = sources
        .iter()
        .filter_map(|s| path.strip_prefix(s.as_str())?.strip_prefix('/'))
        .min_by_key(|rest| rest.len());
    match inside {
        Some(rest) if !rest.is_empty() => rest.to_string(),
        _ => {
            let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
            parts[parts.len().saturating_sub(3)..].join("/")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_source_paths() {
        let v = json!({ "settings": {
            "library": { "sources": [{ "path": "/V/EroScripts/" }, { "path": "" }, { "name": "x" }] },
            "eroscripts": { "session": { "cookie": "secret" } },
        } });
        assert_eq!(parse_sources(&v), vec!["/V/EroScripts".to_string()]);
        assert!(parse_sources(&json!({})).is_empty());
    }

    #[test]
    fn titles_are_paths_inside_the_source() {
        let sources = vec!["/V/GVP/Other".to_string(), "/V/GVP".to_string()];
        assert_eq!(title("/V/GVP/Other/A/S/s.mp4", &sources), "A/S/s.mp4");
        assert_eq!(title("/V/GVP/Others/A/s.mp4", &sources), "Others/A/s.mp4");
        assert_eq!(title("/elsewhere/x/y/A/s.mp4", &sources), "y/A/s.mp4");
    }
}
