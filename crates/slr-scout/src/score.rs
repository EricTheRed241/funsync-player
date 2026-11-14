//! A scene's score, the same way the EroScripts deck scores its candidates
//! (`candidateScore` in renderer/js/preference-score.js), plus the SLR-only
//! extras of `slrScout`:
//!
//! - **Tags**: every tag at its score (TAG_SCORE_GROUPS defaults with the
//!   user's `library.tagScores` on top, 0.5 steps, ±10); a group counts once
//!   (its highest positive plus its lowest negative score), ungrouped tags
//!   add up. A `named` group (monster, feet, FFM) also counts, at its first
//!   tag's score, when a title word is one of its one-word tags (the JS
//!   title patterns aren't shared with Rust; this is their word list).
//!   SLR tags come in two spellings (`blow-job`, `blowjob`): a pair counts
//!   once, at the first spelling that has a score. SLR scenes are live
//!   action, so ANIMATION_TAGS (`animationTags`) are dropped from their
//!   tags: SLR's `3D` category means stereoscopic, not 3D animation.
//! - **RANKING**: `vr`; `artistInLibrary` when a performer has an artist
//!   folder in the library or the scene is a favourite's; the favourite
//!   artist bonus (`library.favouriteArtistScore`, else
//!   RANKING.favouriteArtist) when a performer or the studio is a favourite;
//!   `multiAxis` when a hand-made multi-axis script is known (an EroScripts
//!   post tagged multi-axis); likes: `min(likesMax, favourites /
//!   popularityPer)`. SLR favourites run ~30× EroScripts likes, so they get
//!   their own scale under the shared cap: `slrScout.popularityPer` 1500
//!   puts the median scene (~700 favourites, 742 scenes on 2026-11-13) near
//!   0.5, the 90th percentile (~3000) at 2, and the cap (3) at 4500, which
//!   only the top ~7 % reach. 2D hentai never applies to SLR's live action.
//! - **Extras** (`slrScout.bonuses`): a hand-made script, PMV, compilation.
//!
//! `scoreParts` names each non-zero part: `vr`, `artistInLibrary`,
//! `favourite`, `multiAxis`, `likes`, `tag:<tag>` (each counted tag),
//! `named:<group>`, `handmadeScript`, `pmv`, `compilation`. Two decimals.

use crate::library::{self, UserConfig};
use crate::prefs::{Prefs, TagGroup};
use crate::store::{Handmade, Scene};
use crate::text;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Tag scores run from -TAG_SCORE_MAX to +TAG_SCORE_MAX in TAG_SCORE_STEP
/// steps (renderer/js/preference-score.js).
pub const TAG_SCORE_MAX: f64 = 10.0;
pub const TAG_SCORE_STEP: f64 = 0.5;

pub fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// The key a tag's score is stored under: trimmed, lower case.
pub fn tag_key(tag: &str) -> String {
    tag.trim().to_lowercase()
}

/// `value` as a tag score (normalizeTagScore): a finite number or numeric
/// string rounded to the nearest step (halves up), clamped to ±10.
pub fn normalize_tag_score(value: &Value) -> Option<f64> {
    let n = match value {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) if !s.trim().is_empty() => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    if !n.is_finite() {
        return None;
    }
    let stepped = (n / TAG_SCORE_STEP + 0.5).floor() * TAG_SCORE_STEP;
    Some(stepped.clamp(-TAG_SCORE_MAX, TAG_SCORE_MAX) + 0.0)
}

/// A category name as a tag: lower case, every run of other characters → `-`.
pub fn slug(name: &str) -> String {
    text::normalise(name).replace(' ', "-")
}

/// A scene's tags from SLR's category names: each slugged, plus its
/// hyphen-less spelling (`blow-job` also as `blowjob`); `creampie` when a
/// timestamp says so (`creampie`). Distinct, in order.
pub fn scene_tags(categories: &[String], creampie: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |t: String| {
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    };
    for c in categories {
        let s = slug(c);
        let joined = s.replace('-', "");
        push(s);
        push(joined);
    }
    if creampie {
        push("creampie".into());
    }
    out
}

