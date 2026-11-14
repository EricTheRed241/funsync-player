//! The SLR tab's deck, from two sources: EroScripts waiting posts with a video on SexLikeReal
//! (`deck::Pool::Slr`: cards like the EroScripts deck's, swiped into
//! eroscripts-scout-swipes.json, so `eroscripts:scout --fetch --auto` downloads them from SLR),
//! and the scenes `slr:scout` found (`<userData>/slr-scout.json`) with no swipe in
//! slr-scout-swipes.json and not downloaded yet (`fetchedAt`), minus the scenes a waiting post
//! brings (the post wins: its script is free). One order, highest score first, plus the
//! account's download quota the scout last read (hand-crafted and AI scripts per month, videos
//! per 7 days). Scores, tag scores and the quota come from the scouts; Rust here only deals.

use crate::deck::{self, Deal, TagScore};
use crate::report::{self, id_of, str_of, strings_of};
use crate::swipes::{self, Dir, Swipes};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

pub const SCOUT_FILE: &str = "slr-scout.json";
/// Score points a seeded SLR deal moves each card by, at most, either way: a small, hand-picked
/// pool, so it stays close to score order.
pub const SLR_JITTER: f64 = 2.0;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlrDeck {
    /// Posts and scenes left to swipe (all of them, not just the ones returned).
    pub remaining: usize,
    /// Right swipes on posts still waiting (not in the library) plus right swipes on scenes
    /// in slr-scout.json not downloaded yet.
    pub wanted: usize,
    /// When `slr:scout` wrote the file.
    pub generated_at: Option<String>,
    /// The scout's last quota read: `{checkedAt, scripts, aiScripts, videos}`, each
    /// `{remaining, total, usageExpiresAt}`; null when never read.
    pub quota: Value,
    /// Why the last quota read failed (`no session`, `session expired`, …).
    pub quota_error: Option<String>,
    pub problems: Vec<String>,
    pub cards: Vec<SlrDeckCard>,
}

/// One card of the SLR tab; its `source` says where its swipe goes.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum SlrDeckCard {
    /// An EroScripts post with its video on SLR (`source: "eroscripts"`): `/api/swipe`.
    Post(PostCard),
    /// A scene from slr-scout.json (`source: "slr"`): `/api/slr/swipe`.
    Scene(SlrCard),
}

/// The EroScripts deck's card, marked with its source.
#[derive(Debug, Serialize)]
pub struct PostCard {
    /// Always `eroscripts`.
    pub source: &'static str,
    #[serde(flatten)]
    pub card: deck::Card,
}

impl SlrDeckCard {
    pub fn id(&self) -> u64 {
        match self {
            SlrDeckCard::Post(p) => p.card.id,
            SlrDeckCard::Scene(s) => s.id,
        }
    }

    pub fn score(&self) -> f64 {
        match self {
            SlrDeckCard::Post(p) => p.card.score,
            SlrDeckCard::Scene(s) => s.score,
        }
        .unwrap_or(f64::NEG_INFINITY)
    }

    pub fn is_scene(&self) -> bool {
        matches!(self, SlrDeckCard::Scene(_))
    }

