//! The SLR deck: SexLikeReal scenes `slr:scout` found (`<userData>/slr-scout.json`) with no
//! swipe yet (`slr-scout-swipes.json`), highest score first, as cards for the swipe page's
//! SLR tab, plus the account's download quota the scout last read (hand-crafted and AI
//! scripts per month, videos per 7 days). Scores, bonuses and the quota come from the
//! scout; Rust here only deals.

use crate::deck::{self, Deal};
use crate::report::{self, id_of, str_of, strings_of};
use crate::swipes::{self, Dir, Swipes};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

pub const SCOUT_FILE: &str = "slr-scout.json";
/// Score points a seeded SLR deal moves each card by, at most, either way: a small, hand-picked
/// pool, so it stays close to score order.
pub const SLR_JITTER: f64 = 2.0;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlrDeck {
    /// Scenes left to swipe (all of them, not just the ones returned).
    pub remaining: usize,
    /// Right swipes on scenes still in slr-scout.json.
    pub wanted: usize,
    /// When `slr:scout` wrote the file.
    pub generated_at: Option<String>,
    /// The scout's last quota read: `{checkedAt, scripts, aiScripts, videos}`, each
    /// `{remaining, total, usageExpiresAt}`; null when never read.
    pub quota: Value,
    /// Why the last quota read failed (`no session`, `session expired`, …).
    pub quota_error: Option<String>,
    pub problems: Vec<String>,
    pub cards: Vec<SlrCard>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlrCard {
    /// Always `slr`: the page draws these cards differently.
    pub source: &'static str,
    pub id: u64,
    pub url: String,
    pub title: Option<String>,
    /// The studio, shown where a post shows its author.
    pub author: Option<String>,
    pub images: Vec<String>,
    pub score: Option<f64>,
    pub score_parts: Value,
    pub vr: bool,
    pub creampie: bool,
    pub runtime_ms: Option<u64>,
    pub actors: Vec<String>,
    pub date: Option<String>,
    pub fps: Option<f64>,
    pub passthrough: bool,
    /// `{source: "slr", author}` or `{source: "eroscripts", topicId, title, free, multiAxis}`.
    pub handmade: Vec<Value>,
    pub ai_script: bool,
    pub pmv: bool,
    pub compilation: bool,
    pub favorites: Option<i64>,
}

/// Read slr-scout.json and the SLR swipes and deal the cards `deal` asks for (its `stack` is
/// ignored; its `jitter` defaults to SLR_JITTER).
pub fn load(user_data: &Path, deal: &Deal) -> SlrDeck {
    let mut problems = Vec::new();
    let scout = report::read_json(user_data, SCOUT_FILE, &mut problems);
    let swipes = report::read_swipes(user_data, swipes::SLR, &mut problems);
    let mut d = build(&scout, &swipes);
    d.deal(deal);
    d.problems = problems;
    d
}

/// Every unswiped scene, highest score first (ties: newest id first).
pub fn build(scout: &Value, swipes: &Swipes) -> SlrDeck {
    let mut wanted = 0;
    let mut cards = Vec::new();
    for (id, scene) in scenes(scout) {
        match swipes.dir(id) {
            Some(Dir::Right) => wanted += 1,
            Some(Dir::Left) => {}
            None => cards.push(card(id, scene)),
        }
    }
    cards.sort_by(|a, b| score(b).total_cmp(&score(a)).then(b.id.cmp(&a.id)));
    SlrDeck {
        remaining: cards.len(),
        wanted,
        generated_at: str_of(scout, "generatedAt"),
        quota: scout.get("quota").cloned().unwrap_or(Value::Null),
        quota_error: str_of(scout, "quotaError"),
        problems: Vec::new(),
        cards,
    }
}

fn score(c: &SlrCard) -> f64 {
    c.score.unwrap_or(f64::NEG_INFINITY)
}

impl SlrDeck {
    /// Reorder by `seed` (shifting scores by up to the deal's jitter, default SLR_JITTER),
    /// leave out `exclude`, and keep the first `limit`.
    pub fn deal(&mut self, deal: &Deal) {
        if let Some(seed) = deal.seed {
            let amp = deal.jitter.unwrap_or(SLR_JITTER);
            let key = |c: &SlrCard| score(c) + deck::jitter(seed, c.id, amp);
            self.cards
                .sort_by(|a, b| key(b).total_cmp(&key(a)).then(b.id.cmp(&a.id)));
        }
        self.cards.retain(|c| !deal.exclude.contains(&c.id));
        self.cards.truncate(deal.limit);
    }
}

fn scenes(scout: &Value) -> impl Iterator<Item = (u64, &Value)> {
    scout
        .get("scenes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| Some((id_of(s.get("id")?)?, s)))
}

/// Whether `id` is a scene in slr-scout.json.
pub fn is_scene(scout: &Value, id: u64) -> bool {
    scenes(scout).any(|(s, _)| s == id)
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn card(id: u64, s: &Value) -> SlrCard {
    let url = str_of(s, "url")
        .filter(|u| u.starts_with("https://"))
        .or_else(|| str_of(s, "label").map(|l| format!("https://www.sexlikereal.com/scenes/{l}")))
        .unwrap_or_else(|| format!("https://www.sexlikereal.com/scenes/{id}"));
    SlrCard {
        source: "slr",
        id,
        url,
        title: str_of(s, "title"),
        author: str_of(s, "studio"),
        images: str_of(s, "thumbnail")
            .filter(|u| u.starts_with("https://"))
            .into_iter()
            .collect(),
        score: s.get("score").and_then(Value::as_f64),
        score_parts: s.get("scoreParts").cloned().unwrap_or(Value::Null),
        vr: flag(s, "vr"),
        creampie: flag(s, "creampie"),
        runtime_ms: s
            .get("durationS")
            .and_then(Value::as_f64)
            .filter(|n| *n > 0.0)
            .map(|n| (n * 1000.0).round() as u64),
        actors: strings_of(s, "actors").unwrap_or_default(),
        date: str_of(s, "date"),
        fps: s.get("fps").and_then(Value::as_f64),
        passthrough: flag(s, "passthrough"),
        handmade: s
            .get("handmade")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        ai_script: flag(s, "aiScript"),
        pmv: flag(s, "pmv"),
        compilation: flag(s, "compilation"),
        favorites: s.get("favorites").and_then(Value::as_i64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swipes::Swipe;
    use serde_json::json;

    fn scout() -> Value {
        json!({
            "generatedAt": "2026-11-13T18:00:00.000Z",
            "quota": {"checkedAt": "2026-11-13T18:00:00.000Z",
                "scripts": {"remaining": 2, "total": 3, "usageExpiresAt": null},
                "aiScripts": {"remaining": 3, "total": 3, "usageExpiresAt": null},
                "videos": {"remaining": 35, "total": 35, "usageExpiresAt": null}},
            "quotaError": null,
            "scenes": [
                {"id": 66249, "label": "he-woke-me-up-66249", "title": "He Woke Me Up To Creampie Me",
                 "studio": "Miss Lexa VR", "actors": ["Miss Lexa"], "date": "2025-09-08", "durationS": 1673,
                 "thumbnail": "https://cdn-vr.sexlikereal.com/images/66249/cover.webp", "vr": true, "fps": 60,
                 "passthrough": false, "creampie": true,
                 "handmade": [{"source": "eroscripts", "topicId": 282260, "title": "x", "free": false, "multiAxis": false}],
                 "aiScript": true, "pmv": false, "compilation": false, "favorites": 2016,
                 "score": 6.0, "scoreParts": {"handmadeScript": 4, "popularity": 2}},
                {"id": 99137, "url": "https://www.sexlikereal.com/scenes/top-milf-99137", "title": "25 Top Milf Babes",
                 "creampie": true, "compilation": true, "score": 2.15, "vr": true},
                {"id": 51207, "title": "Hot Milf Gets Creampied", "creampie": true, "score": 1.63, "vr": true,
                 "thumbnail": "http://insecure.example/x.jpg"},
                {"title": "no id"}
            ]
        })
    }

    fn swipes(entries: &[(u64, Dir)]) -> Swipes {
        let mut s = Swipes::default();
        for (id, dir) in entries {
            s.swipes.insert(
                *id,
                Swipe {
                    dir: *dir,
                    at: "x".into(),
                },
            );
        }
        s
    }

    #[test]
    fn deals_unswiped_scenes_best_first_with_the_quota() {
        let d = build(&scout(), &swipes(&[]));
        assert_eq!(
            d.cards.iter().map(|c| c.id).collect::<Vec<_>>(),
            [66249, 99137, 51207]
        );
        assert_eq!((d.remaining, d.wanted), (3, 0));
        assert_eq!(d.quota["scripts"]["remaining"], 2);
        assert_eq!(d.quota_error, None);
        assert_eq!(d.generated_at.as_deref(), Some("2026-11-13T18:00:00.000Z"));

        let d = build(
            &scout(),
            &swipes(&[(66249, Dir::Right), (51207, Dir::Left), (7, Dir::Right)]),
        );
        assert_eq!(d.cards.iter().map(|c| c.id).collect::<Vec<_>>(), [99137]);
        // A right swipe on a scene no longer in the file doesn't count.
        assert_eq!((d.remaining, d.wanted), (1, 1));
    }

    #[test]
    fn card_fields() {
        let d = build(&scout(), &swipes(&[]));
        let c = serde_json::to_value(&d.cards[0]).unwrap();
        assert_eq!(c["source"], "slr");
        assert_eq!(
            c["url"],
            "https://www.sexlikereal.com/scenes/he-woke-me-up-66249"
        );
        assert_eq!(c["author"], "Miss Lexa VR");
        assert_eq!(
            c["images"],
            json!(["https://cdn-vr.sexlikereal.com/images/66249/cover.webp"])
        );
        assert_eq!(c["runtimeMs"], 1_673_000);
        assert_eq!(c["handmade"][0]["topicId"], 282260);
        assert_eq!(c["scoreParts"]["handmadeScript"], 4);
        assert_eq!(
            (c["creampie"].as_bool(), c["aiScript"].as_bool()),
            (Some(true), Some(true))
        );
        // A URL from the file wins over the label; only https images are shown.
        let c = &d.cards[1];
        assert_eq!(c.url, "https://www.sexlikereal.com/scenes/top-milf-99137");
        assert!(c.compilation);
        assert!(d.cards[2].images.is_empty());
        assert_eq!(d.cards[2].url, "https://www.sexlikereal.com/scenes/51207");
    }

    #[test]
    fn deal_pages_and_shuffles_a_little() {
        let mut d = build(&scout(), &swipes(&[]));
        let mut deal = Deal {
            limit: 1,
            ..Deal::default()
        };
        deal.exclude.insert(66249);
        d.deal(&deal);
        assert_eq!(d.cards.iter().map(|c| c.id).collect::<Vec<_>>(), [99137]);
        assert_eq!(d.remaining, 3, "remaining counts the whole deck");

        // A zero jitter keeps score order whatever the seed.
        let mut d = build(&scout(), &swipes(&[]));
        d.deal(&Deal {
            seed: Some(9),
            jitter: Some(0.0),
            ..Deal::default()
        });
        assert_eq!(
            d.cards.iter().map(|c| c.id).collect::<Vec<_>>(),
            [66249, 99137, 51207]
        );
    }

    #[test]
    fn missing_or_broken_files_are_problems_not_errors() {
        let dir = std::env::temp_dir().join(format!("scout-report-slr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = load(&dir, &Deal::default());
        assert_eq!((d.remaining, d.problems.len()), (0, 0));
        assert!(d.quota.is_null());

        std::fs::write(dir.join(SCOUT_FILE), b"{oops").unwrap();
        let d = load(&dir, &Deal::default());
        assert!(d.problems[0].starts_with(SCOUT_FILE));

        std::fs::write(dir.join(SCOUT_FILE), serde_json::to_vec(&scout()).unwrap()).unwrap();
        std::fs::write(dir.join(swipes::SLR.file), b"nope").unwrap();
        let d = load(&dir, &Deal::default());
        assert_eq!(d.remaining, 3);
        assert!(d.problems[0].starts_with(swipes::SLR.file));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn knows_its_scenes() {
        assert!(is_scene(&scout(), 99137));
        assert!(!is_scene(&scout(), 12));
        assert!(!is_scene(&Value::Null, 99137));
    }
}
