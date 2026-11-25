//! The swipe deck's Duplicates tab (`stack=dupes`): two library videos side by side that may be
//! the same video, the likeliest pair first, each with a duplicate confidence (0–100). Videos
//! come from the running app's backend (crate::free_space::Library), the evidence from what the
//! other tools already measured: title words (the shared name rules, content-preferences.json
//! `names`), lengths and script speeds (the backend), frame hashes (`scene-frame-cache.json`,
//! written by `npm run dedupe:scenes`), the main scripts (the backend, only for pairs whose
//! lengths and speeds agree) and the post or scene each came from (sidecars,
//! `eroscripts-links.json`).
//!
//! Which pairs are dealt (`swipeDeck.duplicates` in content-preferences.json): every pair at
//! `minConfidence` or more; when that's fewer than `target`, the best `target` down to
//! `floorConfidence` (crate::duplicates::bar). PMVs and compilations that share a title with
//! another video are capped low (`compilationWords`): rarely the same video, shown last for
//! confirmation.
//!
//! A swipe only marks, in `duplicate-swipes.json` by pair (`<id>-<id>`, the smaller backend id
//! first, the left card): left = keep the left video and delete the right one, right = the
//! other way round, up = not duplicates (never dealt again). A video marked for deletion leaves
//! every other pair. `POST /api/duplicates/delete` then asks the backend to delete each marked
//! copy as a duplicate of the kept one (`/api/remote/delete-duplicate`); the desktop app deletes
//! it the way triage does, with the rejection reason `duplicate`.

use crate::free_space::{self, Library, id_text};
use crate::report;
use crate::scenes::{self, Source};
use age_screen::backend::Backend;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// The cards' `source`, and the `exclude` prefix (`dupes:<id>-<id>`).
pub const SOURCE: &str = "dupes";
pub const SWIPES_FILE: &str = "duplicate-swipes.json";
pub const SWIPES_BACKUP: &str = "duplicate-swipes.json.bak";
const VERSION: u32 = 1;

pub const FRAME_CACHE_FILE: &str = "scene-frame-cache.json";
pub const PROBE_CACHE_FILE: &str = "scene-probe-cache.json";
pub const DIMS_CACHE_FILE: &str = "vr-dims-cache.json";
pub const LINKS_FILE: &str = "eroscripts-links.json";

/// Lengths this close are the same video's (scripts/scene-dedupe-lib.mjs DURATION_TOLERANCE_S).
const SAME_LENGTH_S: f64 = 1.5;
/// A trimmed or padded encode: up to max(this, TRIM_SHARE × the longer).
const TRIM_MIN_S: f64 = 3.0;
const TRIM_SHARE: f64 = 0.01;
/// Frame hashes (scene-frame-cache.json, dHash of 144 bits): at most this many differing bits
/// show the same picture, and a frame needs this much grey-level spread to be a picture at
/// all (scene-dedupe-lib.mjs SAME_HASH_MAX, FRAME_DETAIL_MIN: the cache's own meaning).
const SAME_HASH_MAX: u32 = 20;
const FRAME_DETAIL_MIN: f64 = 12.0;
/// The burst's frame at the sample time itself (BURST_OFFSETS_S = [-0.5, 0, 0.5]).
const BURST_CENTRE: usize = 1;
/// A word in more titles than this pairs nothing by itself (a franchise, a character).
const MAX_WORD_VIDEOS: usize = 150;
/// Pairs whose titles share less than this similarity aren't looked at.
const MIN_TITLE: f64 = 0.34;
/// Highest confidence a PMV / compilation pair gets when the lengths differ.
const COMPILATION_CAP: f64 = 45.0;
/// Highest confidence of a VR and a flat video (one scene's two versions, never one file).
const VERSIONS_CAP: f64 = 85.0;
/// Highest confidence of titles one word apart each whose frames don't match everywhere.
const SWAP_CAP: f64 = 60.0;
/// Titles one word apart: a sample matches only when the fine grids (16×16 grey cells) differ
/// by this much at most anywhere (scene-dedupe-lib.mjs SWAP_CELL_MAX).
const SWAP_CELL_MAX: f64 = 12.0;

const SHARED: &str = include_str!("../../../shared/content-preferences.json");

// ── Shared preferences ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thresholds {
    /// Pairs at this confidence or more are always dealt.
    pub min_confidence: f64,
    /// Never deal a pair under this.
    pub floor_confidence: f64,
    /// Fewer than this at `min_confidence`: deal the best `target` down to the floor.
    pub target: usize,
    /// A title (or a tag) with one of these words is a PMV or compilation.
    pub compilation_words: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Names {
    noise_words: Vec<String>,
    noise_token: String,
    projection_word: String,
    genre_words: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwipeDeck {
    duplicates: Thresholds,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Shared {
    swipe_deck: SwipeDeck,
    names: Names,
}

static SHARED_PREFS: LazyLock<Shared> = LazyLock::new(|| {
    serde_json::from_str(SHARED)
        .expect("shared/content-preferences.json: swipeDeck.duplicates, names")
});

/// The embedded `swipeDeck.duplicates` thresholds.
pub fn thresholds() -> Thresholds {
    SHARED_PREFS.swipe_deck.duplicates.clone()
}

struct Rules {
    noise: HashSet<String>,
    token: Regex,
    projection: Regex,
    genre: HashSet<String>,
}

static RULES: LazyLock<Rules> = LazyLock::new(|| {
    let n = &SHARED_PREFS.names;
    Rules {
        noise: n.noise_words.iter().cloned().collect(),
        token: Regex::new(&n.noise_token).expect("names.noiseToken"),
        projection: Regex::new(&format!("^(?:{})$", n.projection_word))
            .expect("names.projectionWord"),
        genre: n.genre_words.iter().cloned().collect(),
    }
});

// ── Title words ──────────────────────────────────────────────────────────

/// Names as compared everywhere (scene-dedupe-lib.mjs fold): compatibility forms folded, lower
/// case, accents and other combining marks dropped, so a volume's decomposed name equals a
/// forum's composed one.
pub fn fold(s: &str) -> String {
    s.nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .nfkc()
        .collect()
}

/// A word that describes the encode, cut, variant or projection (names.noiseWords, noiseToken,
/// projectionWord).
pub fn is_noise(w: &str) -> bool {
    let r = &*RULES;
    r.noise.contains(w) || r.token.is_match(w) || r.projection.is_match(w)
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
        | 0xFF66..=0xFF9F | 0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF
        | 0x20000..=0x2A6DF)
}

/// A word that can tell titles apart: not a genre word, 3+ letters or CJK.
pub fn is_distinctive(w: &str) -> bool {
    !RULES.genre.contains(w) && (w.chars().count() >= 3 || w.chars().any(is_cjk))
}

/// The words of a file name (no extension) that say what it shows, and its trailing copy
/// number (`Title (2)`), as scene-dedupe-lib.mjs titleWords: the artist's name in front
/// dropped, `fap hero` / `cock hero` one word, noise words dropped. Sorted, each once.
pub fn title_words(stem: &str, artist: &str) -> (Vec<String>, Option<u32>) {
    let mut s = fold(stem);
    let a = fold(artist);
    let a = a.trim();
    if !a.is_empty() && s.starts_with(a) {
        s = s[a.len()..].to_string();
    }
    let copy = trailing_copy(&s);
    if let Some((at, _)) = copy {
        s.truncate(at);
    }
    let tokens: Vec<&str> = s
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let mut words = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let w = tokens[i];
        if (w == "fap" || w == "cock") && tokens.get(i + 1) == Some(&"hero") {
            words.push(format!("{w}hero"));
            i += 2;
        } else {
            words.push(w.to_string());
            i += 1;
        }
    }
    words.retain(|w| !is_noise(w));
    words.sort();
    words.dedup();
    (words, copy.map(|(_, n)| n))
}