    /// Whether the page holds this card already (`exclude` for posts, `excludeScenes` for scenes).
    fn held(&self, deal: &Deal) -> bool {
        match self {
            SlrDeckCard::Post(p) => deal.exclude.contains(&p.card.id),
            SlrDeckCard::Scene(s) => deal.exclude_scenes.contains(&s.id),
        }
    }
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
    /// The scene's SLR categories (slugs) with the user's tag scores when the scout gave them
    /// (`tagScores`; then highest first, ties by name), else in the file's order, unscored.
    pub tags: Vec<SlrTag>,
    /// The best few positive-scored tags (`deck::top_tags`).
    pub top_tags: Vec<TagScore>,
    /// A favourite performer or studio in the scene.
    pub favourite: Option<String>,
    /// Where a download takes the script from: `eroscripts`, `slr-handcrafted`, `slr-ai`, or
    /// null (none).
    pub script_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SlrTag {
    pub tag: String,
    pub score: Option<f64>,
}

/// Read the waiting list (with the state, library and EroScripts swipes `deck::load_pool`
/// reads), slr-scout.json and the SLR swipes, and deal the cards `deal` asks for (its `stack`
/// is ignored; its `jitter` defaults to SLR_JITTER).
pub fn load(user_data: &Path, deal: &Deal) -> SlrDeck {
    let mut problems = Vec::new();
    let (posts, waiting) = deck::load_pool(user_data, deck::Pool::Slr, &mut problems);
    let scout = report::read_json(user_data, SCOUT_FILE, &mut problems);
    let swipes = report::read_swipes(user_data, swipes::SLR, &mut problems);
    let mut d = build(&scout, &swipes, posts, &deck::slr_scene_ids(&waiting));
    d.deal(deal);
    d.problems = problems;
    d
}

/// Every card: `posts` (the waiting posts with an SLR video, `deck::Pool::Slr`) and the
/// unswiped, not yet downloaded scenes of `scout` whose id isn't in `covered` (the SLR scenes
/// of waiting posts, `deck::slr_scene_ids`), highest score first (ties: newest id first, a
/// post before a scene).
pub fn build(scout: &Value, swipes: &Swipes, posts: deck::Deck, covered: &HashSet<u64>) -> SlrDeck {
    let mut wanted = posts.wanted;
    let mut cards: Vec<SlrDeckCard> = posts
        .cards
        .into_iter()
        .map(|card| {
            SlrDeckCard::Post(PostCard {
                source: "eroscripts",
                card,
            })
        })
        .collect();
    for (id, scene) in scenes(scout) {
        if covered.contains(&id) || str_of(scene, "fetchedAt").is_some() {
            continue;
        }
        match swipes.dir(id) {
            Some(Dir::Right) => wanted += 1,
            Some(Dir::Left) => {}
            None => cards.push(SlrDeckCard::Scene(card(id, scene))),
        }
    }
    sort(&mut cards, SlrDeckCard::score);
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

/// Highest `key` first; ties by newest id, then a post before a scene.
fn sort(cards: &mut [SlrDeckCard], key: impl Fn(&SlrDeckCard) -> f64) {
    cards.sort_by(|a, b| {
        key(b)
            .total_cmp(&key(a))
            .then(b.id().cmp(&a.id()))
            .then(a.is_scene().cmp(&b.is_scene()))
    });
}

impl SlrDeck {
    /// Reorder by `seed` (shifting scores by up to the deal's jitter, default SLR_JITTER),
    /// leave out the posts in `exclude` and the scenes in `exclude_scenes`, and keep the first
    /// `limit`.
    pub fn deal(&mut self, deal: &Deal) {
        if let Some(seed) = deal.seed {
            let amp = deal.jitter.unwrap_or(SLR_JITTER);
            sort(&mut self.cards, |c| {
                c.score() + deck::jitter(seed, c.id(), amp)
            });
        }
        self.cards.retain(|c| !c.held(deal));
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

/// The scene's tags (`tags`) scored by `tagScores` when the scout gave them, else unscored.
fn scene_tags(s: &Value) -> (Vec<SlrTag>, Vec<TagScore>) {
    if s.get("tagScores").is_some_and(Value::is_object) {
        let scored = deck::post_tags(s);
        let top = deck::top_tags(&scored);
        let tags = scored
            .into_iter()
            .map(|t| SlrTag {
                tag: t.tag,
                score: Some(t.score),
            })
            .collect();
        return (tags, top);
    }
    let mut seen = HashSet::new();
    let tags = strings_of(s, "tags")
        .unwrap_or_default()
        .into_iter()
        .filter(|t| !t.is_empty() && seen.insert(t.clone()))
        .map(|tag| SlrTag { tag, score: None })
        .collect();
    (tags, Vec::new())
}

fn card(id: u64, s: &Value) -> SlrCard {
    let url = str_of(s, "url")
        .filter(|u| u.starts_with("https://"))
        .or_else(|| str_of(s, "label").map(|l| format!("https://www.sexlikereal.com/scenes/{l}")))
        .unwrap_or_else(|| format!("https://www.sexlikereal.com/scenes/{id}"));
    let (tags, top_tags) = scene_tags(s);
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
        tags,
        top_tags,
        favourite: str_of(s, "favourite"),
        script_source: str_of(s, "scriptSource"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swipes::Swipe;
    use serde_json::json;
    use std::collections::HashMap;

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
                 "tags": ["creampie", "pov", "blowjob", "pov"], "tagScores": {"creampie": 3, "pov": 1.5, "anal": -2},
                 "favourite": "Miss Lexa", "scriptSource": "slr-ai", "fetchedAt": null,
                 "score": 6.0, "scoreParts": {"vr": 2, "tag:creampie": 3, "tag:anal": -2}},
                {"id": 99137, "url": "https://www.sexlikereal.com/scenes/top-milf-99137", "title": "25 Top Milf Babes",
                 "creampie": true, "compilation": true, "score": 2.15, "vr": true},
                {"id": 51207, "title": "Hot Milf Gets Creampied", "creampie": true, "score": 1.63, "vr": true,
                 "thumbnail": "http://insecure.example/x.jpg", "tags": ["milf", "", "milf", "creampie"]},
                {"id": 70001, "title": "Downloaded", "score": 50.0, "fetchedAt": "2026-11-13T19:00:00.000Z"},
                {"title": "no id"}
            ]
        })
    }

