//! The swipe deck: the waiting list's posts with no video in the library and
//! no swipe yet (posts with a video on SexLikeReal go to the SLR tab instead: `Pool`, slr.rs),
//! highest score first (the scout's download order), as cards
//! for `/swipe`; with a `seed`, each card's score is shifted by up to the deal's jitter
//! (the request's `jitter`, else the stack's MULTI_JITTER or SINGLE_JITTER) either way, so
//! the order wanders a little. Tag scores and creampie come from the scout's waiting list
//! (`tagScores`, `creampie`); Rust never computes them.

use crate::report::{self, id_of, str_of, strings_of};
use crate::swipes::{Dir, Swipes};
use serde::{Serialize, Serializer};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 100;
/// Most ids one deck request may leave out (`exclude`: the cards the page holds).
pub const MAX_EXCLUDE: usize = 1000;
/// Score points a seeded multi-axis deck moves each card by, at most, either way: multi-axis
/// posts are a smaller, more valued pool, so the deck stays close to score order.
pub const MULTI_JITTER: f64 = 4.0;
/// Score points a seeded single-axis deck moves each card by, at most, either way: wide enough
/// (scores run about -4 … 24) that low-scoring posts surface too and the whole stack gets swiped.
pub const SINGLE_JITTER: f64 = 10.0;
/// Largest jitter a deck request may ask for (`jitter=N`), in score points.
pub const MAX_JITTER: f64 = 30.0;
/// Images per card (the scout saves at most this many).
pub const MAX_IMAGES: usize = 5;
/// Scenes listed per card; the rest only count (`moreScenes`).
pub const MAX_SCENES: usize = 20;
const TOP_TAGS: usize = 3;

/// Which posts a deck request deals: multi-axis (the scout's `axisType`; packed archives count
/// as multi) or single-axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stack {
    #[default]
    Multi,
    Single,
}

impl Stack {
    pub fn parse(s: &str) -> Option<Stack> {
        match s {
            "multi" => Some(Stack::Multi),
            "single" => Some(Stack::Single),
            _ => None,
        }
    }

    /// Most score points a seeded deal of this stack moves a card, either way.
    pub fn jitter(self) -> f64 {
        match self {
            Stack::Multi => MULTI_JITTER,
            Stack::Single => SINGLE_JITTER,
        }
    }
}

/// What one deck request asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct Deal {
    pub limit: usize,
    /// Shuffle a little (by `jitter`); the same seed deals the same order, so the pages of
    /// one visit agree. None = strictly by score.
    pub seed: Option<u64>,
    /// Score points a seeded deal moves each card by, at most, either way (0 ..= MAX_JITTER;
    /// 0 = strictly by score). None = the stack's own (`Stack::jitter`).
    pub jitter: Option<f64>,
    /// Posts the page holds already (on screen, or skipped for later), by topic id.
    pub exclude: HashSet<u64>,
    /// SLR scenes the page holds already, by SLR scene id (the SLR tab only).
    pub exclude_scenes: HashSet<u64>,
    pub stack: Stack,
}

impl Default for Deal {
    fn default() -> Self {
        Deal {
            limit: DEFAULT_LIMIT,
            seed: None,
            jitter: None,
            exclude: HashSet::new(),
            exclude_scenes: HashSet::new(),
            stack: Stack::default(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Deck {
    /// Cards left to swipe (all of them, not just the ones returned).
    pub remaining: usize,
    /// Right swipes still on the waiting list and not in the library: what the next
    /// `--fetch --auto` downloads (it takes nothing else).
    pub wanted: usize,
    /// Files that exist but couldn't be read or parsed.
    pub problems: Vec<String>,
    /// The first `limit` cards, in deck order.
    pub cards: Vec<Card>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: u64,
    pub url: String,
    pub title: Option<String>,
    pub author: Option<String>,
    /// Up to MAX_IMAGES https URLs: the state's `images`, else its `image`.
    pub images: Vec<String>,
    pub score: Option<f64>,
    pub vr: bool,
    pub axis_type: Option<String>,
    pub axes: Axes,
    /// Scenes (one video each; a packed archive counts as one).
    pub videos: usize,
    pub creampie: bool,
    pub top_tags: Vec<TagScore>,
    /// Every tag of the post with its tag score (`tagScores`, else 0): highest first, ties by name.
    pub tags: Vec<TagScore>,
    /// The first video's length: its main script's (`durationMs`, set by the scout),
    /// else the host's listed video length.
    pub runtime_ms: Option<u64>,
    /// The first video's main-script speed per time bin, in position units per second
    /// (`heat`, set by the scout from renderer/js/heatmap-strip.js computeBins); empty = unknown.
    pub heat: Vec<u32>,
    pub likes: Option<i64>,
    /// Hosts of the post's videos.
    pub hosts: Vec<String>,
    /// Total size of the videos whose size is known, in GB (10^9 bytes).
    pub gb: Option<f64>,
    /// When the topic was created and last active (the state's `created`, `bumped`).
    pub created: Option<String>,
    pub bumped: Option<String>,
    /// The first MAX_SCENES scenes.
    pub scenes: Vec<SceneInfo>,
    /// Scenes left out of `scenes`.
    pub more_scenes: usize,
    /// The first scene whose video is on SexLikeReal (only on the SLR tab's posts).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slr_video: Option<SlrVideo>,
}

/// A post's video on SexLikeReal (the scout's `scene.video` with `host: "slr"`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlrVideo {
    pub scene_id: Option<u64>,
    /// The scene's page (https only).
    pub url: Option<String>,
}

/// Which waiting posts a deck holds: the ones whose videos are all elsewhere (the Multi-axis
/// and Single-axis stacks), or the ones with a video on SexLikeReal (the SLR tab). Each post
/// shows in one tab only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pool {
    Hosts,
    Slr,
}

/// One scene (video) of a post, as the card lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneInfo {
    /// The video's file name, else the scene's base name.
    pub name: Option<String>,
    pub host: Option<String>,
    /// Bytes, when the host lists them.
    pub size: Option<u64>,
    pub runtime_ms: Option<u64>,
    /// Axes the scene scripts besides stroke (unknown for a packed archive).
    pub axes: Vec<String>,
}

/// Axes of the post: 1 (stroke) plus every other axis its scenes script, or
/// `"packed"` when every scene is still an archive (axes unknown).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axes {
    Count(usize),
    Packed,
}