/// What the shared score looks at (a saved EroScripts candidate's fields).
#[derive(Debug, Clone, Default)]
pub struct Candidate<'a> {
    pub tags: &'a [String],
    pub title: &'a str,
    pub vr: bool,
    pub artist_in_library: bool,
    pub favourite: bool,
    pub multi_axis: bool,
    pub likes: f64,
}

pub struct Scorer {
    groups: Vec<TagGroup>,
    group_of: HashMap<String, usize>,
    effective: HashMap<String, f64>,
    prefs: Prefs,
    favourite_score: f64,
    /// Folded favourite name → as spelled.
    favourites: Vec<(String, String)>,
    creators: HashSet<String>,
}

impl Scorer {
    /// `creators`: the library's artist folders, folded.
    pub fn new(prefs: &Prefs, user: &UserConfig, creators: HashSet<String>) -> Scorer {
        let groups = prefs.tag_score_groups.clone();
        let mut group_of = HashMap::new();
        let mut effective = HashMap::new();
        for (i, g) in groups.iter().enumerate() {
            for t in &g.tags {
                group_of.insert(tag_key(t), i);
                effective.insert(tag_key(t), g.score);
            }
        }
        for (tag, value) in &user.tag_scores {
            let key = tag_key(tag);
            if let (false, Some(score)) = (key.is_empty(), normalize_tag_score(value)) {
                effective.insert(key, score);
            }
        }
        effective.retain(|_, v| *v != 0.0);
        let favourites = library::effective_favourites(&prefs.favourites, &user.favourite_changes)
            .into_iter()
            .map(|n| (library::fold(&n), n))
            .collect();
        Scorer {
            groups,
            group_of,
            effective,
            prefs: prefs.clone(),
            favourite_score: normalize_tag_score(&user.favourite_score)
                .unwrap_or(prefs.ranking.favourite_artist),
            favourites,
            creators,
        }
    }

    /// The favourite artists (FAVOURITES with the user's changes).
    pub fn favourites(&self) -> impl Iterator<Item = &str> {
        self.favourites.iter().map(|(_, n)| n.as_str())
    }

