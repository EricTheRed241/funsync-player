//! Speed stats (`library.speedStatsCache`) and preference scores
//! (`library.preferenceScores`, `shared/preference-score.js` `candidateScore`
//! over the library), ported from the old Electron app and backend.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value, json};

use super::{Ctx, RULES, artist_of, axis_kind, fold, num, obj_mut, strs};
use crate::data::Library;

/// `{avgSpeed, maxSpeed}` in units/s for every main script not in the
/// cache yet; a script that can't be parsed gets 0 / 0, a missing one waits.
pub(super) fn speed_stats(lib: &mut Library, cx: &Ctx) {
    let cache = obj_mut(lib.lib_mut("speedStatsCache"));
    for p in cx.videos.iter().filter_map(|v| v.funscript_path.as_ref()) {
        if cache.contains_key(p) {
            continue;
        }
        let Ok(bytes) = std::fs::read(p) else {
            continue;
        };
        let actions = serde_json::from_slice::<Value>(&bytes)
            .map(|s| s["actions"].as_array().cloned().unwrap_or_default())
            .unwrap_or_default();
        let (avg, max) = speeds(&actions);
        cache.insert(p.clone(), json!({"avgSpeed": avg, "maxSpeed": max}));
    }
}

/// Average and top speed of the moves (held positions left out), rounded
/// half to even as Python does.
pub(super) fn speeds(actions: &[Value]) -> (i64, i64) {
    let (mut total, mut max, mut count) = (0.0, 0.0f64, 0);
    for w in actions.windows(2) {
        let f = |a: &Value, k: &str| a[k].as_f64().unwrap_or(0.0);
        let dt = f(&w[1], "at") - f(&w[0], "at");
        let dp = (f(&w[1], "pos") - f(&w[0], "pos")).abs();
        if dt <= 0.0 || dp == 0.0 {
            continue;
        }
        let speed = dp / dt * 1000.0;
        total += speed;
        max = max.max(speed);
        count += 1;
    }
    let avg = if count > 0 { total / count as f64 } else { 0.0 };
    (avg.round_ties_even() as i64, max.round_ties_even() as i64)
}

/// `value` as a tag score: rounded to the nearest step (halves up),
/// clamped to ±max; none for anything but a number or numeric string.
pub fn normalize_tag_score(v: &Value) -> Option<f64> {
    let n = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) if !s.trim().is_empty() => s.trim().parse().ok()?,
        _ => return None,
    };
    let (max, step) = (rule("tagScoreMax"), rule("tagScoreStep"));
    n.is_finite()
        .then(|| ((n / step + 0.5).floor() * step).clamp(-max, max) + 0.0)
}

fn rule(k: &str) -> f64 {
    RULES.mirror[k].as_f64().unwrap()
}

fn ranking(k: &str) -> f64 {
    RULES.prefs["ranking"][k].as_f64().unwrap()
}

/// Every tag's score: the groups' defaults with the user's changes on top;
/// tags at 0 left out.
pub fn effective_tag_scores(user: &Value) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    for g in RULES.prefs["tagScoreGroups"].as_array().unwrap() {
        for t in strs(&g["tags"]) {
            out.insert(t.trim().to_lowercase(), g["score"].as_f64().unwrap());
        }
    }
    for (tag, v) in user.as_object().into_iter().flatten() {
        let key = tag.trim().to_lowercase();
        if let (false, Some(s)) = (key.is_empty(), normalize_tag_score(v)) {
            out.insert(key, s);
        }
    }
    out.retain(|_, s| *s != 0.0);
    out
}

/// The tag part: each distinct tag at its score, a TAG_SCORE_GROUPS group
/// once (its highest positive plus its lowest negative score); a `named`
/// group also counts, at its first tag's score, when the title names it and
/// none of its tags is there.
pub(super) fn tag_score_sum(tags: &[String], title: &str, eff: &HashMap<String, f64>) -> f64 {
    let groups = RULES.prefs["tagScoreGroups"].as_array().unwrap();
    let group_of = |key: &str| {
        groups
            .iter()
            .position(|g| strs(&g["tags"]).iter().any(|t| t == key))
    };
    let score_of = |key: &str| eff.get(key).copied().unwrap_or(0.0);
    let mut extremes: HashMap<usize, (f64, f64)> = HashMap::new();
    let count = |ex: &mut HashMap<usize, (f64, f64)>, g: usize, s: f64| {
        let e = ex.entry(g).or_insert((0.0, 0.0));
        *e = (e.0.max(s), e.1.min(s));
    };
    let mut sum = 0.0;
    let mut seen = HashSet::new();
    for key in tags.iter().map(|t| t.trim().to_lowercase()) {
        if key.is_empty() || !seen.insert(key.clone()) {
            continue;
        }
        match group_of(&key) {
            Some(g) => count(&mut extremes, g, score_of(&key)),
            None => sum += score_of(&key),
        }
    }
    for (i, g) in groups.iter().enumerate() {
        let Some(named) = g["named"].as_str() else {
            continue;
        };
        if !extremes.contains_key(&i) && RULES.named[named].is_match(title) {
            count(
                &mut extremes,
                i,
                score_of(&strs(&g["tags"])[0].trim().to_lowercase()),
            );
        }
    }
    sum + extremes.values().map(|(h, l)| h + l).sum::<f64>()
}