/// `… (12)` at the end: where the copy number starts (spaces before it included) and the number.
fn trailing_copy(s: &str) -> Option<(usize, u32)> {
    let t = s.trim_end();
    let inner = t.strip_suffix(')')?;
    let open = inner.rfind('(')?;
    let digits = &inner[open + 1..];
    if digits.is_empty() || digits.len() > 3 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((t[..open].trim_end().len(), digits.parse().ok()?))
}

// ── Evidence and confidence ──────────────────────────────────────────────

/// What two videos have in common.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Evidence {
    /// Title similarity 0–1 (0 without a shared distinctive word).
    pub title: f64,
    /// The distinctive words both titles have.
    pub shared: Vec<String>,
    /// Both lengths known: how far apart, and the longer one, in seconds.
    pub lengths: Option<(f64, f64)>,
    /// The main scripts' actions are identical.
    pub same_script: bool,
    /// Frame samples at the same times: (showing the same picture, compared).
    pub frames: Option<(usize, usize)>,
    pub same_artist: bool,
    /// Both came from one forum post or one site scene.
    pub same_post: bool,
    /// One is VR, the other flat.
    pub vr_differs: bool,
    /// One is a PMV / compilation (compilationWords).
    pub compilation: bool,
    /// Numbered differently: `Part 1` / `Part 2`, `(1)` / `(2)`, or `X` / `X 2` of other
    /// lengths (a sequel).
    pub series: bool,
    /// The titles differ by one word each (`Tess` / `Haley`: one animation, another
    /// character); `frames` then count only samples that match everywhere in the picture.
    pub swap: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Maybe the same video twice.
    Duplicate,
    /// A VR and a flat video of one scene.
    Versions,
    /// A PMV or compilation sharing a title with another video: rarely a duplicate.
    Compilation,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    /// 0–100, whole numbers.
    pub confidence: f64,
    pub kind: Kind,
    /// Why, most telling first.
    pub reasons: Vec<String>,
}

fn secs(s: f64) -> String {
    if s < 10.0 {
        format!("{s:.1} s")
    } else if s < 120.0 {
        format!("{s:.0} s")
    } else {
        format!("{:.0} min", s / 60.0)
    }
}

/// How sure two videos are one: title and length make the base (55 + 35 points, artist and post
/// 5 each); an identical script or matching frames lift it to 85–100, frames that differ cut
/// it; different part numbers cut it unless script or frames match; a PMV / compilation of
/// another length is capped at COMPILATION_CAP, VR next to flat at VERSIONS_CAP.
pub fn judge(e: &Evidence) -> Verdict {
    let mut reasons = Vec::new();
    let length = match e.lengths {
        None => 0.3,
        Some((d, _)) if d <= SAME_LENGTH_S => 1.0,
        Some((d, longer)) if d <= TRIM_MIN_S.max(TRIM_SHARE * longer) => 0.85,
        Some((d, longer)) if d <= 0.05 * longer => 0.5,
        Some((d, longer)) if d <= 0.15 * longer => 0.2,
        Some(_) => 0.0,
    };
    let mut s = 55.0 * e.title
        + 35.0 * length
        + if e.same_artist { 5.0 } else { 0.0 }
        + if e.same_post { 5.0 } else { 0.0 };
    let frames_match = e
        .frames
        .is_some_and(|(same, n)| n >= 2 && same * 5 >= n * 4);
    if e.same_script {
        s = s.max(85.0 + 10.0 * e.title);
        reasons.push("identical script".to_string());
    }
    if let Some((same, n)) = e.frames.filter(|&(_, n)| n >= 2) {
        let f = same as f64 / n as f64;
        if frames_match {
            s = s.max(85.0 + 15.0 * f);
        } else if f <= 0.3 && n >= 3 {
            s *= 0.6;
        } else {
            s *= 0.8 + 0.2 * f;
        }
        reasons.push(format!("frames match {same}/{n}"));
    }
    if e.title >= 0.99 {
        reasons.push("same title".to_string());
    } else if !e.shared.is_empty() {
        reasons.push(format!("titles share “{}”", e.shared.join(" ")));
    }
    match e.lengths {
        Some((d, _)) if d <= SAME_LENGTH_S => reasons.push(format!("same length (±{d:.1} s)")),
        Some((d, _)) => reasons.push(format!("lengths {} apart", secs(d))),
        None => {}
    }
    if e.same_post {
        reasons.push("same post".to_string());
    }
    if e.same_artist {
        reasons.push("same artist".to_string());
    }
    if e.series && !e.same_script && !frames_match {
        s *= 0.6;
        reasons.push("numbered differently".to_string());
    }
    if e.swap && !frames_match {
        s = s.min(SWAP_CAP);
        reasons.push("titles one word apart (another character?)".to_string());
    }
    let kind = if e.compilation && length < 0.5 && !e.same_script && !frames_match {
        s = s.min(COMPILATION_CAP);
        Kind::Compilation
    } else if e.vr_differs {
        s = s.min(VERSIONS_CAP);
        reasons.push("one VR, one flat".to_string());
        Kind::Versions
    } else {
        Kind::Duplicate
    };
    Verdict {
        confidence: s.clamp(0.0, 100.0).round(),
        kind,
        reasons,
    }
}