    /// The favourite among `names`, as spelled in the favourites.
    pub fn favourite_in<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> Option<String> {
        names.into_iter().find_map(|n| {
            let key = library::fold(n);
            self.favourites
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, name)| name.clone())
        })
    }

    pub fn in_library(&self, name: &str) -> bool {
        self.creators.contains(&library::fold(name))
    }

    fn score_of(&self, key: &str) -> f64 {
        self.effective.get(key).copied().unwrap_or(0.0)
    }

    /// Every scored tag of `tags` at its score (postTagScores).
    pub fn tag_scores(&self, tags: &[String]) -> BTreeMap<String, f64> {
        tags.iter()
            .map(|t| tag_key(t))
            .filter_map(|k| self.effective.get(&k).map(|v| (k, *v)))
            .collect()
    }

    /// The tags that count: one per spelling pair (the first scored
    /// spelling, else the first grouped one, else the first).
    fn counted(&self, tags: &[String]) -> Vec<String> {
        let mut pairs: Vec<(String, Vec<String>)> = Vec::new();
        for t in tags {
            let key = tag_key(t);
            if key.is_empty() {
                continue;
            }
            let joined = key.replace('-', "");
            match pairs.iter_mut().find(|(j, _)| *j == joined) {
                Some((_, keys)) if !keys.contains(&key) => keys.push(key),
                Some(_) => {}
                None => pairs.push((joined, vec![key])),
            }
        }
        pairs
            .into_iter()
            .map(|(_, keys)| {
                keys.iter()
                    .find(|k| self.effective.contains_key(*k))
                    .or_else(|| keys.iter().find(|k| self.group_of.contains_key(*k)))
                    .unwrap_or(&keys[0])
                    .clone()
            })
            .collect()
    }

    /// The tag part (tagScoreSum), as named parts.
    pub fn tag_parts(&self, tags: &[String], title: &str) -> Vec<(String, f64)> {
        // A score and the part it comes from.
        type Extreme = (f64, String);
        // group → highest positive, lowest negative
        let mut groups: BTreeMap<usize, (Extreme, Extreme)> = BTreeMap::new();
        let mut parts = Vec::new();
        for key in self.counted(tags) {
            let score = self.score_of(&key);
            match self.group_of.get(&key) {
                Some(&g) => {
                    let (high, low) = groups
                        .entry(g)
                        .or_insert(((0.0, String::new()), (0.0, String::new())));
                    if score > high.0 {
                        *high = (score, format!("tag:{key}"));
                    }
                    if score < low.0 {
                        *low = (score, format!("tag:{key}"));
                    }
                }
                None if score != 0.0 => parts.push((format!("tag:{key}"), score)),
                None => {}
            }
        }
        let title = text::normalise(title);
        let words: HashSet<&str> = text::words(&title).collect();
        for (i, g) in self.groups.iter().enumerate() {
            let Some(named) = &g.named else { continue };
            if groups.contains_key(&i)
                || !g
                    .tags
                    .iter()
                    .any(|t| !t.contains('-') && words.contains(tag_key(t).as_str()))
            {
                continue;
            }
            let score = g.tags.first().map_or(0.0, |t| self.score_of(&tag_key(t)));
            let name = format!("named:{named}");
            let entry = if score > 0.0 {
                ((score, name), (0.0, String::new()))
            } else {
                ((0.0, String::new()), (score, name))
            };
            groups.insert(i, entry);
        }
        for ((high, high_tag), (low, low_tag)) in groups.into_values() {
            if high > 0.0 {
                parts.push((high_tag, high));
            }
            if low < 0.0 {
                parts.push((low_tag, low));
            }
        }
        parts
    }

    /// The shared score's parts (candidateScore), `likes` on the scale
    /// `likes_per` (RANKING.likesPer for EroScripts likes).
    pub fn shared_parts(&self, c: &Candidate, likes_per: f64) -> Vec<(String, f64)> {
        let r = &self.prefs.ranking;
        let mut parts = Vec::new();
        let mut add = |name: &str, applies: bool, value: f64| {
            if applies && value != 0.0 {
                parts.push((name.to_string(), value));
            }
        };
        add("vr", c.vr, r.vr);
        add(
            "artistInLibrary",
            c.artist_in_library || c.favourite,
            r.artist_in_library,
        );
        add("favourite", c.favourite, self.favourite_score);
        add("multiAxis", c.multi_axis, r.multi_axis);
        add("likes", true, r.likes_max.min(c.likes / likes_per));
        parts.extend(self.tag_parts(c.tags, c.title));
        parts
    }

    /// The shared score alone (candidateScore), two decimals.
    pub fn candidate_score(&self, c: &Candidate, likes_per: f64) -> f64 {
        round2(self.shared_parts(c, likes_per).iter().map(|(_, v)| v).sum())
    }

    /// Set a scene's `favourite`, `tagScores`, `score` and `scoreParts`
    /// (its animation tags dropped first).
    pub fn score(&self, s: &mut Scene) {
        s.tags.retain(|t| !self.prefs.animation_tags.contains(t));
        s.favourite = self.favourite_in(s.actors.iter().chain(&s.studio).map(String::as_str));
        let multi_axis = s.handmade.iter().any(|h| {
            matches!(
                h,
                Handmade::Eroscripts {
                    multi_axis: true,
                    ..
                }
            )
        });
        let c = Candidate {
            tags: &s.tags,
            title: &s.title,
            vr: s.vr,
            artist_in_library: s.actors.iter().any(|a| self.in_library(a)),
            favourite: s.favourite.is_some(),
            multi_axis,
            likes: s.favorites as f64,
        };
        let scout = &self.prefs.slr_scout;
        let mut parts = self.shared_parts(&c, scout.popularity_per);
        let b = &scout.bonuses;
        for (name, applies, value) in [
            ("handmadeScript", !s.handmade.is_empty(), b.handmade_script),
            ("pmv", s.pmv, b.pmv),
            ("compilation", s.compilation, b.compilation),
        ] {
            if applies && value != 0.0 {
                parts.push((name.to_string(), value));
            }
        }
        s.tag_scores = self.tag_scores(&s.tags);
        s.score = round2(parts.iter().map(|(_, v)| v).sum());
        s.score_parts = parts.into_iter().map(|(k, v)| (k, round2(v))).collect();
    }
}