/// One library scene as the scouts' candidate.
#[derive(Default)]
pub(super) struct Scene<'a> {
    pub title: &'a str,
    pub tags: Vec<String>,
    pub likes: f64,
    pub vr: bool,
    pub multi_axis: bool,
    pub artist_in_library: bool,
    pub favourite: bool,
}

/// `candidateScore`: RANKING plus the tag scores, two decimals.
pub(super) fn candidate_score(c: &Scene, eff: &HashMap<String, f64>, favourite_score: f64) -> f64 {
    let has = |list: &str| {
        let l = strs(&RULES.mirror[list]);
        c.tags.iter().any(|t| l.contains(t))
    };
    let hentai_only =
        has("flatHentaiTags") && !has("threeDTags") && !RULES.three_d.is_match(c.title);
    let on = |b: bool, x: f64| if b { x } else { 0.0 };
    let score = on(c.vr, ranking("vr"))
        + on(
            c.artist_in_library || c.favourite,
            ranking("artistInLibrary"),
        )
        + on(c.favourite, favourite_score)
        + on(c.multi_axis, ranking("multiAxis"))
        + ranking("likesMax").min(c.likes / ranking("likesPer"))
        - on(hentai_only, ranking("hentaiOnly"))
        + tag_score_sum(&c.tags, c.title, eff);
    (score * 100.0).round() / 100.0
}

/// The file name without its extension (library-scores.js `titleOf`).
fn score_title(path: &str) -> &str {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.rfind('.') {
        Some(i) if i + 1 < name.len() => &name[..i],
        _ => name,
    }
}

