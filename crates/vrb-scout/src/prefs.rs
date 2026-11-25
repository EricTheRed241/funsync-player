//! This scout's part of the shared preferences (`vrbScout` in
//! shared/content-preferences.json); everything else it scores and
//! screens with comes through `slr_scout::prefs`.

use crate::sites::SITES;
use serde::Deserialize;
use std::collections::BTreeMap;

const SHARED: &str = include_str!("../../../shared/content-preferences.json");

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VrbScout {
    /// Views per likes point, per site (`Site::key`): the sites' view counts
    /// for RANKING's likes term (`min(likesMax, views / popularityPer)`).
    /// Views run on very different scales: on 2026-11-17 VRBangers' 908
    /// scenes had a median of ~19,500 views and a 90th percentile of
    /// ~60,600, VRConk's 249 a median of ~1,700 and a 90th percentile of
    /// ~4,800. 30,000 and 2,500 put each site's median near 0.7, its 90th
    /// percentile near 2 and the cap (3) at about the top 3 %, the same
    /// spread `slrScout.popularityPer` gives SLR scenes.
    pub popularity_per: BTreeMap<String, f64>,
}

#[derive(Deserialize)]
struct Shared {
    #[serde(rename = "vrbScout")]
    vrb_scout: VrbScout,
}

/// The embedded preferences.
pub fn load() -> Result<VrbScout, String> {
    parse(SHARED)
}

pub fn parse(raw: &str) -> Result<VrbScout, String> {
    let shared = serde_json::from_str::<Shared>(raw)
        .map_err(|e| format!("shared/content-preferences.json: {e}"))?;
    let p = shared.vrb_scout;
    for site in SITES {
        match p.popularity_per.get(site.key) {
            Some(per) if *per > 0.0 => {}
            _ => {
                return Err(format!(
                    "content-preferences.json: vrbScout.popularityPer.{} must be above 0",
                    site.key
                ));
            }
        }
    }
    Ok(p)
}

impl VrbScout {
    pub fn popularity_per(&self, site: &str) -> f64 {
        self.popularity_per
            .get(site)
            .copied()
            .unwrap_or(f64::INFINITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_prefs_load() {
        let p = load().unwrap();
        for site in SITES {
            assert!(p.popularity_per(site.key) > 0.0, "{}", site.key);
        }
    }

    #[test]
    fn every_site_needs_a_scale() {
        assert!(parse(r#"{"vrbScout": {"popularityPer": {"vrbangers": 1, "vrconk": 2}}}"#).is_ok());
        assert!(parse(r#"{"vrbScout": {"popularityPer": {"vrbangers": 1}}}"#).is_err());
        assert!(
            parse(r#"{"vrbScout": {"popularityPer": {"vrbangers": 1, "vrconk": 0}}}"#).is_err()
        );
        assert!(parse("{}").is_err());
    }
}
