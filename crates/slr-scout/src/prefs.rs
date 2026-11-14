//! What to look for and how to score it: `slrScout` in the shared part of
//! the content preferences (`renderer/js/content-preferences.json`).
//! Embedded at compile time; no value lives in Rust.

use serde::Deserialize;

const SHARED: &str = include_str!("../../../renderer/js/content-preferences.json");

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bonuses {
    /// A hand-made script exists (SLR hand-crafted or an EroScripts post).
    pub handmade_script: f64,
    pub pmv: f64,
    pub compilation: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlrScout {
    /// Performer names as SLR spells them.
    pub actors: Vec<String>,
    /// Keep VR scenes only (drop flat-screen ones).
    pub vr_only: bool,
    /// Keep scenes with a creampie only.
    pub require_creampie: bool,
    pub bonuses: Bonuses,
    /// Favourites per popularity point...
    pub popularity_per: f64,
    /// ...up to this many points.
    pub popularity_max: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Shared {
    slr_scout: SlrScout,
}

/// The embedded preferences.
pub fn load() -> Result<SlrScout, String> {
    parse(SHARED)
}

pub fn parse(raw: &str) -> Result<SlrScout, String> {
    let prefs = serde_json::from_str::<Shared>(raw)
        .map_err(|e| format!("renderer/js/content-preferences.json: {e}"))?
        .slr_scout;
    let bad = |what: &str| Err(format!("content-preferences.json: slrScout: {what}"));
    if prefs.actors.iter().all(|a| a.trim().is_empty()) {
        return bad("actors lists nobody");
    }
    if prefs.popularity_per <= 0.0 {
        return bad("popularityPer must be above 0");
    }
    if prefs.popularity_max < 0.0 {
        return bad("popularityMax can't be negative");
    }
    Ok(prefs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_prefs_load() {
        let prefs = load().unwrap();
        assert!(!prefs.actors.is_empty());
        assert!(prefs.popularity_per > 0.0);
    }

    #[test]
    fn rejects_bad_values() {
        let wrap = |actors: &str, per: f64, max: f64| {
            format!(
                r#"{{"slrScout": {{"actors": {actors}, "vrOnly": true, "requireCreampie": true,
                "bonuses": {{"handmadeScript": 4, "pmv": 2, "compilation": 2}},
                "popularityPer": {per}, "popularityMax": {max}}}}}"#
            )
        };
        assert!(parse(&wrap(r#"["A"]"#, 1000.0, 2.0)).is_ok());
        assert!(parse(&wrap("[]", 1000.0, 2.0)).is_err());
        assert!(parse(&wrap(r#"[" "]"#, 1000.0, 2.0)).is_err());
        assert!(parse(&wrap(r#"["A"]"#, 0.0, 2.0)).is_err());
        assert!(parse(&wrap(r#"["A"]"#, 1000.0, -1.0)).is_err());
        assert!(parse("{}").is_err());
    }
}