/// Title similarity of two word sets (no copy numbers): half containment (shared / the shorter),
/// half Jaccard (shared / all); 0 without a shared distinctive word. Also the shared distinctive
/// words.
pub fn title_similarity(a: &[String], b: &[String]) -> (f64, Vec<String>) {
    let bs: HashSet<&String> = b.iter().collect();
    let shared: Vec<&String> = a.iter().filter(|w| bs.contains(w)).collect();
    let telling: Vec<String> = shared
        .iter()
        .filter(|w| is_distinctive(w))
        .map(|w| w.to_string())
        .collect();
    if telling.is_empty() {
        return (0.0, telling);
    }
    let n = shared.len() as f64;
    let contain = n / a.len().min(b.len()) as f64;
    let jaccard = n / (a.len() + b.len() - shared.len()) as f64;
    (0.5 * contain + 0.5 * jaccard, telling)
}

/// Different part numbers: each title has a number the other lacks, one has a number the other
/// lacks and the lengths differ (`same_length`: within the trim window), or both have copy
/// numbers and they differ.
fn numbered_apart(
    a: &[String],
    b: &[String],
    ca: Option<u32>,
    cb: Option<u32>,
    same_length: bool,
) -> bool {
    let num = |w: &&String| w.bytes().all(|c| c.is_ascii_digit());
    let only_a = a.iter().filter(num).any(|w| !b.contains(w));
    let only_b = b.iter().filter(num).any(|w| !a.contains(w));
    (only_a && only_b)
        || ((only_a || only_b) && !same_length)
        || matches!((ca, cb), (Some(x), Some(y)) if x != y)
}

/// Titles one word apart each (scene-dedupe-lib.mjs characterSwap).
fn character_swap(a: &[String], b: &[String]) -> bool {
    let only_a = a.iter().filter(|w| !b.contains(w)).count();
    let only_b = b.iter().filter(|w| !a.contains(w)).count();
    only_a == 1 && only_b == 1 && a.len() > 1
}

// ── What the disk knows ──────────────────────────────────────────────────

/// The post or scene a video came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Post {
    /// `eroscripts`, `faptap`, `vrb`, `pmvhaven` or `slr`.
    pub kind: &'static str,
    pub id: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Quality {
    pub width: u32,
    pub height: u32,
    pub fps: Option<f64>,
}

/// A frame-cache burst: frames as `hash:detail[:cells]`, null where none was read.
pub type Burst = Vec<Option<String>>;

/// Everything outside the backend the tab reads; tests fake it.
pub trait Disk {
    /// The file's size; None = not on disk.
    fn size(&self, path: &str) -> Option<u64>;
    /// Its post or scene: the sidecar in its folder, else eroscripts-links.json.
    fn post(&self, path: &str) -> Option<Post>;
    /// scene-frame-cache.json's bursts by sample key, when cached for a file of `size` bytes.
    fn bursts(&self, path: &str, size: u64) -> Option<&HashMap<String, Burst>>;
    /// The main script's actions (at, pos).
    fn actions(&self, id: &str) -> Option<Vec<(i64, i64)>>;
    fn quality(&self, path: &str) -> Option<Quality>;
    /// When the post was made or the scene released, and which of the two (`posted`,
    /// `released`).
    fn posted(&self, post: &Post) -> Option<(String, &'static str)>;
}

// ── Pairs ────────────────────────────────────────────────────────────────

/// A library video as compared.
#[derive(Debug, Clone)]
struct Facts<'a> {
    v: &'a Value,
    id: String,
    path: &'a str,
    source: &'a Path,
    size: u64,
    words: Vec<String>,
    copy: Option<u32>,
    artist: Option<String>,
    duration: f64,
    vr: bool,
    speeds: (f64, f64),
    compilation: bool,
}

/// Two videos that may be one, `a`'s id the smaller.
#[derive(Debug, Clone, PartialEq)]
pub struct Pair {
    pub id: String,
    pub a: String,
    pub b: String,
    pub verdict: Verdict,
}

/// The pair's key: both ids, the smaller first.
pub fn pair_id(a: &str, b: &str) -> String {
    if a <= b {
        format!("{a}-{b}")
    } else {
        format!("{b}-{a}")
    }
}

/// A pair key's two ids (letters and digits each, the smaller first).
pub fn parse_pair(s: &str) -> Option<(&str, &str)> {
    let (a, b) = s.split_once('-')?;
    let ok = |x: &str| {
        (1..=crate::swipes::MAX_ID_LEN).contains(&x.len())
            && x.bytes().all(|c| c.is_ascii_alphanumeric())
    };
    (ok(a) && ok(b) && a < b).then_some((a, b))
}

fn artist_folder(path: &Path, source: &Path) -> Option<String> {
    let rest = path.strip_prefix(source).ok()?;
    let mut parts = rest.components();
    let first = parts.next()?;
    parts.next()?;
    Some(first.as_os_str().to_string_lossy().into_owned())
}

fn facts<'a>(
    lib: &'a Library,
    sources: &'a [PathBuf],
    disk: &dyn Disk,
    words_of: &[String],
) -> Vec<Facts<'a>> {
    let mut out = Vec::new();
    for v in &lib.videos {
        let (Some(id), Some(path)) = (
            v.get("id").and_then(id_text),
            v.get("path").and_then(Value::as_str),
        ) else {
            continue;
        };
        if !swipe_id_ok(&id) {
            continue;
        }
        let p = Path::new(path);
        let Some(source) = sources.iter().find(|s| p.starts_with(s)) else {
            continue;
        };
        let Some(size) = disk.size(path) else {
            continue;
        };
        let folder = artist_folder(p, source);
        let artist = lib.artists.get(&id).cloned().or(folder.clone());
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (mut words, copy) = title_words(&stem, folder.as_deref().unwrap_or(""));
        // The artist's own words say who, not what: same_artist counts that.
        let artist_words: HashSet<String> = [artist.as_deref(), folder.as_deref()]
            .into_iter()
            .flatten()
            .flat_map(|a| title_words(a, "").0)
            .collect();
        if words.iter().any(|w| !artist_words.contains(w)) {
            words.retain(|w| !artist_words.contains(w));
        }
        let raw = fold(&stem);
        let tags: Vec<String> = v
            .get("tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|t| Some(fold(t.as_str()?)))
            .collect();
        let compilation = raw
            .split(|c: char| !c.is_alphanumeric())
            .chain(tags.iter().map(String::as_str))
            .any(|w| words_of.iter().any(|c| c == w))
            || words.iter().any(|w| words_of.contains(w));
        let num = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        out.push(Facts {
            v,
            id,
            path,
            source,
            size,
            words,
            copy,
            artist: artist.map(|a| fold(&a)),
            duration: num("duration"),
            vr: v.get("isVR").and_then(Value::as_bool).unwrap_or(false),
            speeds: (num("avgSpeed"), num("maxSpeed")),
            compilation,
        });
    }
    out
}

fn swipe_id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= crate::swipes::MAX_ID_LEN
        && id.bytes().all(|c| c.is_ascii_alphanumeric())
}