impl Serialize for Axes {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Axes::Count(n) => s.serialize_u64(*n as u64),
            Axes::Packed => s.serialize_str("packed"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TagScore {
    pub tag: String,
    pub score: f64,
}

/// Read the waiting list, the state, the library (`config.json`,
/// `eroscripts-links.json`) and the swipes, and deal the cards `deal` asks for.
pub fn load(user_data: &Path, deal: &Deal) -> Deck {
    let mut problems = Vec::new();
    let (mut deck, _) = load_pool(user_data, Pool::Hosts, &mut problems);
    deck.deal(deal);
    deck.problems = problems;
    deck
}

/// Read the files `load` reads and build `pool`'s deck (not dealt; problems go to `problems`),
/// with the waiting list it came from.
pub fn load_pool(user_data: &Path, pool: Pool, problems: &mut Vec<String>) -> (Deck, Value) {
    let state = report::read_json(user_data, report::STATE_FILE, problems);
    let waiting = report::read_json(user_data, report::WAITING_FILE, problems);
    let config = report::read_json(user_data, "config.json", problems);
    let links = report::read_json(user_data, crate::library::LINKS_FILE, problems);
    let swipes = report::read_swipes(user_data, crate::swipes::EROSCRIPTS, problems);
    let videos = crate::library::topic_videos(&config, &links, problems);
    (build(&state, &waiting, &videos, &swipes, pool), waiting)
}

impl Deck {
    /// Reorder by `seed` (shifting scores by up to the deal's jitter), keep `stack`'s posts
    /// (`remaining` then counts that stack), leave out `exclude`, and keep the first `limit`.
    pub fn deal(&mut self, deal: &Deal) {
        if let Some(seed) = deal.seed {
            let amp = deal.jitter.unwrap_or_else(|| deal.stack.jitter());
            let key = |c: &Card| {
                c.score
                    .map_or(f64::NEG_INFINITY, |s| s + jitter(seed, c.id, amp))
            };
            self.cards
                .sort_by(|a, b| key(b).total_cmp(&key(a)).then(b.id.cmp(&a.id)));
        }
        match deal.stack {
            Stack::Multi => self.cards.retain(Card::is_multi),
            Stack::Single => self.cards.retain(|c| !c.is_multi()),
        }
        self.remaining = self.cards.len();
        self.cards.retain(|c| !deal.exclude.contains(&c.id));
        self.cards.truncate(deal.limit);
    }
}

impl Card {
    pub fn is_multi(&self) -> bool {
        self.axis_type.as_deref() == Some("multi")
    }
}

/// Card `id`'s score shift for `seed`, in [-amplitude, amplitude] (splitmix64 of both):
/// the same for a seed and id on every request.
pub fn jitter(seed: u64, id: u64, amplitude: f64) -> f64 {
    let mut z = (seed ^ id.wrapping_mul(0x9E37_79B9_7F4A_7C15)).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    let unit = (z >> 11) as f64 / (1u64 << 53) as f64; // [0, 1)
    (unit * 2.0 - 1.0) * amplitude
}

fn candidates(waiting: &Value) -> impl Iterator<Item = (u64, &Value)> {
    waiting
        .get("candidates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| Some((id_of(c.get("id")?)?, c)))
}

/// Whether `id` is on the waiting list.
pub fn is_waiting(waiting: &Value, id: u64) -> bool {
    candidates(waiting).any(|(c, _)| c == id)
}

/// The scout's videos on SexLikeReal: the scenes' `video` objects with `host: "slr"`.
fn slr_videos(cand: &Value) -> impl Iterator<Item = &Value> {
    cand.get("scenes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| s.get("video"))
        .filter(|v| v.get("host").and_then(Value::as_str) == Some("slr"))
}

fn has_slr_video(cand: &Value) -> bool {
    slr_videos(cand).next().is_some()
}

/// The SLR scene ids of every waiting post's videos (`slrSceneId`), swiped, in the library
/// or not: those scenes come with the post (its script is free), never on their own.
pub fn slr_scene_ids(waiting: &Value) -> HashSet<u64> {
    candidates(waiting)
        .flat_map(|(_, c)| slr_videos(c))
        .filter_map(|v| id_of(v.get("slrSceneId")?))
        .collect()
}

/// Every card of `pool`, in deck order (`problems` empty).
pub fn build(
    state: &Value,
    waiting: &Value,
    videos: &HashMap<u64, usize>,
    swipes: &Swipes,
    pool: Pool,
) -> Deck {
    let shared = image_counts(state);
    let mut wanted = 0;
    let mut cards = Vec::new();
    for (id, cand) in candidates(waiting) {
        if videos.get(&id).is_some_and(|&n| n > 0) || has_slr_video(cand) != (pool == Pool::Slr) {
            continue;
        }
        match swipes.dir(id) {
            Some(Dir::Right) => wanted += 1,
            Some(Dir::Left) => {}
            None => cards.push(card(
                id,
                cand,
                state.pointer(&format!("/topics/{id}")),
                &shared,
            )),
        }
    }
    cards.sort_by(|a, b| {
        let score = |c: &Card| c.score.unwrap_or(f64::NEG_INFINITY);
        score(b).total_cmp(&score(a)).then(b.id.cmp(&a.id))
    });
    Deck {
        remaining: cards.len(),
        wanted,
        problems: Vec::new(),
        cards,
    }
}

fn card(id: u64, cand: &Value, rec: Option<&Value>, shared: &HashMap<&str, usize>) -> Card {
    let rec = rec.unwrap_or(&Value::Null);
    let scenes: &[Value] = cand
        .get("scenes")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let tags = post_tags(cand);
    Card {
        id,
        url: format!("{}{id}", report::FORUM_TOPIC),
        title: str_of(rec, "title").or_else(|| str_of(cand, "title")),
        author: str_of(rec, "author").or_else(|| str_of(cand, "author")),
        images: images(rec, shared),
        score: cand.get("score").and_then(Value::as_f64),
        vr: cand.get("vr").and_then(Value::as_bool).unwrap_or(false),
        axis_type: str_of(cand, "axisType"),
        axes: axes(scenes),
        videos: scenes.len(),
        creampie: cand
            .get("creampie")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        top_tags: top_tags(&tags),
        tags,
        runtime_ms: scenes.first().and_then(runtime_ms),
        heat: scenes.first().map(heat).unwrap_or_default(),
        likes: cand.get("likes").and_then(Value::as_i64),
        hosts: strings_of(cand, "hosts").unwrap_or_default(),
        gb: report::total_gb(scenes),
        created: str_of(rec, "created"),
        bumped: str_of(rec, "bumped"),
        scenes: scenes.iter().take(MAX_SCENES).map(scene_info).collect(),
        more_scenes: scenes.len().saturating_sub(MAX_SCENES),
        slr_video: slr_videos(cand).next().map(|v| SlrVideo {
            scene_id: v.get("slrSceneId").and_then(id_of),
            url: str_of(v, "url").filter(|u| u.starts_with("https://")),
        }),
    }
}

fn scene_info(scene: &Value) -> SceneInfo {
    let video = scene.get("video").unwrap_or(&Value::Null);
    SceneInfo {
        name: str_of(video, "name").or_else(|| str_of(scene, "base")),
        host: str_of(video, "host"),
        size: video.get("size").and_then(Value::as_u64),
        runtime_ms: runtime_ms(scene),
        axes: strings_of(scene, "axes").unwrap_or_default(),
    }
}

fn runtime_ms(scene: &Value) -> Option<u64> {
    let ms = scene
        .get("durationMs")
        .and_then(Value::as_f64)
        .or_else(|| {
            scene
                .pointer("/video/duration")
                .and_then(Value::as_f64)
                .map(|s| s * 1000.0)
        })?;
    (ms.is_finite() && ms > 0.0).then_some(ms.round() as u64)
}

/// Longest heat strip passed on (the scout stores 100 bins).
const MAX_HEAT_BINS: usize = 200;

fn heat(scene: &Value) -> Vec<u32> {
    scene
        .get("heat")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_HEAT_BINS)
        .map(|v| {
            v.as_f64()
                .filter(|f| f.is_finite() && *f > 0.0)
                .map_or(0, |f| f.round() as u32)
        })
        .collect()
}

/// An upload shown by this many judged posts or more is the poster's branding (Patreon
/// banner, host logo, signature), not a scene preview: the deck leaves it out.
pub const SHARED_IMAGE_POSTS: usize = 3;

/// An upload's identity: the SHA-1 Discourse puts in its file name (the original and its
/// optimized copies share it), else the whole URL.
fn image_key(url: &str) -> &str {
    let name = url.rsplit('/').next().unwrap_or(url);
    let hex = name.bytes().take_while(u8::is_ascii_hexdigit).count();
    if hex == 40 { &name[..40] } else { url }
}

/// The image URLs a state record lists: `images`, else (records from before images were
/// kept) its single `image`.
fn record_images(rec: &Value) -> Vec<&str> {
    match rec.get("images").and_then(Value::as_array) {
        Some(list) => list.iter().filter_map(Value::as_str).collect(),
        None => rec
            .get("image")
            .and_then(Value::as_str)
            .into_iter()
            .collect(),
    }
}

/// How many judged posts show each upload (image_key).
fn image_counts(state: &Value) -> HashMap<&str, usize> {
    let mut counts = HashMap::new();
    for rec in state
        .get("topics")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|t| t.values())
    {
        let keys: BTreeSet<&str> = record_images(rec).into_iter().map(image_key).collect();
        for key in keys {
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    counts
}

/// The record's images (record_images): https only, deduped, without uploads that
/// SHARED_IMAGE_POSTS or more posts show, at most MAX_IMAGES. An empty `images` stays
/// empty: the scout found nothing worth showing.
fn images(rec: &Value, shared: &HashMap<&str, usize>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for url in record_images(rec) {
        if out.len() == MAX_IMAGES {
            break;
        }
        let key = image_key(url);
        if !url.starts_with("https://")
            || shared.get(key).is_some_and(|&n| n >= SHARED_IMAGE_POSTS)
            || out.iter().any(|u| image_key(u) == key)
        {
            continue;
        }
        out.push(url.to_string());
    }
    out
}

fn is_packed(scene: &Value) -> bool {
    scene.get("packed").and_then(Value::as_bool) == Some(true)
}

fn axes(scenes: &[Value]) -> Axes {
    if !scenes.is_empty() && scenes.iter().all(is_packed) {
        return Axes::Packed;
    }
    let extra: BTreeSet<&str> = scenes
        .iter()
        .filter(|s| !is_packed(s))
        .filter_map(|s| s.get("axes").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    Axes::Count(1 + extra.len())
}

/// Every tag of the post (`tags`, plus any other `tagScores` key) with its tag score, else 0:
/// highest first, ties by name.
pub(crate) fn post_tags(cand: &Value) -> Vec<TagScore> {
    let scores = cand.get("tagScores").and_then(Value::as_object);
    let names = strings_of(cand, "tags")
        .unwrap_or_default()
        .into_iter()
        .chain(scores.into_iter().flat_map(|m| m.keys().cloned()));
    let mut seen = HashSet::new();
    let mut tags: Vec<TagScore> = names
        .filter(|t| !t.is_empty() && seen.insert(t.clone()))
        .map(|tag| {
            let score = scores
                .and_then(|m| m.get(&tag))
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            TagScore { tag, score }
        })
        .collect();
    tags.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.tag.cmp(&b.tag)));
    tags
}

/// The post's best TOP_TAGS tags (`tags` comes sorted from post_tags): positive scores only.
pub(crate) fn top_tags(tags: &[TagScore]) -> Vec<TagScore> {
    tags.iter()
        .filter(|t| t.score > 0.0)
        .take(TOP_TAGS)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swipes::Swipe;
    use serde_json::json;

    fn waiting() -> Value {
        json!({"candidates": [
            {"id": 10, "title": "Ten", "author": "a", "score": 5.0, "vr": true, "axisType": "multi",
             "creampie": true, "likes": 42, "hosts": ["pixeldrain", "MEGA"],
             "tags": ["pov", "vr", "anal", "solo", "", "pov"],
             "tagScores": {"pov": 2, "vr": 1, "anal": -3, "toys": 0.5},
             "scenes": [{"axes": ["twist", "roll"], "durationMs": 754321, "heat": [0, 120.4, -5, "x", 480],
                         "video": {"duration": 9, "name": "Ten.mp4", "host": "pixeldrain", "size": 1_500_000_000u64}},
                        {"base": "Ten B", "axes": ["roll", "surge"]}]},
            {"id": 11, "title": "Eleven", "score": 5.0, "axisType": "single",
             "scenes": [{"axes": [], "video": {"duration": 91.4}}]},
            {"id": 12, "title": "In library", "score": 99.0},
            {"id": 13, "title": "Wanted", "score": 1.0},
            {"id": 14, "title": "Hidden", "score": 50.0},
            {"id": "15", "title": "No score"},
            {"id": 16, "title": "Low", "score": -2.5, "scenes": [{"packed": true, "axes": []}]},
            {"id": "x", "title": "Bad id", "score": 7.0},
            {"id": 17, "title": "On SLR", "score": 80.0, "axisType": "multi", "tagScores": {"pov": 2},
             "scenes": [{"axes": ["twist"], "video": {"name": "A - B (C)_180_LR.mp4", "host": "slr", "slrSceneId": 66249,
                         "slrLabel": "b-66249", "url": "https://www.sexlikereal.com/scenes/b-66249", "size": null, "height": 2880}}]},
            {"id": 18, "title": "SLR and MEGA", "score": 3.0, "axisType": "single",
             "scenes": [{"video": {"name": "m.mp4", "host": "MEGA"}},
                        {"video": {"host": "slr", "slrSceneId": "51207", "url": "http://insecure/x"}}]},
            {"id": 19, "title": "SLR in library", "score": 9.0,
             "scenes": [{"video": {"host": "slr", "slrSceneId": 70000}}]}
        ]})
    }

    fn state() -> Value {
        json!({"topics": {
            "10": {"title": "Ten from state", "image": "https://x/a.jpg",
                   "created": "2026-01-02T00:00:00Z", "bumped": "2026-03-04T00:00:00Z",
                   "images": ["https://x/a.jpg", "http://x/insecure.jpg", "https://x/a.jpg", "https://x/b.jpg",
                              "https://x/c.jpg", "https://x/d.jpg", "https://x/e.jpg", "https://x/f.jpg"]},
            "11": {"image": "https://x/only.jpg"},
            "16": {"image": "javascript:alert(1)"}
        }})
    }

    fn swipes() -> Swipes {
        let mut s = Swipes::default();
        for (id, dir) in [
            (13, Dir::Right),
            (14, Dir::Left),
            (12, Dir::Right),
            (18, Dir::Right),
        ] {
            s.swipes.insert(
                id,
                Swipe {
                    dir,
                    at: "2026-11-13T00:00:00.000Z".into(),
                },
            );
        }
        s
    }

    fn deck() -> Deck {
        pool(Pool::Hosts)
    }

    fn pool(pool: Pool) -> Deck {
        build(
            &state(),
            &waiting(),
            &HashMap::from([(12, 1), (11, 0), (19, 1)]),
            &swipes(),
            pool,
        )
    }

    #[test]
    fn posts_with_an_slr_video_go_to_the_slr_pool_only() {
        // The Multi-axis / Single-axis stacks never see 17, 18 or 19, nor count 18's right swipe.
        let hosts = deck();
        assert!(
            hosts
                .cards
                .iter()
                .all(|c| c.id < 17 && c.slr_video.is_none())
        );
        assert_eq!(hosts.wanted, 1);
        for stack in [Stack::Multi, Stack::Single] {
            let mut d = deck();
            d.deal(&Deal {
                stack,
                ..Deal::default()
            });
            assert!(d.cards.iter().all(|c| c.id < 17), "{stack:?}");
        }
        // The SLR pool: 17 (unswiped), 18 wanted, 19 in the library.
        let slr = pool(Pool::Slr);
        assert_eq!(ids(&slr), [17]);
        assert_eq!((slr.remaining, slr.wanted), (1, 1));
        let c = serde_json::to_value(&slr.cards[0]).unwrap();
        assert_eq!(
            c["slrVideo"],
            json!({"sceneId": 66249, "url": "https://www.sexlikereal.com/scenes/b-66249"})
        );
        assert_eq!(c["scenes"][0]["host"], "slr");
        assert_eq!(c["topTags"], json!([{"tag": "pov", "score": 2.0}]));
        // Every waiting post's SLR scenes, whatever its swipe or library state; ids as strings too.
        assert_eq!(
            slr_scene_ids(&waiting()),
            HashSet::from([66249, 51207, 70000])
        );
        assert!(slr_scene_ids(&Value::Null).is_empty());
        // Only https links.
        let mut no_swipes = swipes();
        no_swipes.swipes.remove(&18);
        let d = build(&state(), &waiting(), &HashMap::new(), &no_swipes, Pool::Slr);
        let c = d.cards.iter().find(|c| c.id == 18).unwrap();
        assert_eq!(
            c.slr_video,
            Some(SlrVideo {
                scene_id: Some(51207),
                url: None
            })
        );
        assert_eq!(ids(&d), [17, 19, 18]);
    }

    #[test]
    fn deals_unswiped_posts_not_in_library_best_first() {
        let d = deck();
        // 12 is in the library, 13 wanted, 14 hidden; equal scores by newest id; no score last.
        assert_eq!(
            d.cards.iter().map(|c| c.id).collect::<Vec<_>>(),
            [11, 10, 16, 15]
        );
        assert_eq!(d.remaining, 4);
        // 12's right swipe doesn't count: it's in the library already.
        assert_eq!(d.wanted, 1);
    }

    #[test]
    fn card_fields() {
        let d = deck();
        let c = d.cards.iter().find(|c| c.id == 10).unwrap();
        assert_eq!(
            serde_json::to_value(c).unwrap(),
            json!({
                "id": 10, "url": "https://discuss.eroscripts.com/t/10", "title": "Ten from state",
                "author": "a",
                "images": ["https://x/a.jpg", "https://x/b.jpg", "https://x/c.jpg", "https://x/d.jpg", "https://x/e.jpg"],
                "score": 5.0, "vr": true, "axisType": "multi", "axes": 4, "videos": 2, "creampie": true,
                "topTags": [{"tag": "pov", "score": 2.0}, {"tag": "vr", "score": 1.0}, {"tag": "toys", "score": 0.5}],
                "tags": [{"tag": "pov", "score": 2.0}, {"tag": "vr", "score": 1.0}, {"tag": "toys", "score": 0.5},
                         {"tag": "solo", "score": 0.0}, {"tag": "anal", "score": -3.0}],
                "runtimeMs": 754321, "heat": [0, 120, 0, 0, 480],
                "likes": 42, "hosts": ["pixeldrain", "MEGA"], "gb": 1.5,
                "created": "2026-01-02T00:00:00Z", "bumped": "2026-03-04T00:00:00Z",
                "scenes": [
                    {"name": "Ten.mp4", "host": "pixeldrain", "size": 1_500_000_000u64, "runtimeMs": 754321,
                     "axes": ["twist", "roll"]},
                    {"name": "Ten B", "host": null, "size": null, "runtimeMs": null, "axes": ["roll", "surge"]}
                ],
                "moreScenes": 0
            })
        );
        // Older lists: no tagScores / creampie / images; the single image stands in.
        let c = d.cards.iter().find(|c| c.id == 11).unwrap();
        assert_eq!(c.images, ["https://x/only.jpg"]);
        assert_eq!(
            (c.axes, c.videos, c.creampie, c.vr),
            (Axes::Count(1), 1, false, false)
        );
        assert!(c.top_tags.is_empty());
        // No script length: the host's listed length; no heat strip.
        assert_eq!((c.runtime_ms, c.heat.is_empty()), (Some(91400), true));
        assert_eq!(c.author, None);
        // No tags, likes, hosts, sizes or dates: empty, not an error.
        assert!(c.tags.is_empty() && c.hosts.is_empty());
        assert_eq!(
            (c.likes, c.gb, &c.created, &c.bumped),
            (None, None, &None, &None)
        );
        assert_eq!(
            c.scenes,
            [SceneInfo {
                name: None,
                host: None,
                size: None,
                runtime_ms: Some(91400),
                axes: vec![],
            }]
        );
        let c = d.cards.iter().find(|c| c.id == 15).unwrap();
        assert!(c.images.is_empty());
        assert_eq!((c.axes, c.videos, c.score), (Axes::Count(1), 0, None));
        assert_eq!(c.runtime_ms, None);
        assert!(c.scenes.is_empty() && c.more_scenes == 0);
        let c = serde_json::to_value(d.cards.iter().find(|c| c.id == 16).unwrap()).unwrap();
        assert_eq!(c["axes"], "packed");
        assert_eq!(c["images"], json!([]));
    }

    #[test]
    fn scene_list_is_capped() {
        let scenes: Vec<Value> = (0..MAX_SCENES + 3)
            .map(|i| json!({"base": format!("s{i}"), "video": {"size": 1_000_000_000u64}}))
            .collect();
        let c = card(1, &json!({"scenes": scenes}), None, &HashMap::new());
        assert_eq!(c.videos, MAX_SCENES + 3);
        assert_eq!((c.scenes.len(), c.more_scenes), (MAX_SCENES, 3));
        assert_eq!(c.scenes[0].name.as_deref(), Some("s0"));
        // The size counts every scene, listed or not.
        assert_eq!(c.gb, Some((MAX_SCENES + 3) as f64));
    }

    #[test]
    fn leaves_out_uploads_many_posts_share() {
        let sha = |c: char| c.to_string().repeat(40);
        let (banner, logo, preview) = (sha('a'), sha('b'), sha('c'));
        let up = "https://cdn/original/4X/a/b/c";
        // The banner is in 3 posts (once as an optimized copy), the logo in 2.
        let state = json!({"topics": {
            "1": {"images": [format!("{up}/{preview}.gif"), format!("{up}/{banner}.png"), format!("{up}/{logo}.png"),
                             format!("https://cdn/optimized/4X/a/b/c/{preview}_2_690x388.gif")]},
            "2": {"images": [format!("https://cdn/optimized/4X/a/b/c/{banner}_2_690x172.png"), format!("{up}/{logo}.png")]},
            "3": {"image": format!("{up}/{banner}.png")},
            "4": {"images": []}
        }});
        let shared = image_counts(&state);
        let rec = |id: &str| state.pointer(&format!("/topics/{id}")).unwrap();
        // The preview stays (once), the logo too; the banner goes.
        assert_eq!(
            images(rec("1"), &shared),
            [format!("{up}/{preview}.gif"), format!("{up}/{logo}.png")]
        );
        // Records from before images were kept show their one image unless it's shared;
        // an empty list means the scout found nothing worth showing.
        assert!(images(rec("3"), &shared).is_empty());
        assert!(images(rec("4"), &shared).is_empty());
        assert_eq!(
            image_key("https://cdn/x/plain.png"),
            "https://cdn/x/plain.png"
        );
    }

    fn ids(d: &Deck) -> Vec<u64> {
        d.cards.iter().map(|c| c.id).collect()
    }

    /// `n` posts of `stack`'s kind scored 0, 1, … n-1 (id = score), dealt with `deal`.
    fn dealt(n: u64, deal: &Deal) -> Deck {
        let axis = if deal.stack == Stack::Multi {
            "multi"
        } else {
            "single"
        };
        let cands: Vec<Value> = (0..n)
            .map(|i| json!({"id": i, "score": i as f64, "axisType": axis}))
            .collect();
        let mut d = build(
            &json!({}),
            &json!({"candidates": cands}),
            &HashMap::new(),
            &Swipes::default(),
            Pool::Hosts,
        );
        d.deal(deal);
        d
    }

    #[test]
    fn jitter_stays_within_bounds_and_repeats_per_seed() {
        for amp in [MULTI_JITTER, SINGLE_JITTER] {
            let shifts: Vec<f64> = (0..10_000).map(|id| jitter(7, id, amp)).collect();
            assert!(shifts.iter().all(|s| (-amp..=amp).contains(s)));
            // Spread over the whole range, both ways.
            assert!(
                shifts.iter().any(|&s| s < -amp * 0.9) && shifts.iter().any(|&s| s > amp * 0.9)
            );
        }
        assert_eq!(jitter(7, 42, 4.0), jitter(7, 42, 4.0));
        assert_ne!(jitter(7, 42, 4.0), jitter(8, 42, 4.0));
        assert_eq!(Stack::Multi.jitter(), MULTI_JITTER);
        assert_eq!(Stack::Single.jitter(), SINGLE_JITTER);
    }

    #[test]
    fn a_seed_shuffles_a_little_and_pages_agree() {
        for stack in [Stack::Multi, Stack::Single] {
            let amp = stack.jitter();
            let all = Deal {
                limit: MAX_LIMIT,
                stack,
                ..Deal::default()
            };
            // No seed: strictly best first.
            assert_eq!(ids(&dealt(30, &all)), (0..30).rev().collect::<Vec<_>>());
            let seeded = Deal {
                seed: Some(3),
                ..all.clone()
            };
            let order = ids(&dealt(30, &seeded));
            assert_ne!(order, (0..30).rev().collect::<Vec<_>>());
            // Never further than the jitter allows: a card only passes ones within 2 × amp points.
            for (i, a) in order.iter().enumerate() {
                for b in &order[i + 1..] {
                    assert!(
                        *b as f64 - *a as f64 <= 2.0 * amp,
                        "{stack:?}: {a} before {b}"
                    );
                }
            }
            // The second page (the first 10 left out) continues the same order.
            let page2 = Deal {
                limit: 10,
                exclude: order[..10].iter().copied().collect(),
                ..seeded.clone()
            };
            let d = dealt(30, &page2);
            assert_eq!(ids(&d), order[10..20]);
            assert_eq!(d.remaining, 30);
        }
    }

    /// The most score points a card of `order` (ids = scores) got ahead of a better one.
    fn most_passed(order: &[u64]) -> u64 {
        let mut most = 0;
        for (i, a) in order.iter().enumerate() {
            for b in &order[i + 1..] {
                most = most.max(b.saturating_sub(*a));
            }
        }
        most
    }

    #[test]
    fn a_requested_jitter_overrides_the_stacks() {
        for stack in [Stack::Multi, Stack::Single] {
            let seeded = |jitter: f64| Deal {
                limit: MAX_LIMIT,
                seed: Some(3),
                jitter: Some(jitter),
                stack,
                ..Deal::default()
            };
            // 0: strictly by score, seed or not.
            assert_eq!(
                ids(&dealt(60, &seeded(0.0))),
                (0..60).rev().collect::<Vec<_>>()
            );
            // A custom amplitude bounds how far a card moves: within 2 × amp points.
            for amp in [1.5, 25.0] {
                let order = ids(&dealt(60, &seeded(amp)));
                assert_ne!(order, (0..60).rev().collect::<Vec<_>>(), "{stack:?} ±{amp}");
                assert!(most_passed(&order) as f64 <= 2.0 * amp, "{stack:?} ±{amp}");
            }
            // Wider than either stack's own.
            assert!(most_passed(&ids(&dealt(60, &seeded(25.0)))) as f64 > 2.0 * SINGLE_JITTER);
        }
    }

    /// Multi-axis posts 100…(100+m) and single-axis posts 1…s, higher id = higher score.
    fn kinds(m: u64, s: u64, deal: &Deal) -> Deck {
        let multi =
            (0..m).map(|i| json!({"id": 100 + i, "score": 100.0 + i as f64, "axisType": "multi"}));
        let single = (1..=s).map(|i| json!({"id": i, "score": i as f64, "axisType": "single"}));
        let cands: Vec<Value> = multi.chain(single).collect();
        let mut d = build(
            &json!({}),
            &json!({"candidates": cands}),
            &HashMap::new(),
            &Swipes::default(),
            Pool::Hosts,
        );
        d.deal(deal);
        d
    }

    #[test]
    fn one_kind_stacks_count_only_their_own() {
        let multi = kinds(
            3,
            5,
            &Deal {
                stack: Stack::Multi,
                limit: 2,
                ..Deal::default()
            },
        );
        assert_eq!((ids(&multi), multi.remaining), (vec![102, 101], 3));
        let single = kinds(
            3,
            5,
            &Deal {
                stack: Stack::Single,
                ..Deal::default()
            },
        );
        assert_eq!((ids(&single), single.remaining), (vec![5, 4, 3, 2, 1], 5));
        assert_eq!(Stack::parse("multi"), Some(Stack::Multi));
        assert_eq!(Stack::parse("single"), Some(Stack::Single));
        assert_eq!(Stack::parse("Multi"), None);
        assert_eq!(Stack::parse("both"), None);
        assert_eq!(Stack::default(), Stack::Multi);
    }

    #[test]
    fn axes_skip_packed_scenes() {
        let s = |v: Value| v.as_array().unwrap().clone();
        assert_eq!(axes(&[]), Axes::Count(1));
        assert_eq!(
            axes(&s(json!([{"packed": true, "axes": []}, {"packed": true}]))),
            Axes::Packed
        );
        assert_eq!(
            axes(&s(
                json!([{"packed": true, "axes": ["x"]}, {"axes": ["twist"], "packed": null}])
            )),
            Axes::Count(2)
        );
    }

    #[test]
    fn top_tags_boundaries() {
        let tags = |scores: Value| -> Vec<(String, f64)> {
            top_tags(&post_tags(&json!({"tagScores": scores})))
                .into_iter()
                .map(|t| (t.tag, t.score))
                .collect()
        };
        // Top 3 of more, ties by name; zero and negative scores never show.
        assert_eq!(
            tags(json!({"b": 2, "a": 2, "c": 5, "d": 1, "e": 0, "f": -9})),
            [("c".into(), 5.0), ("a".into(), 2.0), ("b".into(), 2.0)]
        );
        assert_eq!(tags(json!({"x": 0.5})), [("x".into(), 0.5)]);
        assert!(tags(json!({"x": -1, "y": 0})).is_empty());
        assert!(tags(json!({"x": "3"})).is_empty());
        assert!(tags(json!(null)).is_empty());
        assert!(top_tags(&post_tags(&json!({}))).is_empty());
    }

    #[test]
    fn post_tags_score_every_tag() {
        let tags = |cand: Value| -> Vec<(String, f64)> {
            post_tags(&cand)
                .into_iter()
                .map(|t| (t.tag, t.score))
                .collect()
        };
        // Unscored tags count 0; a scored tag missing from `tags` still shows; non-numbers are 0.
        assert_eq!(
            tags(
                json!({"tags": ["b", "a", "neg", "s", 3], "tagScores": {"neg": -1, "x": 2, "s": "3"}})
            ),
            [
                ("x".into(), 2.0),
                ("a".into(), 0.0),
                ("b".into(), 0.0),
                ("s".into(), 0.0),
                ("neg".into(), -1.0)
            ]
        );
        assert!(tags(json!({"tags": null, "tagScores": []})).is_empty());
        assert!(tags(Value::Null).is_empty());
    }

    #[test]
    fn waiting_ids() {
        assert!(is_waiting(&waiting(), 10));
        assert!(is_waiting(&waiting(), 15));
        assert!(!is_waiting(&waiting(), 99));
        assert!(!is_waiting(&Value::Null, 10));
    }

    #[test]
    fn load_limits_and_reports_problems() {
        let dir = std::env::temp_dir().join(format!("scout-report-deck-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = load(&dir, &Deal::default());
        assert_eq!((d.remaining, d.wanted, d.cards.len()), (0, 0, 0));
        assert!(d.problems.is_empty());

        std::fs::write(dir.join(report::WAITING_FILE), waiting().to_string()).unwrap();
        std::fs::write(dir.join(report::STATE_FILE), state().to_string()).unwrap();
        let d = load(
            &dir,
            &Deal {
                limit: 2,
                ..Deal::default()
            },
        );
        // The default multi-axis stack: 10 is the only multi-axis post.
        assert_eq!(d.remaining, 1);
        assert_eq!(d.cards.iter().map(|c| c.id).collect::<Vec<_>>(), [10]);

        std::fs::write(dir.join(crate::swipes::EROSCRIPTS.file), b"nope").unwrap();
        let d = load(
            &dir,
            &Deal {
                limit: 0,
                ..Deal::default()
            },
        );
        assert_eq!((d.remaining, d.cards.len()), (1, 0));
        assert!(d.problems[0].starts_with(crate::swipes::EROSCRIPTS.file));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
