//! A scene's score: the bonuses that apply (hand-made script, PMV,
//! compilation) plus popularity, `min(favorites / popularityPer,
//! popularityMax)`, rounded to 2 decimals. `scoreParts` lists the non-zero
//! parts.

use crate::prefs::SlrScout;
use crate::store::Scene;
use std::cmp::Ordering;
use std::collections::BTreeMap;

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Set `score` and `scoreParts`.
pub fn score(prefs: &SlrScout, s: &mut Scene) {
    let b = &prefs.bonuses;
    let popularity = (s.favorites as f64 / prefs.popularity_per).min(prefs.popularity_max);
    let parts = [
        ("handmadeScript", !s.handmade.is_empty(), b.handmade_script),
        ("pmv", s.pmv, b.pmv),
        ("compilation", s.compilation, b.compilation),
        ("popularity", true, popularity),
    ];
    s.score_parts = BTreeMap::new();
    let mut total = 0.0;
    for (name, applies, value) in parts {
        let value = round2(value);
        if applies && value != 0.0 {
            total += value;
            s.score_parts.insert(name.to_string(), value);
        }
    }
    s.score = round2(total);
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
    use crate::store::Handmade;
    use serde_json::json;

    fn prefs() -> SlrScout {
        crate::prefs::parse(
            r#"{"slrScout": {"actors": ["Miss Lexa"], "vrOnly": true, "requireCreampie": true,
            "bonuses": {"handmadeScript": 4, "pmv": 2, "compilation": 2},
            "popularityPer": 1000, "popularityMax": 2}}"#,
        )
        .unwrap()
    }

    fn scene(favorites: u64) -> Scene {
        let mut s = crate::slr::scene(&json!({"id": 1, "label": "x-1", "title": "X"})).unwrap();
        s.favorites = favorites;
        s
    }

    fn parts(s: &Scene) -> Vec<(&str, f64)> {
        s.score_parts
            .iter()
            .map(|(k, v)| (k.as_str(), *v))
            .collect()
    }

    #[test]
    fn adds_bonuses_and_popularity() {
        let mut s = scene(1123);
        s.handmade.push(Handmade::Slr {
            author: "kimera".into(),
        });
        score(&prefs(), &mut s);
        assert_eq!(s.score, 5.12);
        assert_eq!(parts(&s), [("handmadeScript", 4.0), ("popularity", 1.12)]);

        let mut s = scene(500);
        s.pmv = true;
        s.compilation = true;
        score(&prefs(), &mut s);
        assert_eq!(s.score, 4.5);
        assert_eq!(
            parts(&s),
            [("compilation", 2.0), ("pmv", 2.0), ("popularity", 0.5)]
        );
    }

    #[test]
    fn popularity_is_capped_and_zero_parts_are_left_out() {
        let mut s = scene(25_000);
        score(&prefs(), &mut s);
        assert_eq!(s.score, 2.0);
        assert_eq!(parts(&s), [("popularity", 2.0)]);
        let mut s = scene(0);
        score(&prefs(), &mut s);
        assert_eq!(s.score, 0.0);
        assert!(s.score_parts.is_empty());
        let mut s = scene(1);
        score(&prefs(), &mut s);
        assert_eq!(s.score, 0.0);
    }

    #[test]
    fn sorts_by_score_then_favorites() {
        let mut a = scene(100);
        a.id = 1;
        a.score = 2.0;
        let mut b = scene(900);
        b.id = 2;
        b.score = 2.0;
        let mut c = scene(10);
        c.id = 3;
        c.score = 5.0;
        let mut all = vec![a, b, c];
        sort(&mut all);
        assert_eq!(all.iter().map(|s| s.id).collect::<Vec<_>>(), [3, 2, 1]);
    }
}