/// Index pairs worth judging: titles sharing a distinctive word (not one in more than
/// MAX_WORD_VIDEOS titles), and videos as long (±SAME_LENGTH_S) with the same script speeds
/// (a re-upload under another name).
fn candidates(f: &[Facts]) -> Vec<(usize, usize)> {
    let mut seen = HashSet::new();
    let mut index: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, x) in f.iter().enumerate() {
        for w in x.words.iter().filter(|w| is_distinctive(w)) {
            index.entry(w.as_str()).or_default().push(i);
        }
    }
    for list in index.values().filter(|l| l.len() <= MAX_WORD_VIDEOS) {
        for (k, &i) in list.iter().enumerate() {
            for &j in &list[k + 1..] {
                seen.insert((i.min(j), i.max(j)));
            }
        }
    }
    let mut by_length: Vec<usize> = (0..f.len()).filter(|&i| f[i].duration > 0.0).collect();
    by_length.sort_by(|&a, &b| f[a].duration.total_cmp(&f[b].duration));
    for (k, &i) in by_length.iter().enumerate() {
        for &j in &by_length[k + 1..] {
            if f[j].duration - f[i].duration > SAME_LENGTH_S {
                break;
            }
            if same_speeds(&f[i], &f[j]) {
                seen.insert((i.min(j), i.max(j)));
            }
        }
    }
    let mut out: Vec<_> = seen.into_iter().collect();
    out.sort_unstable();
    out
}

fn same_speeds(a: &Facts, b: &Facts) -> bool {
    a.speeds.0 > 0.0 && a.speeds.1 > 0.0 && a.speeds == b.speeds
}

/// Frame samples both videos have (the same keys: the same times, the same projection), as
/// (same picture, compared); samples where either shows no picture don't count. `strict`
/// (titles that aren't the same: another character or outfit dHash can't tell apart): a sample
/// matches only when the fine grids match too.
pub fn compare_frames(
    a: &HashMap<String, Burst>,
    b: &HashMap<String, Burst>,
    strict: bool,
) -> Option<(usize, usize)> {
    let (mut same, mut n) = (0, 0);
    for (key, fa) in a {
        let Some(fb) = b.get(key) else { continue };
        if let Some(d) = burst_distance(fa, fb) {
            n += 1;
            let everywhere = || {
                let cells = |f: &Burst| f.get(BURST_CENTRE).and_then(cells);
                matches!((cells(fa), cells(fb)), (Some(x), Some(y)) if cell_distance(&x, &y) <= SWAP_CELL_MAX)
            };
            if d <= SAME_HASH_MAX && (!strict || everywhere()) {
                same += 1;
            }
        }
    }
    (n > 0).then_some((same, n))
}

/// A centre frame's fine grid (the third part, base64 grey levels).
fn cells(f: &Option<String>) -> Option<Vec<u8>> {
    base64(f.as_deref()?.split(':').nth(2)?)
}

/// Largest difference between two fine grids with the brightness evened out
/// (scene-dedupe-lib.mjs cellDistance); infinite when they can't be compared.
fn cell_distance(a: &[u8], b: &[u8]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return f64::INFINITY;
    }
    let mean = |x: &[u8]| x.iter().map(|&v| v as f64).sum::<f64>() / x.len() as f64;
    let shift = mean(a) - mean(b);
    a.iter()
        .zip(b)
        .map(|(&x, &y)| (x as f64 - y as f64 - shift).abs())
        .fold(0.0, f64::max)
}

fn base64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|&c| c != b'=') {
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn frame(s: &Option<String>) -> Option<(&str, f64)> {
    let mut parts = s.as_deref()?.split(':');
    let hash = parts.next()?;
    let detail = parts.next()?.parse().ok()?;
    Some((hash, detail))
}

/// The fewest differing bits between two bursts' pictures; None when either has no picture at
/// the sample time.
fn burst_distance(a: &Burst, b: &Burst) -> Option<u32> {
    fn picture(f: &Option<String>) -> Option<(&str, f64)> {
        frame(f).filter(|(_, d)| *d >= FRAME_DETAIL_MIN)
    }
    picture(a.get(BURST_CENTRE)?)?;
    picture(b.get(BURST_CENTRE)?)?;
    let mut best = None;
    for x in a.iter().filter_map(picture) {
        for y in b.iter().filter_map(picture) {
            if let Some(d) = hamming(x.0, y.0) {
                best = Some(best.map_or(d, |b: u32| b.min(d)));
            }
        }
    }
    best
}

fn hamming(a: &str, b: &str) -> Option<u32> {
    if a.len() != b.len() {
        return None;
    }
    a.chars().zip(b.chars()).try_fold(0, |n, (x, y)| {
        Some(n + (x.to_digit(16)? ^ y.to_digit(16)?).count_ones())
    })
}

/// Every pair at `floor` or more, best first (ties: the bigger pair first, then by id).
fn judge_all(f: &[Facts], disk: &dyn Disk, t: &Thresholds) -> Vec<(usize, usize, Verdict)> {
    let mut posts: HashMap<usize, Option<Post>> = HashMap::new();
    let mut post_of = |i: usize| {
        posts
            .entry(i)
            .or_insert_with(|| disk.post(f[i].path))
            .clone()
    };
    let mut scripts: HashMap<usize, Option<Vec<(i64, i64)>>> = HashMap::new();
    let mut out = Vec::new();
    for (i, j) in candidates(f) {
        let (a, b) = (&f[i], &f[j]);
        let (title, shared) = title_similarity(&a.words, &b.words);
        let length_close = a.duration > 0.0
            && b.duration > 0.0
            && (a.duration - b.duration).abs() <= SAME_LENGTH_S;
        if title < MIN_TITLE && !(length_close && same_speeds(a, b)) {
            continue;
        }
        let same_script = length_close && same_speeds(a, b) && {
            let mut script = |k: usize| {
                scripts
                    .entry(k)
                    .or_insert_with(|| disk.actions(&f[k].id))
                    .clone()
            };
            matches!((script(i), script(j)), (Some(x), Some(y)) if !x.is_empty() && x == y)
        };
        if title < MIN_TITLE && !same_script {
            continue;
        }
        let swap = character_swap(&a.words, &b.words);
        let trim_close = a.duration > 0.0
            && b.duration > 0.0
            && (a.duration - b.duration).abs()
                <= TRIM_MIN_S.max(TRIM_SHARE * a.duration.max(b.duration));
        let frames = match (disk.bursts(a.path, a.size), disk.bursts(b.path, b.size)) {
            (Some(x), Some(y)) if a.vr == b.vr => compare_frames(x, y, swap || title < 0.99),
            _ => None,
        };
        let same_post = matches!((post_of(i), post_of(j)), (Some(x), Some(y)) if x.kind == y.kind && x.id == y.id);
        let e = Evidence {
            title,
            shared,
            lengths: (a.duration > 0.0 && b.duration > 0.0)
                .then(|| ((a.duration - b.duration).abs(), a.duration.max(b.duration))),
            same_script,
            frames,
            same_artist: a.artist.is_some() && a.artist == b.artist,
            same_post,
            vr_differs: a.vr != b.vr,
            compilation: a.compilation || b.compilation,
            series: numbered_apart(&a.words, &b.words, a.copy, b.copy, trim_close),
            swap,
        };
        let verdict = judge(&e);
        if verdict.confidence >= t.floor_confidence {
            out.push((i, j, verdict));
        }
    }
    out.sort_by(|x, y| {
        y.2.confidence
            .total_cmp(&x.2.confidence)
            .then((f[y.0].size + f[y.1].size).cmp(&(f[x.0].size + f[x.1].size)))
            .then_with(|| f[x.0].id.cmp(&f[y.0].id))
    });
    out
}

/// The confidence a pair needs to be dealt, from the confidences of the pairs still to swipe
/// (best first): `min_confidence` when at least `target` reach it, else the `target`-th best,
/// never under `floor_confidence`.
pub fn bar(best_first: &[f64], t: &Thresholds) -> f64 {
    let above = best_first
        .iter()
        .filter(|&&c| c >= t.min_confidence)
        .count();
    if above >= t.target || t.target == 0 {
        return t.min_confidence;
    }
    best_first
        .get(t.target - 1)
        .copied()
        .unwrap_or(t.floor_confidence)
        .clamp(t.floor_confidence, t.min_confidence)
}

// ── Swipes ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    /// Keep the left video (the pair's first id), delete the right one.
    Left,
    /// Keep the right video, delete the left one.
    Right,
    /// Not duplicates.
    Up,
}