/// Score desc, then favourites desc, then id.
pub fn sort(scenes: &mut [Scene]) {
    scenes.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then(b.favorites.cmp(&a.favorites))
            .then(a.id.cmp(&b.id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn user(v: Value) -> UserConfig {
        library::from_json(&v)
    }

    fn scorer(v: Value, creators: &[&str]) -> Scorer {
        Scorer::new(
            &crate::prefs::load().unwrap(),
            &user(v),
            creators.iter().map(|c| c.to_string()).collect(),
        )
    }

    #[test]
    fn normalises_tag_scores() {
        assert_eq!(normalize_tag_score(&json!(2.3)), Some(2.5));
        assert_eq!(normalize_tag_score(&json!(-0.25)), Some(0.0));
        assert_eq!(normalize_tag_score(&json!(-0.3)), Some(-0.5));
        assert_eq!(normalize_tag_score(&json!("1.75")), Some(2.0));
        assert_eq!(normalize_tag_score(&json!(99)), Some(10.0));
        assert_eq!(normalize_tag_score(&json!(-99)), Some(-10.0));
        assert_eq!(normalize_tag_score(&json!(" ")), None);
        assert_eq!(normalize_tag_score(&json!(null)), None);
        assert_eq!(normalize_tag_score(&json!(true)), None);
    }

    #[test]
    fn slugs_categories_in_both_spellings() {
        assert_eq!(slug("Reverse Cowgirl"), "reverse-cowgirl");
        assert_eq!(slug("Big tits"), "big-tits");
        assert_eq!(slug("POV"), "pov");
        assert_eq!(slug("Cum on Tits!"), "cum-on-tits");
        assert_eq!(
            scene_tags(&strings(&["Blow Job", "POV", "Hand job"]), true),
            [
                "blow-job", "blowjob", "pov", "hand-job", "handjob", "creampie"
            ]
        );
        assert_eq!(scene_tags(&strings(&["Creampie"]), true), ["creampie"]);
    }

    // Parity with candidateScore (renderer/js/preference-score.js): the
    // expected numbers were computed by running the JS function on the same
    // candidates (node, 2026-11-13).
    #[test]
    fn matches_candidate_score() {
        let user_scores = json!({"library": {
            "tagScores": {"pov": 2.5, "reverse-cowgirl": 3, "anal": -1, "big-tits": -0.5,
                          "cumshot": 0.5, "creampie": "2.2", "feet": -2},
            "favouriteArtistScore": 6.2}});
        let s = scorer(user_scores, &[]);
        let case = |tags: &[&str], title: &str, vr, ail, fav, multi, likes: f64| {
            let tags = strings(tags);
            s.candidate_score(
                &Candidate {
                    tags: &tags,
                    title,
                    vr,
                    artist_in_library: ail,
                    favourite: fav,
                    multi_axis: multi,
                    likes,
                },
                20.0,
            )
        };
        // JS: candidateScore({vr:true, tags:[...], likes:37, ...}, scoring)
        assert_eq!(
            case(
                &[
                    "pov",
                    "male-pov",
                    "creampie",
                    "cowgirl",
                    "reverse-cowgirl",
                    "anal",
                    "big-tits",
                    "cumshot"
                ],
                "Hot Milf Is Hungry",
                true,
                false,
                false,
                false,
                37.0
            ),
            JS_CASE_1
        );
        assert_eq!(
            case(
                &["feet", "foot", "ffm", "monster", "music", "pmv"],
                "A title",
                false,
                true,
                false,
                true,
                100.0
            ),
            JS_CASE_2
        );
        assert_eq!(
            case(
                &["blowjob", "big-tits"],
                "Tentacles and FFM feet fun",
                true,
                false,
                true,
                false,
                5.0
            ),
            JS_CASE_3
        );
        assert_eq!(case(&[], "", false, false, false, false, 0.0), JS_CASE_4);
    }

    const JS_CASE_1: f64 = 11.35;
    const JS_CASE_2: f64 = 8.0;
    const JS_CASE_3: f64 = 9.25;
    const JS_CASE_4: f64 = 0.0;

    #[test]
    fn a_spelling_pair_counts_once() {
        let s = scorer(
            json!({"library": {"tagScores": {"blowjob": 1.5, "blow-job": 1}}}),
            &[],
        );
        let tags = scene_tags(&strings(&["Blow Job"]), false);
        assert_eq!(s.tag_parts(&tags, ""), [("tag:blow-job".to_string(), 1.0)]);
        let s = scorer(json!({"library": {"tagScores": {"handjob": 2}}}), &[]);
        let tags = scene_tags(&strings(&["Hand Job"]), false);
        assert_eq!(s.tag_parts(&tags, ""), [("tag:handjob".to_string(), 2.0)]);
        assert_eq!(
            s.tag_scores(&tags),
            BTreeMap::from([("handjob".into(), 2.0)])
        );
    }

    fn scene(extra: Value) -> Scene {
        let mut d = json!({"id": 1, "label": "x-1", "title": "Oiled Up", "isFlatScreenType": false,
            "studio": {"name": "VRBangers"}, "actors": [{"name": "Miss Lexa"}, {"name": "Lexi Luna"}],
            "quantity": {"favorites": 3000},
            "categories": [{"name": "3D"}, {"name": "POV"}, {"name": "Reverse Cowgirl"}, {"name": "Compilation"}]});
        for (k, v) in extra.as_object().unwrap() {
            d[k] = v.clone();
        }
        crate::slr::scene(&d).unwrap()
    }

    fn parts(s: &Scene) -> Vec<(&str, f64)> {
        s.score_parts
            .iter()
            .map(|(k, v)| (k.as_str(), *v))
            .collect()
    }

    #[test]
    fn scores_scenes_with_the_extras() {
        // No favourites changed: VRBangers isn't one; Lexi Luna has a folder.
        let s = scorer(
            json!({"library": {"tagScores": {"pov": 2.5}}}),
            &["lexi luna"],
        );
        let mut sc = scene(json!({}));
        sc.handmade.push(Handmade::Eroscripts {
            topic_id: 7,
            title: "t".into(),
            free: true,
            multi_axis: true,
        });
        s.score(&mut sc);
        assert_eq!(sc.favourite, None);
        assert_eq!(
            parts(&sc),
            [
                ("artistInLibrary", 2.0),
                ("compilation", 2.0),
                ("handmadeScript", 4.0),
                ("likes", 2.0),
                ("multiAxis", 4.0),
                ("tag:compilation", 2.0),
                ("tag:pov", 2.5),
                ("tag:reverse-cowgirl", 2.0),
                ("vr", 3.0),
            ]
        );
        assert_eq!(sc.score, 23.5);
        assert_eq!(sc.tag_scores["pov"], 2.5);
        // SLR's 3D (stereoscopic) isn't the deck's 3D animation tag.
        assert_eq!(
            sc.tags,
            ["pov", "reverse-cowgirl", "reversecowgirl", "compilation"]
        );

        // The studio is a favourite (the user's change), with the user's bonus.
        let s = scorer(
            json!({"library": {"favouriteArtists": {"VRBangers": true}, "favouriteArtistScore": 1}}),
            &[],
        );
        let mut sc = scene(json!({"quantity": {"favorites": 90000}, "categories": []}));
        s.score(&mut sc);
        assert_eq!(sc.favourite.as_deref(), Some("VRBangers"));
        assert_eq!(
            parts(&sc),
            [
                ("artistInLibrary", 2.0),
                ("favourite", 1.0),
                ("likes", 3.0),
                ("vr", 3.0)
            ]
        );
        assert_eq!(sc.score, 9.0);
    }

    #[test]
    fn sorts_by_score_then_favorites() {
        let mut a = scene(json!({"id": 1}));
        a.score = 2.0;
        a.favorites = 100;
        let mut b = scene(json!({"id": 2}));
        b.score = 2.0;
        b.favorites = 900;
        let mut c = scene(json!({"id": 3}));
        c.score = 5.0;
        let mut all = vec![a, b, c];
        sort(&mut all);
        assert_eq!(all.iter().map(|s| s.id).collect::<Vec<_>>(), [3, 2, 1]);
    }
}
