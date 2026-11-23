//! The swipe deck: three tabs (Stack), each mixing every source. Its cards are the EroScripts
//! waiting list's posts with no video in the library and no swipe yet, and the site scouts'
//! scenes (crate::scenes: SexLikeReal, VRBangers / VRConk, PMVHaven, FapTap) not downloaded,
//! not in the library, not swiped and not a video a waiting post brings (the post wins: its
//! script is free), and the yes creators' posts no other card shows (crate::creators). A
//! card's tab: multi-axis (a post's `axisType` "multi", a scene's `script.axes`) → Multi,
//! else VR or Flat by its `vr`.
//!
//! Highest score first (the scouts' download order); with a `seed`, each card's score is
//! shifted by up to the deal's jitter (the request's `jitter`, else the tab's MULTI_JITTER,
//! VR_JITTER or FLAT_JITTER) either way, so the order wanders a little. Scores, tag scores and
//! creampie come from the scouts' files (`score`, `tagScores`, `creampie`); Rust only adds
//! the yes-creator bonus (`swipeDeck.yesCreatorBonus`) to a yes creator's cards. Dealt cards
//! name the creators in them (`people`, crate::creators::Creators::add_people), each with her
//! best picture, which also joins the card's images.

use crate::creators::{Creators, Person};
use crate::frames;
use crate::report::{self, id_of, str_of, strings_of};
use crate::scenes::{self, Covered, SceneCard};
use crate::swipes::{self, Dir, Swipes};
use serde::{Serialize, Serializer};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 100;
/// Most cards one deck request may leave out (`exclude`: the cards the page holds).
pub const MAX_EXCLUDE: usize = 1000;
/// Score points a seeded Multi deal moves each card by, at most, either way: multi-axis
/// scripts are a smaller, more valued pool, so the deck stays close to score order.
pub const MULTI_JITTER: f64 = 4.0;
/// Score points a seeded VR deal moves each card by, at most, either way: VR posts and the
/// studio scenes (VRBangers) cost many GB each, so the order stays close to the score.
pub const VR_JITTER: f64 = 3.0;
/// Score points a seeded SLR deal moves each card by, at most, either way: every SLR download
/// costs the account's quota, so the order stays closest to the score.
pub const SLR_JITTER: f64 = 2.0;
/// Score points a seeded Flat deal moves each card by, at most, either way: wide enough
/// (scores run about -4 … 24) that low-scoring posts and videos surface too and the whole
/// stack gets swiped.
pub const FLAT_JITTER: f64 = 10.0;
/// Largest jitter a deck request may ask for (`jitter=N`), in score points.
pub const MAX_JITTER: f64 = 30.0;
/// Images per card (the scouts save at most this many).
pub const MAX_IMAGES: usize = 5;
/// Scenes listed per card; the rest only count (`moreScenes`).
pub const MAX_SCENES: usize = 20;
const TOP_TAGS: usize = 3;
/// The source name of a post card (and of its `exclude` entries).
pub const EROSCRIPTS: &str = "eroscripts";

/// Which cards a deck request deals: anything whose video comes from SexLikeReal (an SLR
/// scene, a post whose video is on SLR) → Slr, since those downloads cost the account's
/// quota; else multi-axis scripts (posts and scenes alike), else VR, else flat. `Free` is the
/// Free space tab: library videos to delete (crate::free_space); `Dupes` the Duplicates tab:
/// pairs of library videos (crate::duplicates). Neither is ever a post or scene, so `build`
/// deals them nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stack {
    #[default]
    Multi,
    Vr,
    Flat,
    Slr,
    Free,
    Dupes,
}

impl Stack {
    pub fn parse(s: &str) -> Option<Stack> {
        match s {
            "multi" => Some(Stack::Multi),
            "vr" => Some(Stack::Vr),
            "flat" => Some(Stack::Flat),
            "slr" => Some(Stack::Slr),
            "free" => Some(Stack::Free),
            "dupes" => Some(Stack::Dupes),
            _ => None,
        }
    }

    /// Most score points a seeded deal of this stack moves a card, either way.
    pub fn jitter(self) -> f64 {
        match self {
            Stack::Multi => MULTI_JITTER,
            Stack::Vr => VR_JITTER,
            Stack::Flat => FLAT_JITTER,
            Stack::Slr => SLR_JITTER,
            Stack::Free | Stack::Dupes => 0.0,
        }
    }