impl Dir {
    pub fn parse(s: &str) -> Option<Dir> {
        match s {
            "left" => Some(Dir::Left),
            "right" => Some(Dir::Right),
            "up" => Some(Dir::Up),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    pub dir: Dir,
    /// Left / right: the video kept and the one to delete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete: Option<String>,
    pub at: String,
}

/// `duplicate-swipes.json`: `{version, pairs: {"<id>-<id>": {dir, keep?, delete?, at}}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marks {
    pub version: u32,
    #[serde(default)]
    pub pairs: BTreeMap<String, Mark>,
}

impl Default for Marks {
    fn default() -> Self {
        Marks {
            version: VERSION,
            pairs: BTreeMap::new(),
        }
    }
}

impl Marks {
    /// Videos marked for deletion → the video kept instead (the first mark's).
    pub fn doomed(&self) -> BTreeMap<&str, &str> {
        let mut out = BTreeMap::new();
        for m in self.pairs.values() {
            if let (Some(keep), Some(del)) = (&m.keep, &m.delete) {
                out.entry(del.as_str()).or_insert(keep.as_str());
            }
        }
        out
    }

    /// Read the file; a missing one is empty, an unreadable or other-version one an error
    /// (never written over).
    pub fn read(user_data: &Path) -> Result<Marks, String> {
        let bytes = match std::fs::read(user_data.join(SWIPES_FILE)) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Marks::default()),
            Err(e) => return Err(format!("{SWIPES_FILE}: {e}")),
        };
        let m: Marks = serde_json::from_slice(&bytes).map_err(|e| format!("{SWIPES_FILE}: {e}"))?;
        if m.version != VERSION {
            return Err(format!(
                "{SWIPES_FILE}: version {}, this CLI reads version {VERSION}",
                m.version
            ));
        }
        Ok(m)
    }

    fn write(&self, user_data: &Path) -> Result<(), String> {
        video_tags::store::write_json_atomic(user_data, SWIPES_FILE, SWIPES_BACKUP, self)
            .map(|_| ())
    }

    /// Record `dir` on pair `id` (replacing an earlier mark) and write the file.
    pub fn set(user_data: &Path, id: &str, dir: Dir, at: String) -> Result<Mark, String> {
        let (a, b) = parse_pair(id).ok_or_else(|| format!("{id} is not a pair"))?;
        let (keep, delete) = match dir {
            Dir::Left => (Some(a.to_string()), Some(b.to_string())),
            Dir::Right => (Some(b.to_string()), Some(a.to_string())),
            Dir::Up => (None, None),
        };
        let mut marks = Marks::read(user_data)?;
        let mark = Mark {
            dir,
            keep,
            delete,
            at,
        };
        marks.pairs.insert(id.to_string(), mark.clone());
        marks.write(user_data)?;
        Ok(mark)
    }

    /// Remove pair `id`'s mark; whether there was one.
    pub fn undo(user_data: &Path, id: &str) -> Result<bool, String> {
        let mut marks = Marks::read(user_data)?;
        if marks.pairs.remove(id).is_none() {
            return Ok(false);
        }
        marks.write(user_data)?;
        Ok(true)
    }
}

// ── The deck ─────────────────────────────────────────────────────────────

