//! What to look for and how to score it, from the shared part of the
//! content preferences (`renderer/js/content-preferences.json`): `slrScout`
//! (who to scout, the SLR-only extras), and the values the EroScripts deck
//! scores with too (`ranking`, `tagScoreGroups`, `favourites`), the library
//! cap (`libraryMaxVideos`), the video caps (`videoCaps`) and the pornstar
//! list (`pornstarTags.pornstars`). Embedded at compile time; no value lives
//! in Rust.

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
    /// Performer names as SLR spells them, always scouted.
    pub actors: Vec<String>,
    /// Also scout the favourite artists (FAVOURITES with the user's changes).
    pub include_favourites: bool,
    /// Also scout `pornstarTags.pornstars`.
    pub include_pornstars: bool,
    /// Keep VR scenes only (drop flat-screen ones).
    pub vr_only: bool,
    /// Keep scenes with a creampie only.
    pub require_creampie: bool,
    pub bonuses: Bonuses,
    /// SLR favourites per likes point (SLR's scale for RANKING's likes term).
    pub popularity_per: f64,
}

/// RANKING in content-preferences.js.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ranking {
    pub vr: f64,
    pub artist_in_library: f64,
    pub favourite_artist: f64,
    pub multi_axis: f64,
    pub likes_per: f64,
    pub likes_max: f64,
    pub hentai_only: f64,
}

/// One of TAG_SCORE_GROUPS.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TagGroup {
    pub name: String,
    pub score: f64,
    pub tags: Vec<String>,
    /// The group also counts when the title names it (`monster`, `feet`, `ffm`).
    #[serde(default)]
    pub named: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoCaps {
    pub vr_max_height: u64,
    pub vr_max_fps: f64,
    pub flat_max_height: u64,
}

/// A listed pornstar (`pornstarTags.pornstars[]`); other fields are the
/// pornstar tagger's.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Pornstar {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PornstarTags {
    pub pornstars: Vec<Pornstar>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prefs {
    pub slr_scout: SlrScout,
    pub ranking: Ranking,
    pub tag_score_groups: Vec<TagGroup>,
    pub favourites: Vec<String>,
    /// ANIMATION_TAGS: never a live-action SLR scene's tags.
    pub animation_tags: Vec<String>,
    pub library_max_videos: usize,
    pub video_caps: VideoCaps,
    pub pornstar_tags: PornstarTags,
}

/// The embedded preferences.
pub fn load() -> Result<Prefs, String> {
    parse(SHARED)
}

pub fn parse(raw: &str) -> Result<Prefs, String> {
    let prefs = serde_json::from_str::<Prefs>(raw)
        .map_err(|e| format!("renderer/js/content-preferences.json: {e}"))?;
    let bad = |what: &str| Err(format!("content-preferences.json: {what}"));
    let s = &prefs.slr_scout;
    if s.actors.iter().all(|a| a.trim().is_empty()) && !s.include_favourites && !s.include_pornstars
    {
        return bad("slrScout lists nobody to scout");
    }
    if s.popularity_per <= 0.0 {
        return bad("slrScout.popularityPer must be above 0");
    }
    if prefs.ranking.likes_per <= 0.0 {
        return bad("ranking.likesPer must be above 0");
    }
    Ok(prefs)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// The shared file with `slrScout` replaced.
    pub fn with_slr_scout(slr_scout: &str) -> String {
        let mut v: serde_json::Value = serde_json::from_str(SHARED).unwrap();
        v["slrScout"] = serde_json::from_str(slr_scout).unwrap();
        v.to_string()
    }

    #[test]
    fn the_embedded_prefs_load() {
        let prefs = load().unwrap();
        assert!(prefs.slr_scout.include_favourites && prefs.slr_scout.include_pornstars);
        assert!(!prefs.slr_scout.require_creampie);
        assert!(prefs.slr_scout.popularity_per > 0.0);
        assert!(prefs.tag_score_groups.iter().any(|g| g.name == "Creampie"));
        assert!(
            prefs
                .tag_score_groups
                .iter()
                .any(|g| g.named.as_deref() == Some("feet"))
        );
        assert!(!prefs.favourites.is_empty() && !prefs.pornstar_tags.pornstars.is_empty());
        assert!(prefs.library_max_videos > 0 && prefs.video_caps.vr_max_height > 0);
    }

    #[test]
    fn rejects_bad_values() {
        let wrap = |actors: &str, favourites: bool, per: f64| {
            with_slr_scout(&format!(
                r#"{{"actors": {actors}, "includeFavourites": {favourites}, "includePornstars": false,
                "vrOnly": true, "requireCreampie": false,
                "bonuses": {{"handmadeScript": 4, "pmv": 2, "compilation": 2}},
                "popularityPer": {per}}}"#
            ))
        };
        assert!(parse(&wrap(r#"["A"]"#, false, 1000.0)).is_ok());
        assert!(parse(&wrap("[]", true, 1000.0)).is_ok());
        assert!(parse(&wrap("[]", false, 1000.0)).is_err());
        assert!(parse(&wrap(r#"[" "]"#, false, 1000.0)).is_err());
        assert!(parse(&wrap(r#"["A"]"#, false, 0.0)).is_err());
        assert!(parse("{}").is_err());
    }
}