    /// The tab a card goes to: SLR first (quota), then multi-axis, then VR or flat.
    pub fn of(slr: bool, multi: bool, vr: bool) -> Stack {
        if slr {
            Stack::Slr
        } else if multi {
            Stack::Multi
        } else if vr {
            Stack::Vr
        } else {
            Stack::Flat
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
    /// Cards the page holds already (on screen, or skipped for later), as `<source>:<id>`
    /// (DeckCard::key): ids of different sources may be equal.
    pub exclude: HashSet<String>,
    pub stack: Stack,
}

impl Default for Deal {
    fn default() -> Self {
        Deal {
            limit: DEFAULT_LIMIT,
            seed: None,
            jitter: None,
            exclude: HashSet::new(),
            stack: Stack::default(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Deck {
    /// Cards left to swipe in this tab (all of them, not just the ones returned).
    pub remaining: usize,
    /// Right swipes in this tab still to download, every source: waiting posts not in the
    /// library (the next `eroscripts:scout --fetch --auto`) and scenes not fetched or in the
    /// library (each scout's `--fetch`).
    pub wanted: usize,
    /// When the EroScripts scout last scanned the forum (`lastScanAt` in its state file).
    pub last_scan_at: Option<String>,
    /// When each scene scout last wrote its file (`generatedAt`), by source name.
    pub generated_at: BTreeMap<&'static str, Option<String>>,
    /// slr:scout's last quota read: `{checkedAt, scripts, aiScripts, videos}`, each
    /// `{remaining, total, usageExpiresAt}`; null when never read. The page shows it on VR.
    pub quota: Value,
    /// Why the last quota read failed (`no session`, `session expired`, …).
    pub quota_error: Option<String>,
    /// Files that exist but couldn't be read or parsed.
    pub problems: Vec<String>,
    /// The first `limit` cards, in deck order.
    pub cards: Vec<DeckCard>,
}

/// One card; its `source` says where its swipe goes.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum DeckCard {
    /// A waiting EroScripts post (`source: "eroscripts"`): `/api/swipe`.
    Post(Card),
    /// A scout's scene: `/api/scenes/<source>/swipe`.
    Scene(SceneCard),
}

impl DeckCard {
    pub fn source(&self) -> &'static str {
        match self {
            DeckCard::Post(p) => p.source,
            DeckCard::Scene(s) => s.source,
        }
    }

    /// The card's id within its source.
    pub fn id(&self) -> String {
        match self {
            DeckCard::Post(p) => p.id.to_string(),
            DeckCard::Scene(s) => s.id.clone(),
        }
    }

    /// `<source>:<id>`: the card's identity across sources (`exclude` takes these).
    pub fn key(&self) -> String {
        format!("{}:{}", self.source(), self.id())
    }

    pub fn score(&self) -> f64 {
        match self {
            DeckCard::Post(p) => p.score,
            DeckCard::Scene(s) => s.score,
        }
        .unwrap_or(f64::NEG_INFINITY)
    }

    /// What the seeded jitter is drawn from: a post's topic id (as before scenes joined the
    /// tabs), else a hash of the key, so equal ids of two sources move independently.
    fn jitter_id(&self) -> u64 {
        match self {
            DeckCard::Post(p) => p.id,
            DeckCard::Scene(_) => fnv1a(self.key().as_bytes()),
        }
    }

    /// Ties in score: posts first, then SLR, VRBangers, PMVHaven, FapTap; within a source
    /// the newest (highest) id first.
    fn tie(&self, other: &DeckCard) -> Ordering {
        let rank =
            |s: &SceneCard| scenes::Source::parse(s.source).map_or(usize::MAX, |src| src as usize);
        match (self, other) {
            (DeckCard::Post(a), DeckCard::Post(b)) => b.id.cmp(&a.id),
            (DeckCard::Post(_), DeckCard::Scene(_)) => Ordering::Less,
            (DeckCard::Scene(_), DeckCard::Post(_)) => Ordering::Greater,
            (DeckCard::Scene(a), DeckCard::Scene(b)) => rank(a)
                .cmp(&rank(b))
                .then_with(|| (b.id.len(), &b.id).cmp(&(a.id.len(), &a.id))),
        }
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    /// Always `eroscripts`.
    pub source: &'static str,
    pub id: u64,
    pub url: String,
    pub title: Option<String>,
    pub author: Option<String>,
    /// Up to MAX_IMAGES: the state's https `images` (else its `image`), then the post's preview
    /// frames on disk (`/frames/eroscripts/…`, crate::frames).
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
    /// The first scene whose video is on SexLikeReal (`eroscripts:scout --fetch` downloads it
    /// from there with the account's quota).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slr_video: Option<SlrVideo>,
    /// The yes creator (crate::creators) whose post it is: the Creator badge; the score
    /// includes the creator bonus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    /// The creators in the post (crate::creators::Person), at most two, each with her picture.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<Person>,
}

/// A post's video on SexLikeReal (the scout's `scene.video` with `host: "slr"`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlrVideo {
    pub scene_id: Option<u64>,
    /// The scene's page (https only).
    pub url: Option<String>,
}

/// One scene (video) of a post, as the card lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneInfo {
    /// The video's file name, else the scene's base name.
    pub name: Option<String>,
    pub host: Option<String>,
    /// The video on its host (rule34video, pixeldrain, MEGA, SLR, …; https only): the card's
    /// Watch link.
    pub url: Option<String>,
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

/// Everything a deck is built from, parsed (missing files are `Null` / empty).
#[derive(Debug, Default)]
pub struct Files {
    /// eroscripts-scout-state.json: titles, authors, images, dates, lastScanAt.
    pub state: Value,
    /// eroscripts-scout.json: the waiting posts.
    pub waiting: Value,
    /// The library's videos per topic id (crate::library::topic_videos).
    pub videos: HashMap<u64, usize>,
    /// eroscripts-scout-swipes.json.
    pub post_swipes: Swipes,
    /// Each scene scout's file with its swipes.
    pub scenes: Vec<(scenes::Source, Value, Swipes)>,
    /// creators.json, creator-swipes.json and the yes-creator bonus.
    pub creators: Creators,
}

impl Files {
    /// Read every file the deck uses; problems go to `problems`.
    pub fn read(user_data: &Path, problems: &mut Vec<String>) -> Files {
        let state = report::read_json(user_data, report::STATE_FILE, problems);
        let waiting = report::read_json(user_data, report::WAITING_FILE, problems);
        let config = report::read_json(user_data, "config.json", problems);
        let links = report::read_json(user_data, crate::library::LINKS_FILE, problems);
        let post_swipes = report::read_swipes(user_data, swipes::EROSCRIPTS, problems);
        let videos = crate::library::topic_videos(&config, &links, problems);
        let scenes = scenes::Source::ALL
            .into_iter()
            .map(|src| {
                let file = report::read_json(user_data, src.file(), problems);
                let swipes = report::read_swipes(user_data, src.swipes(), problems);
                (src, file, swipes)
            })
            .collect();
        let creators = Creators::read(user_data, problems);
        Files {
            state,
            waiting,
            videos,
            post_swipes,
            scenes,
            creators,
        }
    }

    fn scene_file(&self, source: scenes::Source) -> &Value {
        self.scenes
            .iter()
            .find(|(s, _, _)| *s == source)
            .map_or(&Value::Null, |(_, f, _)| f)
    }
}

/// Read the files and deal the cards `deal` asks for. Dealt posts get their frames (the
/// post's record in frames::EROSCRIPTS_FILE) and SLR scenes theirs (the scene's `frames` in
/// slr-scout.json) after their images, then every dealt card the creators in it (`people`)
/// and their pictures.
pub fn load(user_data: &Path, deal: &Deal) -> Deck {
    let mut problems = Vec::new();
    let files = Files::read(user_data, &mut problems);
    let made = report::read_json(user_data, frames::EROSCRIPTS_FILE, &mut problems);
    let mut deck = build(&files, deal.stack);
    deck.deal(deal);
    let slr = files.scene_file(scenes::Source::Slr);
    for card in &mut deck.cards {
        match card {
            DeckCard::Post(p) => frames::append(
                &mut p.images,
                frames::urls(
                    user_data,
                    frames::Source::Eroscripts,
                    p.id,
                    frames::post_record(&made, p.id),
                ),
            ),
            DeckCard::Scene(s) if s.source == scenes::Source::Slr.name() => {
                if let Ok(id) = s.id.parse::<u64>() {
                    let rec = scenes::scenes(scenes::Source::Slr, slr)
                        .find(|(sid, _)| *sid == s.id)
                        .map_or(&Value::Null, |(_, rec)| rec);
                    frames::append(
                        &mut s.images,
                        frames::urls(user_data, frames::Source::Slr, id, rec),
                    );
                }
            }
            DeckCard::Scene(_) => {}
        }
        files.creators.add_people(user_data, card);
    }
    deck.problems = problems;
    deck
}

impl Deck {
    /// Reorder by `seed` (shifting scores by up to the deal's jitter), leave out `exclude`
    /// (`remaining` still counts them), and keep the first `limit`.
    pub fn deal(&mut self, deal: &Deal) {
        if let Some(seed) = deal.seed {
            let amp = deal.jitter.unwrap_or_else(|| deal.stack.jitter());
            sort(&mut self.cards, |c| {
                c.score() + jitter(seed, c.jitter_id(), amp)
            });
        }
        if !deal.exclude.is_empty() {
            self.cards.retain(|c| !deal.exclude.contains(&c.key()));
        }
        self.cards.truncate(deal.limit);
    }
}

/// Highest `key` first (each card's key computed once); ties by DeckCard::tie.
fn sort(cards: &mut Vec<DeckCard>, key: impl Fn(&DeckCard) -> f64) {
    let mut keyed: Vec<(f64, DeckCard)> = cards.drain(..).map(|c| (key(&c), c)).collect();
    keyed.sort_by(|(ka, a), (kb, b)| kb.total_cmp(ka).then_with(|| a.tie(b)));
    cards.extend(keyed.into_iter().map(|(_, c)| c));
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

/// A waiting post's tab: its video on SexLikeReal (a scene's `video.host` "slr") → Slr,
/// else the scout's `axisType` "multi" (packed archives count as multi) → Multi, else VR or
/// Flat by `vr`.
pub fn post_stack(cand: &Value) -> Stack {
    Stack::of(
        slr_videos(cand).next().is_some(),
        cand.get("axisType").and_then(Value::as_str) == Some("multi"),
        cand.get("vr").and_then(Value::as_bool).unwrap_or(false),
    )
}

/// Every card of `stack`, in deck order (no frames; `problems` empty). Yes creators' cards
/// carry her name and the creator bonus, and her other posts come as creator cards
/// (crate::creators).
pub fn build(f: &Files, stack: Stack) -> Deck {
    let shared = image_counts(&f.state);
    let mut wanted = 0;
    let mut cards = Vec::new();
    for (id, cand) in candidates(&f.waiting) {
        if f.videos.get(&id).is_some_and(|&n| n > 0) || post_stack(cand) != stack {
            continue;
        }
        match f.post_swipes.dir(&id.to_string()) {
            Some(Dir::Right) => wanted += 1,
            Some(Dir::Left) => {}
            None => cards.push(DeckCard::Post(card(
                id,
                cand,
                f.state.pointer(&format!("/topics/{id}")),
                &shared,
            ))),
        }
    }
    let covered = Covered::of(&f.waiting);
    let mut generated_at = BTreeMap::new();
    for (source, file, swipes) in &f.scenes {
        generated_at.insert(source.name(), str_of(file, "generatedAt"));
        for (id, scene) in scenes::scenes(*source, file) {
            if scenes::is_done(scene)
                || covered.covers(*source, &id, scene)
                || scenes::stack_of(*source, scene) != stack
            {
                continue;
            }
            match swipes.dir(&id) {
                Some(Dir::Right) => wanted += 1,
                Some(Dir::Left) => {}
                None => cards.push(DeckCard::Scene(scenes::card(*source, id, scene))),
            }
        }
    }
    for c in &mut cards {
        f.creators.tag(c);
    }
    // Posts another card shows (or showed: swiped, in the library) never come as creator cards.
    let shown: HashSet<String> = candidates(&f.waiting)
        .map(|(id, _)| format!("eroscripts:{id}"))
        .chain(
            scenes::scenes(scenes::Source::Faptap, f.scene_file(scenes::Source::Faptap))
                .map(|(id, _)| format!("faptap:{id}")),
        )
        .collect();
    let (creator_cards, creator_wanted) = f.creators.cards(stack, &shown, &f.videos);
    wanted += creator_wanted;
    cards.extend(creator_cards.into_iter().map(DeckCard::Scene));
    sort(&mut cards, DeckCard::score);
    let slr = f.scene_file(scenes::Source::Slr);
    Deck {
        remaining: cards.len(),
        wanted,
        last_scan_at: str_of(&f.state, "lastScanAt"),
        generated_at,
        quota: slr.get("quota").cloned().unwrap_or(Value::Null),
        quota_error: str_of(slr, "quotaError"),
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
        source: EROSCRIPTS,
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
        creator: None,
        people: Vec::new(),
    }
}

fn scene_info(scene: &Value) -> SceneInfo {
    let video = scene.get("video").unwrap_or(&Value::Null);
    SceneInfo {
        name: str_of(video, "name").or_else(|| str_of(scene, "base")),
        host: str_of(video, "host"),
        url: str_of(video, "url").and_then(|u| watch_url(&u)),
        size: video.get("size").and_then(Value::as_u64),
        runtime_ms: runtime_ms(scene),
        axes: strings_of(scene, "axes").unwrap_or_default(),
    }
}

/// A video link to open in a browser (https only): pixeldrain's file API (`/api/file/<id>…`,
/// which downloads) becomes its viewer page (`/u/<id>`); other hosts' pages as they are.
pub fn watch_url(url: &str) -> Option<String> {
    if !url.starts_with("https://") {
        return None;
    }
    if let Some(rest) = url.strip_prefix("https://pixeldrain.com/api/file/") {
        let id: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if !id.is_empty() {
            return Some(format!("https://pixeldrain.com/u/{id}"));
        }
    }
    Some(url.to_string())
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
    use crate::scenes::Source;
    use crate::swipes::Swipe;
    use serde_json::json;

    fn waiting() -> Value {
        json!({"candidates": [
            {"id": 10, "title": "Ten", "author": "a", "score": 5.0, "vr": true, "axisType": "multi",
             "creampie": true, "likes": 42, "hosts": ["pixeldrain", "MEGA"],
             "tags": ["pov", "vr", "anal", "solo", "", "pov"],
             "tagScores": {"pov": 2, "vr": 1, "anal": -3, "toys": 0.5},
             "scenes": [{"axes": ["twist", "roll"], "durationMs": 754321, "heat": [0, 120.4, -5, "x", 480],
                         "video": {"duration": 9, "name": "Ten.mp4", "host": "pixeldrain", "size": 1_500_000_000u64,
                                   "url": "https://pixeldrain.com/api/file/ten?download"}},
                        {"base": "Ten B", "axes": ["roll", "surge"], "video": {"url": "http://plain/ten-b"}}]},
            {"id": 11, "title": "Eleven", "score": 5.0, "axisType": "single",
             "scenes": [{"axes": [], "video": {"duration": 91.4}}]},
            {"id": 12, "title": "In library", "score": 99.0},
            {"id": 13, "title": "Wanted", "score": 1.0},
            {"id": 14, "title": "Hidden", "score": 50.0},
            {"id": "15", "title": "No score"},
            {"id": 16, "title": "Low", "score": -2.5, "scenes": [{"packed": true, "axes": []}]},
            {"id": "x", "title": "Bad id", "score": 7.0},
            {"id": 17, "title": "On SLR", "score": 80.0, "axisType": "multi", "vr": true, "tagScores": {"pov": 2},
             "scenes": [{"axes": ["twist"], "video": {"name": "A - B (C)_180_LR.mp4", "host": "slr", "slrSceneId": 66249,
                         "slrLabel": "b-66249", "url": "https://www.sexlikereal.com/scenes/b-66249", "size": null, "height": 2880}}]},
            {"id": 18, "title": "SLR and MEGA", "score": 3.0, "axisType": "single",
             "scenes": [{"video": {"name": "m.mp4", "host": "MEGA"}},
                        {"video": {"host": "slr", "slrSceneId": "51207", "url": "http://insecure/x"}}]},
            {"id": 19, "title": "SLR in library", "score": 9.0, "vr": true,
             "scenes": [{"video": {"host": "slr", "slrSceneId": 70000}}]},
            {"id": 20, "title": "VRBangers post", "score": 4.0, "vr": true, "axisType": "single",
             "scenes": [{"axes": [], "video": {"host": "vrbangers", "slug": "full-body-massage",
                         "url": "https://vrbangers.com/video/full-body-massage/"}}]}
        ]})
    }

    fn state() -> Value {
        json!({"lastScanAt": "2026-11-17T00:00:00.000Z", "topics": {
            "10": {"title": "Ten from state", "image": "https://x/a.jpg",
                   "created": "2026-01-02T00:00:00Z", "bumped": "2026-03-04T00:00:00Z",
                   "images": ["https://x/a.jpg", "http://x/insecure.jpg", "https://x/a.jpg", "https://x/b.jpg",
                              "https://x/c.jpg", "https://x/d.jpg", "https://x/e.jpg", "https://x/f.jpg"]},
            "11": {"image": "https://x/only.jpg"},
            "16": {"image": "javascript:alert(1)"}
        }})
    }

    /// slr-scout.json: 66249 and 51207 come with posts 17 and 18; 901 is downloaded, 902
    /// swiped right.
    fn slr() -> Value {
        json!({"generatedAt": "2026-11-16T00:00:00.000Z",
        "quota": {"videos": {"remaining": 30, "total": 35}}, "quotaError": "session expired",
        "scenes": [
            {"id": 66249, "title": "Same as post 17", "score": 90, "vr": true},
            {"id": 51207, "title": "Same as post 18", "score": 90, "vr": true},
            {"id": 900, "title": "SLR scene", "score": 6, "vr": true},
            {"id": 901, "title": "Downloaded", "score": 50, "vr": true, "fetchedAt": "2026-11-16T00:00:00.000Z"},
            {"id": 902, "title": "Wanted", "score": 50, "vr": true}
        ]})
    }

    /// vrb-scout.json: a1 comes with post 20 (same slug), a2 has hand-made axes, a3 is in the
    /// library, a5 hidden.
    fn vrb() -> Value {
        json!({"version": 1, "generatedAt": "2026-11-17T18:00:00.000Z", "scenes": [
            {"id": "a1", "site": "vrbangers", "url": "https://vrbangers.com/video/full-body-massage/", "score": 70, "vr": true},
            {"id": "a2", "site": "vrconk", "url": "https://vrconk.com/video/two/", "score": 7, "vr": true,
             "script": {"url": "https://x/a2.funscript", "axes": ["twist"]}},
            {"id": "a3", "site": "vrbangers", "url": "https://vrbangers.com/video/three/", "score": 60, "vr": true, "inLibrary": true},
            {"id": "a4", "site": "vrbangers", "url": "https://vrbangers.com/video/four/", "score": 2, "vr": true,
             "script": {"axes": []}},
            {"id": "a5", "site": "vrbangers", "url": "https://vrbangers.com/video/five/", "score": 60, "vr": true}
        ]})
    }

    fn pmvhaven() -> Value {
        json!({"version": 1, "scenes": [
            {"id": "p1", "site": "pmvhaven", "score": 3, "vr": false, "pmv": true},
            {"id": "p2", "site": "pmvhaven", "score": 1, "vr": true},
            {"id": "p3", "site": "pmvhaven", "score": 40, "vr": false, "fetchedAt": "2026-11-17T00:00:00.000Z"}
        ]})
    }

    /// faptap-scout.json's candidates; 12 is also a topic id (in the library there).
    fn faptap() -> Value {
        json!({"generatedAt": "2026-11-15T00:00:00.000Z", "candidates": [
            {"id": "f1", "title": "Flat FapTap", "score": 4, "vr": false},
            {"id": "f2", "title": "VR FapTap", "score": 9, "vr": true, "projection": "VR180_SBS"},
            {"id": "f3", "title": "Wanted", "score": 1, "vr": false},
            {"id": 12, "title": "Topic twelve's twin", "score": 0.5}
        ]})
    }

    fn swipes(entries: &[(&str, Dir)]) -> Swipes {
        let mut s = Swipes::default();
        for (id, dir) in entries {
            s.swipes.insert(
                id.to_string(),
                Swipe {
                    dir: *dir,
                    at: "2026-11-13T00:00:00.000Z".into(),
                },
            );
        }
        s
    }

    fn files() -> Files {
        Files {
            state: state(),
            waiting: waiting(),
            videos: HashMap::from([(12, 1), (11, 0), (19, 1)]),
            post_swipes: swipes(&[
                ("13", Dir::Right),
                ("14", Dir::Left),
                ("12", Dir::Right),
                ("18", Dir::Right),
            ]),
            scenes: vec![
                (
                    Source::Slr,
                    slr(),
                    swipes(&[
                        ("902", Dir::Right),
                        ("66249", Dir::Right),
                        ("901", Dir::Right),
                    ]),
                ),
                (Source::Vrb, vrb(), swipes(&[("a5", Dir::Left)])),
                (Source::Pmvhaven, pmvhaven(), swipes(&[("p3", Dir::Right)])),
                (Source::Faptap, faptap(), swipes(&[("f3", Dir::Right)])),
            ],
            creators: Creators::default(),
        }
    }

    fn keys(d: &Deck) -> Vec<String> {
        d.cards.iter().map(DeckCard::key).collect()
    }

    fn post(d: &Deck, id: u64) -> &Card {
        d.cards
            .iter()
            .find_map(|c| match c {
                DeckCard::Post(p) if p.id == id => Some(p),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no post {id}"))
    }

    #[test]
    fn every_source_lands_in_its_tab_best_first() {
        // SLR: whatever downloads from SexLikeReal, whatever its axes or VR: post 17 (multi-axis,
        // its video on SLR) and the SLR scene; post 18 (swiped right) and 902 are its wanted.
        let d = build(&files(), Stack::Slr);
        assert_eq!(keys(&d), ["eroscripts:17", "slr:900"]);
        assert_eq!((d.remaining, d.wanted), (2, 2));
        // Multi: multi-axis posts and a scene with script axes, none of them on SLR.
        let d = build(&files(), Stack::Multi);
        assert_eq!(keys(&d), ["vrb:a2", "eroscripts:10"]);
        assert_eq!((d.remaining, d.wanted), (2, 0));
        // VR: single-axis VR posts and VR scenes of every source but SLR.
        let d = build(&files(), Stack::Vr);
        assert_eq!(
            keys(&d),
            ["faptap:f2", "eroscripts:20", "vrb:a4", "pmvhaven:p2"]
        );
        assert_eq!((d.remaining, d.wanted), (4, 0));
        // Flat: the rest; a topic and a FapTap video may share an id.
        let d = build(&files(), Stack::Flat);
        assert_eq!(
            keys(&d),
            [
                "eroscripts:11",
                "faptap:f1",
                "pmvhaven:p1",
                "faptap:12",
                "eroscripts:16",
                "eroscripts:15"
            ]
        );
        // Post 13 and FapTap f3 are wanted; 12 is in the library, p3 downloaded.
        assert_eq!((d.remaining, d.wanted), (6, 2));
        assert_eq!(d.last_scan_at.as_deref(), Some("2026-11-17T00:00:00.000Z"));
        assert_eq!(
            serde_json::to_value(&d.generated_at).unwrap(),
            json!({"slr": "2026-11-16T00:00:00.000Z", "vrb": "2026-11-17T18:00:00.000Z",
                   "pmvhaven": null, "faptap": "2026-11-15T00:00:00.000Z"})
        );
        // The SLR quota rides along on every tab (the page shows it on SLR).
        assert_eq!(d.quota["videos"]["remaining"], 30);
        assert_eq!(d.quota_error.as_deref(), Some("session expired"));
    }

    #[test]
    fn a_waiting_post_wins_over_its_scene() {
        // SLR 66249 and 51207 come with posts 17 and 18, VRBangers a1 with post 20 (same slug):
        // in no tab, whatever their swipes.
        for stack in [Stack::Multi, Stack::Vr, Stack::Flat, Stack::Slr] {
            let k = keys(&build(&files(), stack));
            for gone in ["slr:66249", "slr:51207", "vrb:a1"] {
                assert!(!k.contains(&gone.to_string()), "{stack:?} {gone}");
            }
        }
        // Without the posts they stand on their own.
        let mut f = files();
        f.waiting = json!({"candidates": []});
        assert_eq!(&keys(&build(&f, Stack::Slr))[..2], ["slr:51207", "slr:900"]);
        assert_eq!(keys(&build(&f, Stack::Vr))[0], "vrb:a1");
        // 66249's right swipe counts once no post brings it.
        assert_eq!(build(&f, Stack::Slr).wanted, 2);
    }

    #[test]
    fn fetched_in_library_and_swiped_scenes_stay_hidden() {
        let all: Vec<String> = [Stack::Multi, Stack::Vr, Stack::Flat, Stack::Slr]
            .into_iter()
            .flat_map(|s| keys(&build(&files(), s)))
            .collect();
        for gone in [
            "slr:901",
            "slr:902",
            "vrb:a3",
            "vrb:a5",
            "pmvhaven:p3",
            "faptap:f3",
        ] {
            assert!(!all.contains(&gone.to_string()), "{gone}");
        }
        // Swipes of one source never hide another's card with the same id.
        let mut f = files();
        f.post_swipes = swipes(&[("12", Dir::Left)]);
        assert!(keys(&build(&f, Stack::Flat)).contains(&"faptap:12".to_string()));
        f.scenes[3].2 = swipes(&[("11", Dir::Left)]);
        assert!(keys(&build(&f, Stack::Flat)).contains(&"eroscripts:11".to_string()));
    }

    #[test]
    fn posts_on_slr_keep_their_video_link() {
        let d = build(&files(), Stack::Slr);
        let c = serde_json::to_value(&d.cards[0]).unwrap();
        assert_eq!(
            (c["source"].as_str(), c["id"].as_u64()),
            (Some("eroscripts"), Some(17))
        );
        assert_eq!(
            c["slrVideo"],
            json!({"sceneId": 66249, "url": "https://www.sexlikereal.com/scenes/b-66249"})
        );
        assert_eq!(c["scenes"][0]["host"], "slr");
        // Only https links.
        let mut f = files();
        f.post_swipes = Swipes::default();
        let d = build(&f, Stack::Slr);
        assert_eq!(
            post(&d, 18).slr_video,
            Some(SlrVideo {
                scene_id: Some(51207),
                url: None
            })
        );
        // A scene card: its id a string, its source where the swipe goes.
        let d = build(&files(), Stack::Slr);
        let c = serde_json::to_value(&d.cards[1]).unwrap();
        assert_eq!(
            (c["source"].as_str(), c["id"].as_str(), c.get("slrVideo")),
            (Some("slr"), Some("900"), None)
        );
    }

    #[test]
    fn posts_go_by_slr_then_axis_type_then_vr() {
        let on_slr = json!([{"video": {"host": "MEGA"}}, {"video": {"host": "slr"}}]);
        for (cand, stack) in [
            (
                json!({"axisType": "multi", "vr": true, "scenes": on_slr}),
                Stack::Slr,
            ),
            (json!({"axisType": "single", "scenes": on_slr}), Stack::Slr),
            (json!({"axisType": "multi", "vr": true}), Stack::Multi),
            (json!({"axisType": "multi"}), Stack::Multi),
            (json!({"axisType": "single", "vr": true}), Stack::Vr),
            (json!({"vr": true}), Stack::Vr),
            (json!({"axisType": "single"}), Stack::Flat),
            (json!({}), Stack::Flat),
        ] {
            assert_eq!(post_stack(&cand), stack, "{cand}");
        }
    }

    #[test]
    fn card_fields() {
        let d = build(&files(), Stack::Multi);
        assert_eq!(
            serde_json::to_value(post(&d, 10)).unwrap(),
            json!({
                "source": "eroscripts",
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
                    {"name": "Ten.mp4", "host": "pixeldrain", "url": "https://pixeldrain.com/u/ten",
                     "size": 1_500_000_000u64, "runtimeMs": 754321, "axes": ["twist", "roll"]},
                    // Only https links.
                    {"name": "Ten B", "host": null, "url": null, "size": null, "runtimeMs": null, "axes": ["roll", "surge"]}
                ],
                "moreScenes": 0
            })
        );
        let d = build(&files(), Stack::Flat);
        // Older lists: no tagScores / creampie / images; the single image stands in.
        let c = post(&d, 11);
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
                url: None,
                size: None,
                runtime_ms: Some(91400),
                axes: vec![],
            }]
        );
        let c = post(&d, 15);
        assert!(c.images.is_empty());
        assert_eq!((c.axes, c.videos, c.score), (Axes::Count(1), 0, None));
        assert_eq!(c.runtime_ms, None);
        assert!(c.scenes.is_empty() && c.more_scenes == 0);
        let c = serde_json::to_value(post(&d, 16)).unwrap();
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
        d.cards.iter().map(|c| c.id().parse().unwrap()).collect()
    }

    /// `n` posts of `stack`'s kind scored 0, 1, … n-1 (id = score), dealt with `deal`.
    fn dealt(n: u64, deal: &Deal) -> Deck {
        let cands: Vec<Value> = (0..n)
            .map(|i| {
                let mut c = json!({"id": i, "score": i as f64});
                match deal.stack {
                    Stack::Multi => c["axisType"] = json!("multi"),
                    Stack::Vr => c["vr"] = json!(true),
                    Stack::Slr => c["scenes"] = json!([{"video": {"host": "slr"}}]),
                    Stack::Flat | Stack::Free | Stack::Dupes => {}
                }
                c
            })
            .collect();
        let f = Files {
            waiting: json!({"candidates": cands}),
            ..Files::default()
        };
        let mut d = build(&f, deal.stack);
        d.deal(deal);
        d
    }

    const STACKS: [Stack; 4] = [Stack::Multi, Stack::Vr, Stack::Flat, Stack::Slr];

    #[test]
    fn jitter_stays_within_bounds_and_repeats_per_seed() {
        for amp in [MULTI_JITTER, VR_JITTER, FLAT_JITTER, SLR_JITTER] {
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
        assert_eq!(Stack::Vr.jitter(), VR_JITTER);
        assert_eq!(Stack::Flat.jitter(), FLAT_JITTER);
        assert_eq!(Stack::Slr.jitter(), SLR_JITTER);
    }

    #[test]
    fn a_seed_shuffles_a_little_and_pages_agree() {
        for stack in STACKS {
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
            assert!(most_passed(&order) as f64 <= 2.0 * amp, "{stack:?}");
            // The second page (the first 10 left out) continues the same order.
            let page2 = Deal {
                limit: 10,
                exclude: order[..10]
                    .iter()
                    .map(|id| format!("eroscripts:{id}"))
                    .collect(),
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
        for stack in STACKS {
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
            // Wider than any stack's own.
            assert!(most_passed(&ids(&dealt(60, &seeded(25.0)))) as f64 > 2.0 * FLAT_JITTER);
        }
    }

    #[test]
    fn excludes_by_source_and_id_and_mixes_sources_when_seeded() {
        // eroscripts:11 isn't faptap:11.
        let mut f = files();
        f.scenes[3].1 = json!({"candidates": [{"id": 11, "score": 5.0}]});
        let mut d = build(&f, Stack::Flat);
        d.deal(&Deal {
            exclude: HashSet::from(["eroscripts:11".to_string(), "pmvhaven:p1".to_string()]),
            stack: Stack::Flat,
            ..Deal::default()
        });
        assert_eq!(keys(&d), ["faptap:11", "eroscripts:16", "eroscripts:15"]);
        assert_eq!(d.remaining, 5, "remaining counts the whole tab");
        // Equal scores: the post first, then the sources in order, newest id first.
        assert_eq!(
            keys(&build(&f, Stack::Flat))[..3],
            ["eroscripts:11", "faptap:11", "pmvhaven:p1"]
        );
        let mut tie = build(&files(), Stack::Vr);
        sort(&mut tie.cards, |_| 1.0);
        assert_eq!(
            keys(&tie),
            ["eroscripts:20", "vrb:a4", "pmvhaven:p2", "faptap:f2"]
        );
        // The default VR jitter: a card only passes ones within 2 × VR_JITTER points, and
        // some seed reorders the sources.
        let base = build(&files(), Stack::Vr);
        let scores: HashMap<String, f64> =
            base.cards.iter().map(|c| (c.key(), c.score())).collect();
        let mut moved = false;
        for seed in 0..50 {
            let mut d = build(&files(), Stack::Vr);
            d.deal(&Deal {
                seed: Some(seed),
                stack: Stack::Vr,
                ..Deal::default()
            });
            moved |= keys(&d) != keys(&base);
            for (i, a) in d.cards.iter().enumerate() {
                for b in &d.cards[i + 1..] {
                    assert!(
                        scores[&b.key()] - scores[&a.key()] <= 2.0 * VR_JITTER,
                        "seed {seed}"
                    );
                }
            }
        }
        assert!(moved);
    }

    #[test]
    fn stacks_parse() {
        assert_eq!(Stack::parse("multi"), Some(Stack::Multi));
        assert_eq!(Stack::parse("vr"), Some(Stack::Vr));
        assert_eq!(Stack::parse("flat"), Some(Stack::Flat));
        assert_eq!(Stack::parse("slr"), Some(Stack::Slr));
        for bad in ["single", "SLR", "Multi", "VR", "both", ""] {
            assert_eq!(Stack::parse(bad), None, "{bad}");
        }
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
        assert!(d.quota.is_null());

        std::fs::write(dir.join(report::WAITING_FILE), waiting().to_string()).unwrap();
        std::fs::write(dir.join(report::STATE_FILE), state().to_string()).unwrap();
        std::fs::write(dir.join(Source::Vrb.file()), vrb().to_string()).unwrap();
        let d = load(
            &dir,
            &Deal {
                limit: 2,
                ..Deal::default()
            },
        );
        // The default Multi tab: post 10 and scene a2 (post 17 is on SLR), both dealt.
        assert_eq!(d.remaining, 2);
        assert_eq!(keys(&d), ["vrb:a2", "eroscripts:10"]);

        // Broken files are problems; the rest still deals.
        std::fs::write(dir.join(swipes::EROSCRIPTS.file), b"nope").unwrap();
        std::fs::write(dir.join(Source::Pmvhaven.file()), b"{oops").unwrap();
        std::fs::write(dir.join(swipes::FAPTAP.file), b"[]").unwrap();
        let d = load(
            &dir,
            &Deal {
                limit: 0,
                ..Deal::default()
            },
        );
        assert_eq!((d.remaining, d.cards.len()), (2, 0));
        for file in [
            swipes::EROSCRIPTS.file,
            Source::Pmvhaven.file(),
            swipes::FAPTAP.file,
        ] {
            assert!(
                d.problems.iter().any(|p| p.starts_with(file)),
                "{file}: {:?}",
                d.problems
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn with_creators() -> Files {
        let mut f = files();
        f.creators = Creators::new(
            json!({"creators": {
                "ann": {"name": "Ann", "sources": ["favourite"],
                    "pictures": [{"file": "es-9.jpg", "source": "eroscripts"}, {"file": "ig-1.jpg", "source": "instagram"}],
                    "posts": [
                        {"kind": "eroscripts", "id": 11, "downloadable": true},
                        {"kind": "faptap", "id": "f1", "downloadable": true},
                        {"kind": "eroscripts", "id": "500", "title": "Ann's own", "score": 6.0, "downloadable": true,
                         "image": "es-500.jpg", "url": "https://discuss.eroscripts.com/t/500", "tags": ["pov"], "durationS": 90},
                        {"kind": "eroscripts", "id": 501, "score": 2.0, "downloadable": false},
                        {"kind": "faptap", "id": "700", "score": 1.0, "downloadable": true, "inLibrary": true},
                        {"kind": "eroscripts", "id": 19, "score": 1.0, "downloadable": true},
                        {"kind": "eroscripts", "id": 502, "score": 3.0, "downloadable": true},
                        {"kind": "eroscripts", "id": 503, "score": 3.0, "downloadable": true, "multiAxis": true}]},
                "bea": {"name": "Bea", "posts": [{"kind": "eroscripts", "id": 600, "score": 50.0, "downloadable": true}]},
                "cat": {"name": "Cat", "sources": ["pornstar"],
                    "posts": [{"kind": "eroscripts", "id": 601, "score": 1.0, "downloadable": true}]}
            }}),
            json!({"version": 1, "creators": {"cat": {"choice": "no", "at": "t"}},
                "posts": {"eroscripts:502": {"choice": "want", "at": "t", "creator": "ann"}}}),
            4.0,
        );
        f
    }

    #[test]
    fn yes_creators_cards_get_her_badge_the_bonus_and_her_other_posts() {
        let plain = build(&files(), Stack::Flat);
        let d = build(&with_creators(), Stack::Flat);
        // Her waiting post and FapTap scene: tagged, 4 points up.
        let p11 = post(&d, 11);
        assert_eq!(
            (p11.creator.as_deref(), p11.score),
            (Some("Ann"), Some(9.0))
        );
        let f1 = d
            .cards
            .iter()
            .find_map(|c| match c {
                DeckCard::Scene(s) if s.id == "f1" => Some(s),
                _ => None,
            })
            .unwrap();
        assert_eq!((f1.creator.as_deref(), f1.score), (Some("Ann"), Some(8.0)));
        assert_eq!(f1.score_parts["yes creator"], 4.0);
        // Her post no other card shows comes as a creator card; not the undownloadable one,
        // the one in the library (topic 19 has a video), the swiped one, nor anything of Bea
        // (not yes) or Cat (swiped no).
        let mine: Vec<_> = d
            .cards
            .iter()
            .filter(|c| c.source() == crate::creators::SOURCE)
            .map(DeckCard::key)
            .collect();
        assert_eq!(mine, ["creators:eroscripts-500"]);
        let DeckCard::Scene(own) = d.cards.iter().find(|c| c.source() == "creators").unwrap()
        else {
            unreachable!()
        };
        assert_eq!(own.score, Some(10.0));
        assert_eq!(own.site, "eroscripts");
        assert_eq!(own.runtime_ms, Some(90_000));
        assert_eq!(
            own.images,
            [
                "pictures/ann/es-500.jpg",
                "pictures/ann/ig-1.jpg",
                "pictures/ann/es-9.jpg"
            ]
        );
        assert_eq!(d.remaining, plain.remaining + 1);
        // The swiped want counts until it's in the library.
        assert_eq!(d.wanted, plain.wanted + 1);
        // Higher score, earlier card: the creator card now outranks post 11.
        let at = |k: &str| keys(&d).iter().position(|x| x == k).unwrap();
        assert!(at("creators:eroscripts-500") < at("eroscripts:11"));
        // A multi-axis post goes to Multi.
        let multi = build(&with_creators(), Stack::Multi);
        assert!(keys(&multi).contains(&"creators:eroscripts-503".to_string()));
        let v = serde_json::to_value(own).unwrap();
        assert_eq!(
            (&v["source"], &v["creator"]),
            (&json!("creators"), &json!("Ann"))
        );
    }
}
