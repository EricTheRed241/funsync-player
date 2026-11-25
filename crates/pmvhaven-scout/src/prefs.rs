//! `pmvhavenScout` in the shared preferences
//! (`shared/content-preferences.json`): the PMVHaven-only words and
//! scales. Everything the other scouts share (screening, ranking, tag
//! scores, favourites, the library cap) comes through `slr_scout::prefs`.
//! Embedded at compile time; no value lives in Rust.

use serde::Deserialize;

const SHARED: &str = include_str!("../../../shared/content-preferences.json");

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PmvhavenScout {
    /// PMVHaven likes per likes point (its scale for RANKING's likes term).
    pub popularity_per: f64,
    /// A title or tag word saying the video is a PMV (sets `pmv`).
    pub pmv_words: Vec<String>,
    /// A title word saying it is a compilation.
    pub compilation_words: Vec<String>,
    /// A title or tag word saying it is an audio piece, not a video.
    pub audio_words: Vec<String>,
    /// A title or tag word saying a 2:1 or 1:1 frame is stereo VR.
    pub vr_words: Vec<String>,
    /// A community script's description saying it was generated.
    pub generated_script_words: Vec<String>,
    /// A community script's description saying it is for another device.
    pub other_device_script_words: Vec<String>,
    /// A cached video detail is read again after this many days.
    pub detail_max_age_days: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Wrapper {
    pmvhaven_scout: PmvhavenScout,
}

/// The embedded `pmvhavenScout`.
pub fn load() -> Result<PmvhavenScout, String> {
    parse(SHARED)
}

pub fn parse(raw: &str) -> Result<PmvhavenScout, String> {
    let p = serde_json::from_str::<Wrapper>(raw)
        .map_err(|e| format!("shared/content-preferences.json: {e}"))?
        .pmvhaven_scout;
    if p.popularity_per <= 0.0 {
        return Err("content-preferences.json: pmvhavenScout.popularityPer must be above 0".into());
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_prefs_load() {
        let p = load().unwrap();
        assert!(p.popularity_per > 0.0 && p.detail_max_age_days > 0);
        assert!(p.pmv_words.iter().any(|w| w == "pmv"));
        assert!(!p.vr_words.is_empty() && !p.audio_words.is_empty());
    }

    #[test]
    fn rejects_bad_values() {
        let mut v: serde_json::Value = serde_json::from_str(SHARED).unwrap();
        v["pmvhavenScout"]["popularityPer"] = 0.into();
        assert!(parse(&v.to_string()).is_err());
        assert!(parse("{}").is_err());
    }
}
