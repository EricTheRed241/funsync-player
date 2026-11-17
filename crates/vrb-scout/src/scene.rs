//! A listing entry and its detail as a deck scene, screened and scored.
//!
//! Scoring is the EroScripts deck's (`slr_scout::score::Scorer`, the
//! candidateScore port): the categories as tags (`score::scene_tags`: each
//! name slugged, plus its hyphen-less spelling; `creampie` when a category
//! or the title says so), animation tags dropped as for SLR (these are live
//! action), `vr` for the VR projection every scene of these sites has, the
//! library-artist and favourite parts for the performers and studio, and
//! likes from the site's view count on the site's own scale
//! (`vrbScout.popularityPer`). Of the SLR extras (`slrScout.bonuses`) the
//! PMV and compilation ones fit (same title test); the hand-made-script one
//! doesn't: every scene here has the studio's own toy script, so it would
//! lift the whole deck alike.

use crate::api::{Detail, Listed};
use crate::prefs::VrbScout;
use crate::sites::Site;
use crate::store::{Scene, Script};
use slr_scout::prefs::Prefs;
use slr_scout::score::{self, Candidate, Scorer, round2};
use slr_scout::screen::{self, Item, Reason};
use slr_scout::text;
use std::collections::BTreeMap;

/// A fresh scene from a listing entry and its detail (the detail's fields
/// left empty when there is none); scores are set by [`assess`].
pub fn build(site: &Site, l: &Listed, d: Option<&Detail>) -> Scene {
    let script = l
        .toy_script
        .clone()
        .or_else(|| l.short_name.as_deref().map(|s| site.script_url(s)))
        .map(|url| Script { url, axes: vec![] });
    let d = d.cloned().unwrap_or_default();
    Scene {
        id: l.id.clone(),
        site: site.key.to_string(),
        url: site.scene_url(&l.slug),
        title: l.title.clone(),
        studio: site.name.to_string(),
        actors: l.models.clone(),
        date: l
            .published_at
            .map(|t| slr_scout::quota::epoch_iso(t)[..10].to_string()),
        duration_s: l.duration_s,
        thumbnail: l.poster.as_deref().and_then(|p| site.asset(p)),
        images: d.images,
        vr: false,
        fps: d.fps,
        passthrough: d.passthrough,
        creampie: false,
        pmv: false,
        compilation: false,
        likes: l.views,
        tags: vec![],
        tag_scores: BTreeMap::new(),
        favourite: None,
        script,
        handmade: vec![],
        in_library: false,
        fetched_at: None,
        score: 0.0,
        score_parts: BTreeMap::new(),
        slug: l.slug.clone(),
        short_name: l.short_name.clone(),
        qualities: l.qualities.clone(),
        categories: d.categories,
        updated_at: d.updated_at,
        list_key: l.key.clone(),
        excluded: None,
    }
}

/// The scene's listing-side fields refreshed from a newer listing entry with
/// the same detail (views move every day; nothing else changed).
pub fn refresh(s: &mut Scene, l: &Listed) {
    s.likes = l.views;
}

fn has_word(normalised: &str, word: &str) -> bool {
    text::words(normalised).any(|w| w == word)
}