pub(super) fn preference_scores(lib: &mut Library, cx: &Ctx) {
    let l = lib.lib();
    let eff = effective_tag_scores(&l["tagScores"]);
    let fav_score =
        normalize_tag_score(&l["favouriteArtistScore"]).unwrap_or(ranking("favouriteArtist"));
    let favourites: HashSet<String> = super::favourites(lib).into_iter().map(|f| f.0).collect();
    let artists: Vec<Option<String>> = cx
        .videos
        .iter()
        .map(|v| artist_of(&v.path, &cx.sources).map(|a| fold(&a)))
        .collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for a in artists.iter().flatten() {
        *counts.entry(a).or_default() += 1;
    }
    let min_videos = RULES.mirror["minArtistVideos"].as_u64().unwrap() as usize;
    let mut scores = Map::new();
    for (v, artist) in cx.videos.iter().zip(&artists) {
        let record = &l["scriptTags"][&v.path];
        let mut tags: Vec<String> = Vec::new();
        for t in strs(&record["tags"])
            .iter()
            .map(|t| t.trim().to_lowercase())
        {
            if !tags.contains(&t) {
                tags.push(t);
            }
        }
        let favourite = artist.as_ref().is_some_and(|a| favourites.contains(a));
        let scene = Scene {
            title: score_title(&v.path),
            tags,
            likes: record["likes"].as_f64().unwrap_or(0.0),
            vr: cx.vr[v.path.as_str()],
            multi_axis: axis_kind(v, cx.embedded) == Some("handmade"),
            artist_in_library: favourite
                || artist
                    .as_ref()
                    .is_some_and(|a| counts[a.as_str()] >= min_videos),
            favourite,
        };
        scores.insert(
            v.path.clone(),
            num(candidate_score(&scene, &eff, fav_score)),
        );
    }
    let same = l["preferenceScores"].as_object().is_some_and(|old| {
        old.len() == scores.len()
            && scores
                .iter()
                .all(|(k, s)| old.get(k).and_then(Value::as_f64) == s.as_f64())
    });
    if !same {
        *lib.lib_mut("preferenceScores") = Value::Object(scores);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn speed_skips_holds_and_rounds_half_to_even() {
        let a = json!([{"at": 0, "pos": 0}, {"at": 1000, "pos": 50}, {"at": 2000, "pos": 50},
                       {"at": 2400, "pos": 0}, {"at": 2400, "pos": 90}]);
        // 50 u/s and 125 u/s → avg 87.5 → 88 (even); max 125.
        assert_eq!(speeds(a.as_array().unwrap()), (88, 125));
        let b = json!([{"at": 0, "pos": 0}, {"at": 1000, "pos": 25}, {"at": 2000, "pos": 45}]);
        assert_eq!(speeds(b.as_array().unwrap()), (22, 25), "22.5 rounds to 22");
        assert_eq!(speeds(&[]), (0, 0));
    }

    #[test]
    fn tag_scores_step_clamp_and_override_defaults() {
        assert_eq!(normalize_tag_score(&json!(1.26)), Some(1.5));
        assert_eq!(normalize_tag_score(&json!(-1.25)), Some(-1.0), "halves up");
        assert_eq!(normalize_tag_score(&json!("12")), Some(10.0));
        assert_eq!(normalize_tag_score(&json!(" ")), None);
        assert_eq!(normalize_tag_score(&json!("-42")), Some(-10.0));
        assert_eq!(normalize_tag_score(&json!(true)), None);
        let eff = effective_tag_scores(&json!({"creampie": 4, "pov": 0, "Anal": -1}));
        assert_eq!(eff["creampie"], 4.0);
        assert_eq!(eff["anal-creampie"], 2.0, "group default");
        assert_eq!(eff["anal"], -1.0);
        assert!(!eff.contains_key("pov"), "explicit 0 removes a default");
    }

    #[test]
    fn a_group_counts_once_synonyms_dont_add_up() {
        let eff = effective_tag_scores(&json!({"anal-creampie": -1}));
        // Creampie group: creampie 2, anal-creampie -1 → 2 + -1; Riding twice → 2 once.
        let s = tag_score_sum(
            &tags(&["creampie", "anal-creampie", "riding", "cowgirl", "Riding"]),
            "",
            &eff,
        );
        assert_eq!(s, 3.0);
        // Ungrouped tags add up.
        let eff = effective_tag_scores(&json!({"a": 1, "b": 1.5}));
        assert_eq!(tag_score_sum(&tags(&["a", "b"]), "", &eff), 2.5);
    }

    #[test]
    fn named_groups_count_from_the_title_once() {
        let eff = effective_tag_scores(&json!({}));
        assert_eq!(tag_score_sum(&[], "Tentacle Monsters", &eff), -0.5);
        assert_eq!(tag_score_sum(&[], "Mercy X WW", &eff), -0.5, "WW token");
        assert_eq!(
            tag_score_sum(&[], "Mercy x ww", &eff),
            0.0,
            "WW is case-sensitive"
        );
        assert_eq!(tag_score_sum(&[], "触手", &eff), -0.5);
        assert_eq!(tag_score_sum(&[], "Monstrous", &eff), 0.0, "whole words");
        assert_eq!(
            tag_score_sum(&tags(&["tentacle"]), "Monster", &eff),
            -0.5,
            "tag and title: once"
        );
        assert_eq!(
            tag_score_sum(&[], "FFM feet", &eff),
            -1.0,
            "two groups add up"
        );
    }

    #[test]
    fn candidate_score_ranking() {
        let eff = effective_tag_scores(&json!({}));
        let base = |f: &dyn Fn(&mut Scene)| {
            let mut s = Scene {
                title: "x",
                ..Default::default()
            };
            f(&mut s);
            candidate_score(&s, &eff, 5.0)
        };
        assert_eq!(base(&|_| {}), 0.0);
        assert_eq!(base(&|s| s.vr = true), 3.0);
        assert_eq!(
            base(&|s| s.favourite = true),
            7.0,
            "favourite counts as an artist too"
        );
        assert_eq!(base(&|s| s.artist_in_library = true), 2.0);
        assert_eq!(base(&|s| s.multi_axis = true), 4.0);
        assert_eq!(base(&|s| s.likes = 30.0), 1.5);
        assert_eq!(base(&|s| s.likes = 500.0), 3.0, "likes capped");
        assert_eq!(
            base(&|s| s.tags = tags(&["hentai"])),
            -3.0,
            "2D hentai only"
        );
        assert_eq!(base(&|s| s.tags = tags(&["hentai", "3d"])), 0.0);
        assert_eq!(
            base(&|s| {
                s.tags = tags(&["hentai"]);
                s.title = "SFM thing";
            }),
            0.0,
            "3D in the title"
        );
    }

    #[test]
    fn score_title_drops_only_the_extension() {
        assert_eq!(score_title("/a/b/Scene.v2.mp4"), "Scene.v2");
        assert_eq!(score_title("/a/b/Scene."), "Scene.");
    }
}