/// One side of a pair: what the stats row shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Side {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    pub path: String,
    /// The library source folder's name (`EroScripts`, `FapTap`, `Other` …).
    pub library: Option<String>,
    /// Frames of the video to tap through after the thumbnail (relative `dupes-frames/<id>/<n>.jpg`,
    /// taken on first request, crate::duplicates::frame_image).
    pub frames: Vec<String>,
    pub size: u64,
    pub score: Option<f64>,
    /// 0 = unrated.
    pub rating: f64,
    pub favourite: bool,
    pub vr: bool,
    pub runtime_ms: Option<u64>,
    pub quality: Option<Quality>,
    /// `handmade` / `generated` / `single`; None while the backend still checks.
    pub axes: Option<String>,
    /// When its post was made or its scene released (`posted_from`), else the file's date
    /// (`added`).
    pub posted: Option<String>,
    pub posted_from: &'static str,
    pub post_url: Option<String>,
    /// The backend's thumbnail path; the page puts the backend's origin in front.
    pub thumb: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairCard {
    /// Always SOURCE.
    pub source: &'static str,
    /// `<left id>-<right id>`.
    pub id: String,
    pub confidence: f64,
    pub kind: Kind,
    pub reasons: Vec<String>,
    pub left: Side,
    pub right: Side,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupesDeck {
    /// Pairs dealt (at `bar` or more) left to swipe, all of them.
    pub remaining: usize,
    /// Videos marked for deletion, still in the library.
    pub wanted: usize,
    pub marked_bytes: u64,
    /// The confidence this deal needs (crate::duplicates::bar).
    pub bar: f64,
    pub min_confidence: f64,
    pub floor_confidence: f64,
    /// Pairs at the floor or more still to swipe (dealt or not).
    pub candidates: usize,
    /// Pairs swiped "not duplicates".
    pub distinct: usize,
    pub backend_port: Option<u16>,
    pub drives: Vec<free_space::Drive>,
    pub problems: Vec<String>,
    pub cards: Vec<PairCard>,
}

fn side(x: &Facts, lib: &Library, disk: &dyn Disk) -> Side {
    let v = x.v;
    let p = Path::new(x.path);
    let post = disk.post(x.path);
    let (posted, posted_from) = match post.as_ref().and_then(|p| disk.posted(p)) {
        Some((at, from)) => (Some(at), from),
        None => (
            v.get("dateAdded")
                .and_then(Value::as_u64)
                .filter(|&n| n > 0)
                .map(|ms| {
                    video_tags::timefmt::iso_utc(
                        std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms),
                    )
                }),
            "added",
        ),
    };
    Side {
        id: x.id.clone(),
        frames: FRAME_SHARES
            .iter()
            .map(|n| format!("dupes-frames/{}/{n}.jpg", x.id))
            .collect(),
        title: v
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| Some(p.file_stem()?.to_string_lossy().into_owned()))
            .unwrap_or_else(|| x.id.clone()),
        author: lib
            .artists
            .get(&x.id)
            .cloned()
            .or_else(|| artist_folder(p, x.source)),
        path: x.path.to_string(),
        library: x
            .source
            .file_name()
            .map(|s| s.to_string_lossy().into_owned()),
        size: x.size,
        score: v.get("score").and_then(Value::as_f64),
        rating: v.get("rating").and_then(Value::as_f64).unwrap_or(0.0),
        favourite: v.get("favorite").and_then(Value::as_bool) == Some(true),
        vr: x.vr,
        runtime_ms: (x.duration > 0.0).then(|| (x.duration * 1000.0).round() as u64),
        quality: disk.quality(x.path),
        axes: lib.axes.get(x.path).cloned(),
        posted,
        posted_from,
        post_url: post.and_then(|p| p.url),
        thumb: v
            .get("thumbUrl")
            .and_then(Value::as_str)
            .map(str::to_string),
        tags: v
            .get("categories")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|c| Some(c.as_str()?.to_string()))
            .collect(),
    }
}

/// The tab from the backend's answers and the disk: pairs not swiped and with no side marked
/// for deletion, at the deal's bar or more, best first; `limit` cards not in `exclude`
/// (`dupes:<pair>`).
pub fn build(
    lib: &Library,
    marks: &Marks,
    t: &Thresholds,
    sources: &[PathBuf],
    disk: &dyn Disk,
    limit: usize,
    exclude: &HashSet<String>,
) -> DupesDeck {
    let f = facts(lib, sources, disk, &t.compilation_words);
    let doomed = marks.doomed();
    let mut deck = DupesDeck {
        min_confidence: t.min_confidence,
        floor_confidence: t.floor_confidence,
        distinct: marks.pairs.values().filter(|m| m.dir == Dir::Up).count(),
        ..DupesDeck::default()
    };
    for x in &f {
        if doomed.contains_key(x.id.as_str()) {
            deck.wanted += 1;
            deck.marked_bytes += x.size;
        }
    }
    let open: Vec<(usize, usize, Verdict)> = judge_all(&f, disk, t)
        .into_iter()
        .filter(|(i, j, _)| {
            let (a, b) = (&f[*i].id, &f[*j].id);
            !marks.pairs.contains_key(&pair_id(a, b))
                && !doomed.contains_key(a.as_str())
                && !doomed.contains_key(b.as_str())
        })
        .collect();
    deck.candidates = open.len();
    let confidences: Vec<f64> = open.iter().map(|p| p.2.confidence).collect();
    deck.bar = bar(&confidences, t);
    let dealt: Vec<_> = open
        .into_iter()
        .filter(|p| p.2.confidence >= deck.bar)
        .collect();
    deck.remaining = dealt.len();
    for (i, j, verdict) in dealt {
        let (l, r) = if f[i].id < f[j].id { (i, j) } else { (j, i) };
        let id = pair_id(&f[l].id, &f[r].id);
        if exclude.contains(&format!("{SOURCE}:{id}")) {
            continue;
        }
        if deck.cards.len() >= limit {
            break;
        }
        deck.cards.push(PairCard {
            source: SOURCE,
            id,
            confidence: verdict.confidence,
            kind: verdict.kind,
            reasons: verdict.reasons,
            left: side(&f[l], lib, disk),
            right: side(&f[r], lib, disk),
        });
    }
    deck
}

// ── The real disk ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct FrameEntry {
    size: u64,
    #[serde(default)]
    bursts: HashMap<String, Burst>,
}

/// userData's caches and sidecars, each read once per request and only when needed.
pub struct RealDisk<'a> {
    user_data: &'a Path,
    backend: Option<&'a Backend>,
    frames: OnceCell<HashMap<String, FrameEntry>>,
    probe: OnceCell<Value>,
    dims: OnceCell<Value>,
    links: OnceCell<Value>,
    state: OnceCell<Value>,
    scouts: RefCell<HashMap<Source, Value>>,
    sidecars: RefCell<HashMap<PathBuf, Option<Post>>>,
    pub problems: RefCell<Vec<String>>,
}

impl<'a> RealDisk<'a> {
    pub fn new(user_data: &'a Path, backend: Option<&'a Backend>) -> Self {
        RealDisk {
            user_data,
            backend,
            frames: OnceCell::new(),
            probe: OnceCell::new(),
            dims: OnceCell::new(),
            links: OnceCell::new(),
            state: OnceCell::new(),
            scouts: RefCell::new(HashMap::new()),
            sidecars: RefCell::new(HashMap::new()),
            problems: RefCell::new(Vec::new()),
        }
    }

    fn json(&self, cell: &'a OnceCell<Value>, name: &str) -> &Value {
        cell.get_or_init(|| {
            report::read_json(self.user_data, name, &mut self.problems.borrow_mut())
        })
    }
}

fn sidecar(dir: &Path) -> Option<Post> {
    let read = |name: &str| -> Option<Value> {
        serde_json::from_slice(&std::fs::read(dir.join(name)).ok()?).ok()
    };
    let url = |v: &Value| report::str_of(v, "url");
    if let Some(v) = read("eroscripts.json") {
        let id = report::id_of(v.get("topicId")?)?;
        return Some(Post {
            kind: "eroscripts",
            id: id.to_string(),
            url: url(&v).or_else(|| Some(format!("{}{id}", report::FORUM_TOPIC))),
        });
    }
    for (file, kind, key) in [
        ("faptap.json", "faptap", "faptapId"),
        ("vrb.json", "vrb", "id"),
        ("pmvhaven.json", "pmvhaven", "id"),
        ("slr.json", "slr", "sceneId"),
    ] {
        if let Some(v) = read(file) {
            let id = crate::swipes::id_key(v.get(key)?)?;
            return Some(Post {
                kind,
                id,
                url: url(&v),
            });
        }
    }
    None
}