/// Set the derived fields (tags, flags, favourite, score) and screen the
/// scene; returns why it is turned down, if it is.
pub fn assess(s: &mut Scene, scorer: &Scorer, prefs: &Prefs, vrb: &VrbScout) -> Option<Reason> {
    let title = text::normalise(&s.title);
    let cats: Vec<String> = s.categories.iter().map(|c| text::normalise(c)).collect();
    s.creampie = has_word(&title, "creampie") || cats.iter().any(|c| has_word(c, "creampie"));
    s.pmv = has_word(&title, "pmv");
    s.compilation = has_word(&title, "compilation")
        || cats
            .iter()
            .any(|c| c == "compilation" || c == "compilations");
    s.vr = s.projection().is_some();
    let mut tags = score::scene_tags(&s.categories, s.creampie);
    let creators: Vec<String> = s.actors.iter().chain([&s.studio]).cloned().collect();
    // Screened with every tag, so a blocked tag that is also an animation
    // tag (mmd) still turns the scene down.
    let reason = screen::screen(
        &prefs.screening,
        &Item {
            title: &s.title,
            tags: &tags,
            text: "",
            creators: &creators,
            duration_ms: s.duration_s.map(|d| d * 1000),
        },
    );
    tags.retain(|t| !prefs.animation_tags.contains(t));
    s.tags = tags;
    s.favourite = scorer.favourite_in(creators.iter().map(String::as_str));
    let c = Candidate {
        tags: &s.tags,
        title: &s.title,
        vr: s.vr,
        artist_in_library: s.actors.iter().any(|a| scorer.in_library(a)),
        favourite: s.favourite.is_some(),
        multi_axis: false,
        likes: s.likes as f64,
    };
    let mut parts = scorer.shared_parts(&c, vrb.popularity_per(&s.site));
    let b = &prefs.slr_scout.bonuses;
    for (name, applies, value) in [
        ("pmv", s.pmv, b.pmv),
        ("compilation", s.compilation, b.compilation),
    ] {
        if applies && value != 0.0 {
            parts.push((name.to_string(), value));
        }
    }
    s.tag_scores = scorer.tag_scores(&s.tags);
    s.score = round2(parts.iter().map(|(_, v)| v).sum());
    s.score_parts = parts.into_iter().map(|(k, v)| (k, round2(v))).collect();
    s.excluded = reason.as_ref().map(ToString::to_string);
    reason
}