    /// Waiting posts: 300 and 301 have their video on SLR (300 brings scene 99137), 302 is on
    /// MEGA (the Multi-axis / Single-axis stacks' own).
    fn waiting() -> Value {
        let slr = |scene: u64| {
            json!([{"axes": [], "video": {"name": "x_180_LR.mp4", "host": "slr", "slrSceneId": scene,
                "url": format!("https://www.sexlikereal.com/scenes/x-{scene}"), "size": null, "height": 2880}}])
        };
        json!({"candidates": [
            {"id": 300, "title": "Post on SLR", "score": 4.0, "axisType": "single", "tagScores": {"pov": 2},
             "scenes": slr(99137)},
            {"id": 301, "title": "Another post on SLR", "score": 9.0, "scenes": slr(12345)},
            {"id": 302, "title": "MEGA post", "score": 99.0,
             "scenes": [{"video": {"host": "MEGA"}}]}
        ]})
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

    /// The SLR tab's deck: the waiting posts (EroScripts swipes `post_swipes`) and scout()
    /// (SLR swipes `scene_swipes`).
    fn merged(post_swipes: &[(u64, Dir)], scene_swipes: &[(u64, Dir)]) -> SlrDeck {
        let posts = deck::build(
            &json!({}),
            &waiting(),
            &HashMap::new(),
            &swipes(post_swipes),
            deck::Pool::Slr,
        );
        build(
            &scout(),
            &swipes(scene_swipes),
            posts,
            &deck::slr_scene_ids(&waiting()),
        )
    }

    /// (source, id) of each card, in deck order.
    fn order(d: &SlrDeck) -> Vec<(&'static str, u64)> {
        d.cards
            .iter()
            .map(|c| (if c.is_scene() { "slr" } else { "eroscripts" }, c.id()))
            .collect()
    }

    const ALL: [(&str, u64); 4] = [
        ("eroscripts", 301),
        ("slr", 66249),
        ("eroscripts", 300),
        ("slr", 51207),
    ];

    #[test]
    fn deals_posts_and_scenes_in_one_order_without_duplicates() {
        let d = merged(&[], &[]);
        // 99137 comes with post 300 (its script is free); 70001 is downloaded; 302 is on MEGA.
        assert_eq!(order(&d), ALL);
        assert_eq!((d.remaining, d.wanted), (4, 0));
        assert_eq!(d.quota["scripts"]["remaining"], 2);
        assert_eq!(d.quota_error, None);
        assert_eq!(d.generated_at.as_deref(), Some("2026-11-13T18:00:00.000Z"));

        // Each kind counts its own swipes: post 301 wanted, 300 hidden; scene 66249 wanted,
        // 51207 hidden. Right swipes on a downloaded scene, a scene a post brings, or a scene
        // no longer in the file don't count.
        let d = merged(
            &[(301, Dir::Right), (300, Dir::Left)],
            &[
                (66249, Dir::Right),
                (51207, Dir::Left),
                (70001, Dir::Right),
                (99137, Dir::Right),
                (7, Dir::Right),
            ],
        );
        assert!(d.cards.is_empty());
        assert_eq!((d.remaining, d.wanted), (0, 2));

        // An SLR swipe on a post's id, or an EroScripts swipe on a scene's id, is another card's.
        let d = merged(
            &[(66249, Dir::Left)],
            &[(300, Dir::Left), (301, Dir::Right)],
        );
        assert_eq!((d.remaining, d.wanted), (4, 0));
    }

    #[test]
    fn each_card_says_where_its_swipe_goes() {
        let d = merged(&[], &[]);
        let cards: Vec<Value> = d
            .cards
            .iter()
            .map(|c| serde_json::to_value(c).unwrap())
            .collect();
        // A post: the EroScripts deck's card (tag scores, scenes) with its source and SLR link.
        let post = &cards[0];
        assert_eq!(
            (post["source"].as_str(), post["id"].as_u64()),
            (Some("eroscripts"), Some(301))
        );
        assert_eq!(post["url"], "https://discuss.eroscripts.com/t/301");
        assert_eq!(
            post["slrVideo"],
            json!({"sceneId": 12345, "url": "https://www.sexlikereal.com/scenes/x-12345"})
        );
        assert_eq!(post["scenes"][0]["host"], "slr");
        assert_eq!(cards[2]["topTags"], json!([{"tag": "pov", "score": 2.0}]));
        // A scene: source slr.
        assert_eq!(
            (cards[1]["source"].as_str(), cards[1]["id"].as_u64()),
            (Some("slr"), Some(66249))
        );
        assert!(cards[1].get("slrVideo").is_none());
    }

    #[test]
    fn scene_card_fields() {
        let d = merged(&[], &[]);
        let scene = |id: u64| match d.cards.iter().find(|c| c.is_scene() && c.id() == id) {
            Some(SlrDeckCard::Scene(s)) => s,
            _ => panic!("no scene {id}"),
        };
        let c = serde_json::to_value(scene(66249)).unwrap();
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
        assert_eq!(c["scoreParts"]["tag:creampie"], 3);
        assert_eq!(
            (c["creampie"].as_bool(), c["aiScript"].as_bool()),
            (Some(true), Some(true))
        );
        assert_eq!(
            (c["favourite"].as_str(), c["scriptSource"].as_str()),
            (Some("Miss Lexa"), Some("slr-ai"))
        );
        // Scored tags: every tag and tagScores key, highest first; the top ones positive only.
        assert_eq!(
            c["tags"],
            json!([{"tag": "creampie", "score": 3.0}, {"tag": "pov", "score": 1.5},
                   {"tag": "blowjob", "score": 0.0}, {"tag": "anal", "score": -2.0}])
        );
        assert_eq!(
            c["topTags"],
            json!([{"tag": "creampie", "score": 3.0}, {"tag": "pov", "score": 1.5}])
        );
        // No tagScores: the tags unscored, in the file's order, deduped; nothing on top.
        let c = serde_json::to_value(scene(51207)).unwrap();
        assert_eq!(
            c["tags"],
            json!([{"tag": "milf", "score": null}, {"tag": "creampie", "score": null}])
        );
        assert_eq!(c["topTags"], json!([]));
        assert_eq!(
            (&c["favourite"], &c["scriptSource"]),
            (&Value::Null, &Value::Null)
        );
        // Only https images; no URL or label: the id's page.
        assert_eq!(c["images"], json!([]));
        assert_eq!(c["url"], "https://www.sexlikereal.com/scenes/51207");
    }

    #[test]
    fn a_url_from_the_file_wins_over_the_label() {
        let c = card(
            99137,
            &json!({"url": "https://www.sexlikereal.com/scenes/top-milf-99137", "label": "x-99137",
                    "compilation": true}),
        );
        assert_eq!(c.url, "https://www.sexlikereal.com/scenes/top-milf-99137");
        assert!(c.compilation);
        let c = card(99137, &json!({"url": "http://x/1", "label": "x-99137"}));
        assert_eq!(c.url, "https://www.sexlikereal.com/scenes/x-99137");
    }

    #[test]
    fn deal_pages_and_shuffles_a_little() {
        // Posts and scenes are left out by their own ids: topic 66249 isn't scene 66249.
        let mut d = merged(&[], &[]);
        let mut deal = Deal {
            limit: 2,
            ..Deal::default()
        };
        deal.exclude.extend([301, 66249]);
        d.deal(&deal);
        assert_eq!(order(&d), [("slr", 66249), ("eroscripts", 300)]);
        assert_eq!(d.remaining, 4, "remaining counts the whole deck");

        let mut d = merged(&[], &[]);
        deal.exclude_scenes.extend([66249, 51207]);
        d.deal(&deal);
        assert_eq!(order(&d), [("eroscripts", 300)]);

        // A zero jitter keeps score order whatever the seed.
        let mut d = merged(&[], &[]);
        d.deal(&Deal {
            seed: Some(9),
            jitter: Some(0.0),
            ..Deal::default()
        });
        assert_eq!(order(&d), ALL);
        // Equal scores and ids: the post first.
        let mut tie = merged(&[], &[]);
        for c in &mut tie.cards {
            match c {
                SlrDeckCard::Post(p) => p.card.id = 5,
                SlrDeckCard::Scene(s) => s.id = 5,
            }
        }
        sort(&mut tie.cards, |_| 1.0);
        assert!(!tie.cards[0].is_scene() && tie.cards[3].is_scene());

        // The default SLR jitter: a card only passes ones within 2 × SLR_JITTER points.
        let scores: HashMap<(bool, u64), f64> = merged(&[], &[])
            .cards
            .iter()
            .map(|c| ((c.is_scene(), c.id()), c.score()))
            .collect();
        let mut moved = false;
        for seed in 0..50 {
            let mut d = merged(&[], &[]);
            d.deal(&Deal {
                seed: Some(seed),
                ..Deal::default()
            });
            moved |= order(&d) != ALL;
            for (i, a) in d.cards.iter().enumerate() {
                for b in &d.cards[i + 1..] {
                    let sa = scores[&(a.is_scene(), a.id())];
                    let sb = scores[&(b.is_scene(), b.id())];
                    assert!(sb - sa <= 2.0 * SLR_JITTER, "seed {seed}");
                }
            }
        }
        assert!(
            moved,
            "some seed reorders 300 and 51207 (2.37 points apart)"
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
        // No waiting list: 99137 stands on its own.
        assert_eq!(d.remaining, 3);
        assert!(d.problems[0].starts_with(swipes::SLR.file));

        // The waiting list's SLR posts join; the scene a post brings leaves.
        std::fs::write(
            dir.join(report::WAITING_FILE),
            serde_json::to_vec(&waiting()).unwrap(),
        )
        .unwrap();
        std::fs::write(dir.join(swipes::EROSCRIPTS.file), b"bad").unwrap();
        let d = load(&dir, &Deal::default());
        assert_eq!(order(&d), ALL);
        assert!(
            d.problems
                .iter()
                .any(|p| p.starts_with(swipes::EROSCRIPTS.file))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn knows_its_scenes() {
        assert!(is_scene(&scout(), 99137));
        assert!(!is_scene(&scout(), 12));
        assert!(!is_scene(&Value::Null, 99137));
    }
}
