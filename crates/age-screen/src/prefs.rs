//! The screener's preferences, from the shared part of the content
//! preferences (`renderer/js/content-preferences.json`, also exported by
//! `content-preferences.js`). Embedded at compile time, so `cargo run`
//! rebuilds when the file changes; no rule constant lives in Rust.

use serde::Deserialize;

const SHARED: &str = include_str!("../../../renderer/js/content-preferences.json");

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgeScreen {
    /// `p_minor` at or above this: purge (1★), favourites to the playlist.
    pub purge_at: f64,
    /// `p_minor` at or above this (and below `purge_at`): the playlist.
    pub review_at: f64,
    /// The review playlist's name (created by name when missing).
    pub playlist: String,
    /// `--weekly` runs when the last complete run is at least this old.
    pub interval_days: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ratings {
    /// Rated this or lower (and above 0): the app deletes the video.
    pub delete_at_or_below: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prefs {
    pub age_screen: AgeScreen,
    pub ratings: Ratings,
}

/// The embedded preferences.
pub fn load() -> Result<Prefs, String> {
    parse(SHARED)
}

pub fn parse(raw: &str) -> Result<Prefs, String> {
    let prefs: Prefs = serde_json::from_str(raw)
        .map_err(|e| format!("renderer/js/content-preferences.json: {e}"))?;
    let a = &prefs.age_screen;
    let unit = |v: f64| (0.0..=1.0).contains(&v);
    if !(unit(a.review_at) && unit(a.purge_at) && a.review_at <= a.purge_at) {
        return Err(
            "content-preferences.json: ageScreen needs 0 ≤ reviewAt ≤ purgeAt ≤ 1".to_string(),
        );
    }
    if a.playlist.trim().is_empty() {
        return Err("content-preferences.json: ageScreen.playlist is empty".to_string());
    }
    if !(a.interval_days.is_finite() && a.interval_days > 0.0) {
        return Err("content-preferences.json: ageScreen.intervalDays must be above 0".to_string());
    }
    Ok(prefs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_file_parses() {
        let p = load().unwrap();
        assert!(p.age_screen.review_at <= p.age_screen.purge_at);
        assert!(!p.age_screen.playlist.is_empty());
        assert!(p.ratings.delete_at_or_below > 0.0);
    }

    #[test]
    fn rejects_inverted_thresholds() {
        let raw = r#"{"ageScreen": {"purgeAt": 0.4, "reviewAt": 0.5, "playlist": "x", "intervalDays": 7},
                      "ratings": {"deleteAtOrBelow": 2}}"#;
        assert!(parse(raw).is_err());
    }
}