/// Score desc, then views desc, then newest, then id.
pub fn sort(scenes: &mut [Scene]) {
    scenes.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(b.likes.cmp(&a.likes))
            .then(b.date.cmp(&a.date))
            .then(a.id.cmp(&b.id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sites::by_key;
    use serde_json::{Value, json};
    use slr_scout::library::{self, UserConfig};

    fn listed(title: &str, views: u64) -> Listed {
        crate::api::listed(&json!({
            "id": "6a", "slug": "a-scene", "title": title, "hasToyScript": true,
            "models": [{"title": "Angel Youngs"}, {"title": "Scarlette Moon"}],
            "poster": {"permalink": "/uploads/p.jpg"}, "publishedAt": 1792738860,
            "videoSettings": {"duration": 3115, "videoShortName": "a_scene",
                "downloadsQuality": ["4K_180x180_3dh", "8K_180x180_3dh"]},
            "viewCount": views
        }))
        .unwrap()
    }

    fn detail(categories: &[&str]) -> Detail {
        Detail {
            updated_at: Some(5),
            categories: categories.iter().map(|c| c.to_string()).collect(),
            images: vec!["https://content.vrbangers.com/1.jpg".into()],
            fps: Some(60.0),
            passthrough: false,
        }
    }

    fn scorer(config: Value, creators: &[&str]) -> (Scorer, Prefs) {
        let prefs = slr_scout::prefs::load().unwrap();
        let user: UserConfig = library::from_json(&config);
        let s = Scorer::new(
            &prefs,
            &user,
            creators.iter().map(|c| library::fold(c)).collect(),
        );
        (s, prefs)
    }

    #[test]
    fn builds_the_scene() {
        let site = by_key("vrbangers").unwrap();
        let s = build(site, &listed("Wet & Wild", 10), Some(&detail(&["Babe"])));
        assert_eq!(s.url, "https://vrbangers.com/video/a-scene/");
        assert_eq!(s.studio, "VRBangers");
        assert_eq!(s.date.as_deref(), Some("2026-10-23"));
        assert_eq!(
            s.thumbnail.as_deref(),
            Some("https://content.vrbangers.com/uploads/p.jpg")
        );
        assert_eq!(s.script.as_ref().unwrap().url, site.script_url("a_scene"));
        assert_eq!(s.categories, ["Babe"]);
        assert_eq!(s.updated_at, Some(5));
        // No detail yet: asked again next scan.
        let s = build(site, &listed("T", 10), None);
        assert!(s.categories.is_empty() && s.updated_at.is_none() && s.images.is_empty());
    }

    #[test]
    fn scores_like_the_deck() {
        let site = by_key("vrbangers").unwrap();
        let vrb = crate::prefs::load().unwrap();
        let (scorer, prefs) = scorer(
            json!({"settings": {"library": {"tagScores": {"pov": 1.5}}}}),
            &["Scarlette Moon"],
        );
        let per = vrb.popularity_per("vrbangers");
        let mut s = build(
            site,
            &listed("Creampie Compilation", (per * 1.5) as u64),
            Some(&detail(&["POV", "Big Tits", "3D", "Animation"])),
        );
        assert_eq!(assess(&mut s, &scorer, &prefs, &vrb), None);
        assert!(s.creampie && s.compilation && !s.pmv && s.vr);
        // Animation tags are dropped (live action); creampie from the title.
        assert!(!s.tags.contains(&"3d".to_string()) && !s.tags.contains(&"animation".to_string()));
        assert!(
            s.tags.contains(&"creampie".to_string()) && s.tags.contains(&"bigtits".to_string())
        );
        let r = &prefs.ranking;
        assert_eq!(s.score_parts["vr"], r.vr);
        assert_eq!(s.score_parts["likes"], 1.5);
        assert_eq!(s.score_parts["artistInLibrary"], r.artist_in_library);
        assert_eq!(s.score_parts["tag:pov"], 1.5);
        assert_eq!(
            s.score_parts["compilation"],
            prefs.slr_scout.bonuses.compilation
        );
        assert!(!s.score_parts.contains_key("handmadeScript"));
        assert_eq!(s.tag_scores["pov"], 1.5);
        let sum: f64 = s.score_parts.values().sum();
        assert!((s.score - round2(sum)).abs() < 1e-9);
        // The likes term stops at likesMax.
        s.likes = (per * 100.0) as u64;
        assess(&mut s, &scorer, &prefs, &vrb);
        assert_eq!(s.score_parts["likes"], r.likes_max);
        // VRConk views run on their own scale.
        let mut c = build(
            by_key("vrconk").unwrap(),
            &listed("T", vrb.popularity_per("vrconk") as u64),
            Some(&detail(&[])),
        );
        assess(&mut c, &scorer, &prefs, &vrb);
        assert_eq!(c.score_parts["likes"], 1.0);
        assert_eq!(c.studio, "VRConk");
    }

    #[test]
    fn favourites_and_screening() {
        let site = by_key("vrbangers").unwrap();
        let vrb = crate::prefs::load().unwrap();
        let (scorer, prefs) = scorer(
            json!({"settings": {"library": {"favouriteArtists": {"Angel Youngs": true}}}}),
            &[],
        );
        let mut s = build(site, &listed("T", 0), Some(&detail(&[])));
        assert_eq!(assess(&mut s, &scorer, &prefs, &vrb), None);
        assert_eq!(s.favourite.as_deref(), Some("Angel Youngs"));
        assert!(s.score_parts.contains_key("favourite"));
        let word = prefs.screening.excluded_words[0].clone();
        let mut bad = build(
            site,
            &listed(&format!("A {word} scene"), 0),
            Some(&detail(&[])),
        );
        let why = assess(&mut bad, &scorer, &prefs, &vrb).unwrap();
        assert_eq!(why.key(), "excluded");
        assert_eq!(bad.excluded.as_deref(), Some(why.to_string().as_str()));
        let mut short = build(site, &listed("T", 0), Some(&detail(&[])));
        short.duration_s = Some(prefs.screening.min_video_ms / 1000 - 1);
        assert_eq!(
            assess(&mut short, &scorer, &prefs, &vrb).unwrap().key(),
            "too-short"
        );
        let tag = prefs.screening.blocked_tags[0].clone();
        assert!(
            prefs.animation_tags.contains(&tag),
            "mmd: blocked and an animation tag"
        );
        let mut blocked = build(site, &listed("T", 0), Some(&detail(&[tag.as_str()])));
        assert_eq!(
            assess(&mut blocked, &scorer, &prefs, &vrb).unwrap().key(),
            "blocked"
        );
    }

    #[test]
    fn sorts_best_first() {
        let site = by_key("vrbangers").unwrap();
        let mut a = build(site, &listed("A", 1), None);
        a.id = "a".into();
        a.score = 2.0;
        let mut b = a.clone();
        b.id = "b".into();
        b.score = 5.0;
        let mut c = a.clone();
        c.id = "c".into();
        c.likes = 9;
        let mut list = vec![a, b, c];
        sort(&mut list);
        let ids: Vec<&str> = list.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["b", "c", "a"]);
    }
}