impl Disk for RealDisk<'_> {
    fn size(&self, path: &str) -> Option<u64> {
        free_space::file_size(path)
    }

    fn post(&self, path: &str) -> Option<Post> {
        let dir = Path::new(path).parent()?.to_path_buf();
        let found = self
            .sidecars
            .borrow_mut()
            .entry(dir.clone())
            .or_insert_with(|| sidecar(&dir))
            .clone();
        found.or_else(|| {
            let links = self.json(&self.links, LINKS_FILE);
            let link = links.get("links").unwrap_or(links).get(path)?;
            let id = report::id_of(link.get("topicId")?)?;
            Some(Post {
                kind: "eroscripts",
                id: id.to_string(),
                url: report::str_of(link, "topicUrl")
                    .or_else(|| Some(format!("{}{id}", report::FORUM_TOPIC))),
            })
        })
    }

    fn bursts(&self, path: &str, size: u64) -> Option<&HashMap<String, Burst>> {
        let cache = self.frames.get_or_init(|| {
            match std::fs::read(self.user_data.join(FRAME_CACHE_FILE)) {
                Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
                    self.problems
                        .borrow_mut()
                        .push(format!("{FRAME_CACHE_FILE}: {e}"));
                    HashMap::new()
                }),
                Err(_) => HashMap::new(),
            }
        });
        cache
            .get(path)
            .filter(|e| e.size == size)
            .map(|e| &e.bursts)
    }

    fn actions(&self, id: &str) -> Option<Vec<(i64, i64)>> {
        let script = self.backend?.get(&format!("/api/media/script/{id}")).ok()?;
        script
            .get("actions")?
            .as_array()?
            .iter()
            .map(|a| {
                Some((
                    a.get("at")?.as_f64()? as i64,
                    a.get("pos")?.as_f64()? as i64,
                ))
            })
            .collect()
    }

    fn quality(&self, path: &str) -> Option<Quality> {
        let from = |v: &Value| -> Option<Quality> {
            Some(Quality {
                width: v.get("width")?.as_u64()? as u32,
                height: v.get("height")?.as_u64()? as u32,
                fps: v.get("fps").and_then(Value::as_f64),
            })
        };
        let probe = self.json(&self.probe, PROBE_CACHE_FILE);
        if let Some(q) = probe.get(path).and_then(from) {
            return Some(q);
        }
        let dims = self.json(&self.dims, DIMS_CACHE_FILE);
        if let Some(q) = dims.get("entries").and_then(|e| e.get(path)).and_then(from) {
            return Some(q);
        }
        ffprobe(path)
    }

    fn posted(&self, post: &Post) -> Option<(String, &'static str)> {
        if post.kind == "eroscripts" {
            let state = self.json(&self.state, report::STATE_FILE);
            let at = report::str_of(state.get("topics")?.get(&post.id)?, "created")?;
            return Some((at, "posted"));
        }
        let source = Source::parse(post.kind)?;
        let mut scouts = self.scouts.borrow_mut();
        let file = scouts.entry(source).or_insert_with(|| {
            report::read_json(
                self.user_data,
                source.file(),
                &mut self.problems.borrow_mut(),
            )
        });
        let (_, scene) = scenes::scenes(source, file).find(|(id, _)| *id == post.id)?;
        Some((report::str_of(scene, "date")?, "released"))
    }
}

/// Width, height and frame rate of a file the caches don't know (`ffprobe`); None without one.
fn ffprobe(path: &str) -> Option<Quality> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,avg_frame_rate",
            "-of",
            "json",
            path,
        ])
        .output()
        .ok()?;
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    let s = v.get("streams")?.get(0)?;
    let fps = s
        .get("avg_frame_rate")
        .and_then(Value::as_str)
        .and_then(|r| {
            let (n, d) = r.split_once('/')?;
            let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
            (d > 0.0).then(|| n / d)
        });
    Some(Quality {
        width: s.get("width")?.as_u64()? as u32,
        height: s.get("height")?.as_u64()? as u32,
        fps,
    })
}

// ── Frames ───────────────────────────────────────────────────────────────

/// Where the tap-through frames are taken, in % of the video's length.
pub const FRAME_SHARES: [u32; 4] = [20, 40, 60, 80];
/// Taken frames: `<userData>/scout-frames/library/<video id>/<n>.jpg`.
pub const FRAMES_DIR: &str = "library";

/// The video id and share a request path names: `/dupes-frames/<id>/<n>.jpg`, `n` one of
/// FRAME_SHARES, the id letters and digits.
pub fn frame_request(path: &str) -> Option<(&str, u32)> {
    let rest = path.strip_prefix("/dupes-frames/")?;
    let (id, name) = rest.split_once('/')?;
    let n: u32 = name.strip_suffix(".jpg")?.parse().ok()?;
    (swipe_id_ok(id) && FRAME_SHARES.contains(&n) && name == format!("{n}.jpg")).then_some((id, n))
}

/// `GET /dupes-frames/<id>/<n>.jpg`: the frame `n` % into a library video (the left eye of a
/// VR one), 640 px wide, taken with ffmpeg the first time and kept in FRAMES_DIR. 404 for a
/// path of another shape or a video the backend doesn't list.
pub fn frame_image(user_data: &Path, path: &str) -> Result<Vec<u8>, (u16, String)> {
    let (id, n) = frame_request(path).ok_or((404, "Not found".to_string()))?;
    let dir = user_data.join(crate::frames::DIR).join(FRAMES_DIR).join(id);
    let file = dir.join(format!("{n}.jpg"));
    if let Ok(b) = std::fs::read(&file) {
        return Ok(b);
    }
    let (_, backend, _) = free_space::app(user_data).map_err(|e| (500, e))?;
    let videos = backend
        .get("/api/remote/videos")
        .map_err(|e| (503, free_space::backend_problem(e)))?;
    let v = videos
        .get("videos")
        .unwrap_or(&videos)
        .as_array()
        .into_iter()
        .flatten()
        .find(|v| v.get("id").and_then(id_text).as_deref() == Some(id))
        .ok_or((404, format!("{id} is not in the library")))?;
    let video = v
        .get("path")
        .and_then(Value::as_str)
        .ok_or((404, format!("{id} has no path")))?;
    let duration = v.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
    let vr = v.get("isVR").and_then(Value::as_bool).unwrap_or(false);
    std::fs::create_dir_all(&dir).map_err(|e| (500, format!("{}: {e}", dir.display())))?;
    let tmp = dir.join(format!("{n}.{}.tmp.jpg", std::process::id()));
    let filter = format!("{}scale=640:-2", if vr { "crop=iw/2:ih:0:0," } else { "" });
    let at = format!("{:.2}", duration * n as f64 / 100.0);
    let ok = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-ss",
            &at,
            "-i",
            video,
            "-frames:v",
            "1",
            "-vf",
            &filter,
            "-q:v",
            "4",
        ])
        .arg(&tmp)
        .status()
        .is_ok_and(|s| s.success());
    if !ok || std::fs::rename(&tmp, &file).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return Err((502, format!("no frame at {at} s of {video}")));
    }
    std::fs::read(&file).map_err(|e| (500, e.to_string()))
}

// ── Requests ─────────────────────────────────────────────────────────────

/// `GET /api/deck?stack=dupes`: the next `limit` pairs not in `exclude` (`dupes:<pair>`).
pub fn load(user_data: &Path, limit: usize, exclude: &HashSet<String>) -> DupesDeck {
    let (sources, backend, port) = match free_space::app(user_data) {
        Ok(a) => a,
        Err(e) => {
            return DupesDeck {
                problems: vec![e],
                ..DupesDeck::default()
            };
        }
    };
    let mut problems = Vec::new();
    let marks = Marks::read(user_data).unwrap_or_else(|e| {
        problems.push(e);
        Marks::default()
    });
    let t = thresholds();
    let mut deck = match Library::fetch(&backend) {
        Ok(lib) => {
            let disk = RealDisk::new(user_data, Some(&backend));
            let deck = build(&lib, &marks, &t, &sources, &disk, limit, exclude);
            problems.extend(disk.problems.take());
            deck
        }
        Err(e) => {
            problems.push(free_space::backend_problem(e));
            DupesDeck {
                min_confidence: t.min_confidence,
                floor_confidence: t.floor_confidence,
                ..DupesDeck::default()
            }
        }
    };
    deck.backend_port = Some(port);
    deck.drives = free_space::drives(&sources);
    deck.problems.extend(problems);
    deck
}

/// `POST /api/duplicates/swipe {id, dir}`: mark a pair of videos the backend lists.
pub fn swipe(user_data: &Path, id: &str, dir: Dir) -> Result<Value, (u16, String)> {
    let (a, b) =
        parse_pair(id).ok_or_else(|| (400, format!("{id:?} is not a pair of video ids")))?;
    let (_, backend, _) = free_space::app(user_data).map_err(|e| (500, e))?;
    let lib = Library::fetch(&backend).map_err(|e| (503, free_space::backend_problem(e)))?;
    let known: HashSet<String> = lib
        .videos
        .iter()
        .filter_map(|v| v.get("id").and_then(id_text))
        .collect();
    for x in [a, b] {
        if !known.contains(x) {
            return Err((400, format!("{x} is not in the library")));
        }
    }
    let m = Marks::set(user_data, id, dir, video_tags::timefmt::now_iso()).map_err(|e| (500, e))?;
    Ok(json!({"id": id, "dir": m.dir, "keep": m.keep, "delete": m.delete, "at": m.at}))
}

/// What a delete does: ask the backend to delete these (video, kept video) ids, drop these
/// marks (a side gone from the library), report the rest.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// (delete id, keep id, bytes)
    pub delete: Vec<(String, String, u64)>,
    pub stale: Vec<String>,
    pub skipped: Vec<(String, &'static str)>,
}

pub fn plan(
    lib: &Library,
    marks: &Marks,
    sources: &[PathBuf],
    size_of: impl Fn(&str) -> Option<u64>,
) -> Plan {
    let paths: HashMap<String, &str> = lib
        .videos
        .iter()
        .filter_map(|v| {
            Some((
                v.get("id").and_then(id_text)?,
                v.get("path").and_then(Value::as_str)?,
            ))
        })
        .collect();
    let doomed = marks.doomed();
    let mut p = Plan::default();
    let mut planned = HashSet::new();
    for (pair, m) in &marks.pairs {
        let (Some(keep), Some(del)) = (&m.keep, &m.delete) else {
            continue;
        };
        let (Some(del_path), Some(_)) = (paths.get(del), paths.get(keep)) else {
            p.stale.push(pair.clone());
            continue;
        };
        if doomed.contains_key(keep.as_str()) && doomed.get(del.as_str()) == Some(&keep.as_str()) {
            // Kept here, deleted in another pair: deleting both would lose the video.
            p.skipped
                .push((pair.clone(), "the kept copy is marked for deletion too"));
            continue;
        }
        if !planned.insert(del.clone()) {
            continue;
        }
        if !sources.iter().any(|s| Path::new(del_path).starts_with(s)) {
            p.skipped.push((pair.clone(), "not in a library source"));
        } else if let Some(size) = size_of(del_path) {
            p.delete.push((del.clone(), keep.clone(), size));
        } else {
            p.skipped.push((pair.clone(), "not on disk"));
        }
    }
    p
}

/// `POST /api/duplicates/delete`: the desktop deletes every copy marked for deletion (as a
/// duplicate of the one kept); the marks of those pairs go once the backend took it, and so do
/// marks of pairs a side of which left the library.
pub fn delete(user_data: &Path) -> Result<Value, (u16, String)> {
    let (sources, backend, _) = free_space::app(user_data).map_err(|e| (500, e))?;
    let marks = Marks::read(user_data).map_err(|e| (500, e))?;
    let lib = Library::fetch(&backend).map_err(|e| (503, free_space::backend_problem(e)))?;
    let p = plan(&lib, &marks, &sources, free_space::file_size);
    let mut done = HashSet::new();
    let mut bytes = 0u64;
    let mut errors = Vec::new();
    for (del, keep, size) in &p.delete {
        match backend.delete_duplicate(&Value::String(del.clone()), &Value::String(keep.clone())) {
            Ok(()) => {
                done.insert(del.clone());
                bytes += size;
            }
            Err(e) => errors.push(format!("{del}: {}", free_space::backend_problem(e))),
        }
    }
    let gone: Vec<String> = marks
        .pairs
        .iter()
        .filter(|(pair, m)| {
            p.stale.contains(pair) || m.delete.as_ref().is_some_and(|d| done.contains(d))
        })
        .map(|(pair, _)| pair.clone())
        .collect();
    if !gone.is_empty() {
        let mut now = Marks::read(user_data).map_err(|e| (500, e))?;
        for pair in &gone {
            now.pairs.remove(pair);
        }
        now.write(user_data).map_err(|e| (500, e))?;
    }
    Ok(json!({
        "deleted": done.len(),
        "bytes": bytes,
        "skipped": p.skipped.iter().map(|(id, why)| json!({"id": id, "why": why})).collect::<Vec<_>>(),
        "errors": errors,
    }))
}

#[cfg(test)]
mod tests;
