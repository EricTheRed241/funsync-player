//! What the Electron renderer derives after each scan, computed by the
//! server into the same keys of the old `config.json` shape, so the web
//! remote (and later the headsets) show it:
//!
//! - script tags (`tags`): scan sidecars, `eroscripts-links.json`, then
//!   `video-tags.json` + `pornstar-tags.json` (renderer/js/eroscripts-links.js,
//!   video-tags.js), all read from the old app's folder (`side_dir`), never
//!   written;
//! - auto categories, artist playlists and their cleanup, auto collections,
//!   score lists, smart playlists (`lists`: renderer/js/auto-categories.js,
//!   artist-playlists.js, playlist-cleanup.js, auto-collections.js,
//!   score-bands.js, smart-playlists.js), with the same bookkeeping keys;
//! - speed stats and preference scores (`score`: backend/routes/media.py
//!   `_compute_speed_stats`, renderer/js/preference-score.js,
//!   library-scores.js, favourite-artists.js).
//!
//! Values come from renderer/js/content-preferences.json; the ones that live
//! only in content-preferences.js are mirrored in `mirror.json` until they
//! move into the shared JSON.
//!
//! Known differences from the old app: VR is `vr::detect` (VR Format, Mark
//! as VR / Flat, file name, cached frame size) without the old backend's
//! look at the frames, so a VR video only the frames tell apart scores and
//! lists as flat; it's only taken out of the VR category when the user
//! marked it flat. The axis kind comes from the scan's companion files (axes
//! embedded in the main script aren't read).

mod lists;
mod score;
mod tags;

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, LazyLock};

use axum::Router;
use axum::extract::State;
use axum::response::Json;
use axum::routing::get;
use regex::Regex;
use serde_json::{Map, Value, json};
use unicode_normalization::UnicodeNormalization;

use crate::data::{self, Library};
use crate::scan::Video;
use crate::server::{AppState, Snapshot};

pub use score::{effective_tag_scores, normalize_tag_score};
pub use tags::SideFiles;

/// `renderer/js/content-preferences.json`, parsed once.
pub fn content_preferences() -> &'static Value {
    &RULES.prefs
}

/// Run every rule, in the old app's order, after a scan. Skipped unless the
/// snapshot is a complete scan (no unreadable source): a disconnected drive
/// would make its videos look gone.
pub fn refresh(lib: &mut Library, snap: &Snapshot) {
    let side = SideFiles::load(side_dir(lib).as_deref());
    refresh_with(lib, snap, Some(&side), data::now_ms(), |_| None);
}

/// Where the old app keeps its side files: beside the `config.json` the
/// library was imported from, else the old app's default folder.
pub fn side_dir(lib: &Library) -> Option<std::path::PathBuf> {
    lib.imported_from
        .as_deref()
        .and_then(|c| std::path::Path::new(c).parent().map(Into::into))
        .or_else(data::default_old_user_data)
}

/// `refresh` with the side files, the clock and the videos' known frame
/// sizes (`(width, height)`, for the VR decision) given. `side` none: after
/// an edit (`AppState::library_changed`), so the post tags aren't merged
/// again and no playlist is cleaned up (the old app does both after scans
/// only).
pub fn refresh_with(
    lib: &mut Library,
    snap: &Snapshot,
    side: Option<&SideFiles>,
    now: u64,
    dims: impl Fn(&str) -> Option<(u32, u32)>,
) {
    if !snap.failed_paths.is_empty() || snap.videos.is_empty() {
        return;
    }
    if let Some(side) = side {
        tags::merge(lib, &snap.videos, side, now);
    }
    let cx = Ctx {
        videos: &snap.videos,
        embedded: &snap.embedded_axes,
        vr: snap
            .videos
            .iter()
            .map(|v| {
                (
                    v.path.as_str(),
                    crate::vr::detect(lib, v, dims(&v.path)).is_3d,
                )
            })
            .collect(),
        sources: lib
            .sources()
            .into_iter()
            .map(|s| s.path)
            .filter(|p| !p.is_empty())
            .collect(),
        now,
    };
    lists::categories(lib, &cx);
    lists::artist_playlists(lib, &cx);
    if side.is_some() {
        lists::playlist_cleanup(lib, &cx);
    }
    lists::auto_collections(lib, &cx);
    score::speed_stats(lib, &cx);
    score::preference_scores(lib, &cx);
    lists::score_bands(lib, &cx);
    lists::smart_playlists(lib, &cx);
}

pub(crate) struct Ctx<'a> {
    videos: &'a [Video],
    embedded: &'a HashSet<String>,
    vr: HashMap<&'a str, bool>,
    sources: Vec<String>,
    now: u64,
}

pub(crate) struct Rules {
    /// renderer/js/content-preferences.json
    prefs: Value,
    /// mirror.json
    mirror: Value,
    /// `named` tag-score group → its title pattern (MONSTER_TITLE with
    /// WEREWOLF_TOKEN, FEET_TITLE, FFM_TITLE).
    named: HashMap<String, Regex>,
    three_d: Regex,
    /// AUTO_COLLECTIONS title patterns, by key.
    collection_titles: HashMap<String, Regex>,
}

pub(crate) static RULES: LazyLock<Rules> = LazyLock::new(|| {
    let prefs: Value = serde_json::from_str(include_str!(
        "../../../../renderer/js/content-preferences.json"
    ))
    .expect("content-preferences.json");
    let mirror: Value = serde_json::from_str(include_str!("mirror.json")).expect("mirror.json");
    // `(?<![\p{L}\p{N}])…(?![\p{L}\p{N}])` without look-around: fine for is_match.
    let word = |body: &str| format!(r"(?:^|[^\p{{L}}\p{{N}}])(?:{body})(?:$|[^\p{{L}}\p{{N}}])");
    let title = |name: &str| {
        let mut re = format!("(?i){}", word(mirror["titleWords"][name].as_str().unwrap()));
        if let Some(any) = mirror["titleAnywhere"][name].as_str() {
            re += &format!("|{any}");
        }
        if name == "monster" {
            re += &format!("|(?-i:{})", word(mirror["werewolfToken"].as_str().unwrap()));
        }
        Regex::new(&re).unwrap()
    };
    let named = ["monster", "feet", "ffm"]
        .into_iter()
        .map(|n| (n.to_string(), title(n)))
        .collect();
    // JS `\b(?:…)\b` without the u flag: ASCII word characters.
    let collection_titles = mirror["autoCollections"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| {
            let t = d["title"].as_str()?;
            let re = format!("(?:^|[^0-9A-Za-z_])(?:{t})(?:$|[^0-9A-Za-z_])");
            Some((d["key"].as_str()?.to_string(), Regex::new(&re).unwrap()))
        })
        .collect();
    Rules {
        three_d: title("threeD"),
        named,
        collection_titles,
        prefs,
        mirror,
    }
});

/// Names compared like artist folders: NFC, trimmed, lower case.
pub fn fold(s: &str) -> String {
    s.nfc().collect::<String>().trim().to_lowercase()
}

/// Category / collection names: trimmed, lower case.
pub(crate) fn name_key(s: &str) -> String {
    s.trim().to_lowercase()
}

/// The strings of a JSON array (anything else: none).
pub(crate) fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(String::from))
        .collect()
}

/// `v` as an object, replaced by `{}` when it isn't one.
pub(crate) fn obj_mut(v: &mut Value) -> &mut Map<String, Value> {
    if !v.is_object() {
        *v = json!({});
    }
    v.as_object_mut().unwrap()
}

/// `v` as an array, replaced by `[]` when it isn't one.
pub(crate) fn arr_mut(v: &mut Value) -> &mut Vec<Value> {
    if !v.is_array() {
        *v = json!([]);
    }
    v.as_array_mut().unwrap()
}

/// A number as JS writes it to JSON: whole numbers without `.0`.
pub(crate) fn num(x: f64) -> Value {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        json!(x as i64)
    } else {
        json!(x)
    }
}

/// A `crypto.randomUUID()`-shaped id for a new playlist, category or
/// collection.
pub(crate) fn new_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let seed = format!(
        "{:?}-{}-{}",
        std::time::SystemTime::now(),
        std::process::id(),
        N.fetch_add(1, AtomicOrdering::Relaxed)
    );
    let h = format!("{:x}", md5::compute(seed));
    format!(
        "{}-{}-4{}-a{}-{}",
        &h[..8],
        &h[8..12],
        &h[13..16],
        &h[17..20],
        &h[20..32]
    )
}

/// An approximation of `localeCompare` (ICU root collation): case-blind,
/// spaces, punctuation and symbols before digits before letters, with
/// `numeric` comparing digit runs by value.
pub(crate) fn locale_cmp(a: &str, b: &str, numeric: bool) -> Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    let digits = |it: &mut std::iter::Peekable<std::str::Chars>| {
        let mut s = String::new();
        while let Some(c) = it.next_if(char::is_ascii_digit) {
            s.push(c);
        }
        s.trim_start_matches('0').to_string()
    };
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(c), Some(d)) if numeric && c.is_ascii_digit() && d.is_ascii_digit() => {
                let (m, n) = (digits(&mut x), digits(&mut y));
                let o = m.len().cmp(&n.len()).then(m.cmp(&n));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                if c != d {
                    let class = |c: char| (c.is_alphanumeric() as u8) + (c.is_alphabetic() as u8);
                    return (class(c), c).cmp(&(class(d), d));
                }
                x.next();
                y.next();
            }
        }
    }
}

/// A video's title as the collections and lists sort and match it
/// (auto-collections.js `titleOf`): file name without its extension, NFC,
/// lower case, underscores as spaces.
pub(crate) fn list_title(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    };
    let lower = stem.nfc().collect::<String>().to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut run = false;
    for c in lower.chars() {
        if c != '_' {
            out.push(c);
        } else if !run {
            out.push(' ');
        }
        run = c == '_';
    }
    out
}

/// `handmade` (a hand-made companion axis, or axes embedded in the main
/// script: `embedded`, from `Snapshot::embedded_axes`), `generated` (only
/// generated companions) or `single`; none without a script (backend
/// `axis_kind`).
pub fn axis_kind(v: &Video, embedded: &HashSet<String>) -> Option<&'static str> {
    v.funscript_path.as_ref()?;
    Some(
        if embedded.contains(&v.path) || v.axes.iter().any(|a| !a.generated) {
            "handmade"
        } else if v.axes.is_empty() {
            "single"
        } else {
            "generated"
        },
    )
}

/// The videos whose main script carries axes of its own (the old backend's
/// `embeddedAxes`); reads every main script, so once per scan.
pub fn embedded_axes(videos: &[Video]) -> HashSet<String> {
    videos
        .iter()
        .filter(|v| {
            let Some(p) = &v.funscript_path else {
                return false;
            };
            std::fs::read(p)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .is_some_and(|s| !crate::devices::script::embedded(&s).is_empty())
        })
        .map(|v| v.path.clone())
        .collect()
}

/// The old backend's flag endpoints the Rust CLIs read (age-screen,
/// scout-report): `{"flags": {path: …}, "pending": 0}`.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/media/vr-flags", get(vr_flags))
        .route("/api/media/axis-flags", get(axis_flags))
}

/// Every video's VR decision, with the cached frame sizes.
async fn vr_flags(State(st): State<Arc<AppState>>) -> Json<Value> {
    let snap = st.snapshot.read().await.clone();
    let lib = st.library.read().await;
    let flags: Map<String, Value> = snap
        .videos
        .iter()
        .map(|v| {
            let dims = st.media.cached(&v.path).map(|m| (m.width, m.height));
            (
                v.path.clone(),
                json!(crate::vr::detect(&lib, v, dims).is_3d),
            )
        })
        .collect();
    Json(json!({"flags": flags, "pending": 0}))
}

/// Every scripted video's axis kind.
async fn axis_flags(State(st): State<Arc<AppState>>) -> Json<Value> {
    let snap = st.snapshot.read().await.clone();
    let flags: Map<String, Value> = snap
        .videos
        .iter()
        .filter_map(|v| Some((v.path.clone(), json!(axis_kind(v, &snap.embedded_axes)?))))
        .collect();
    Json(json!({"flags": flags, "pending": 0}))
}

/// An artist's own name for `name` (ARTIST_ALIASES); others as given.
pub fn canonical_artist(name: &str) -> String {
    let key = fold(name);
    RULES.mirror["artistAliases"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(alias, _)| fold(alias) == key)
        .and_then(|(_, artist)| artist.as_str())
        .unwrap_or(name)
        .to_string()
}

/// The artist of a video: the first folder under its source when the video
/// sits at least two folders deeper (`<artist>/<scene>/<video>`), under its
/// own name for an alias; none for NOT_AN_ARTIST folders.
pub fn artist_of(path: &str, sources: &[String]) -> Option<String> {
    let p = path.replace('\\', "/");
    let fp = fold(&p);
    let root = sources
        .iter()
        .map(|s| s.replace('\\', "/").trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty() && fp.starts_with(&format!("{}/", fold(s))))
        .reduce(|a, b| if b.len() > a.len() { b } else { a })?;
    let parts: Vec<&str> = p.get(root.len() + 1..)?.split('/').collect();
    if parts.len() < 3 {
        return None;
    }
    let artist = canonical_artist(parts[0].trim());
    let not_artist = strs(&RULES.mirror["notAnArtist"]).contains(&fold(&artist));
    (!artist.is_empty() && !not_artist).then_some(artist)
}

/// The favourite artists: FAVOURITES with the user's changes
/// (`library.favouriteArtists`: `true` adds, `false` removes), aliases as
/// the artist they name. `(fold, name)` pairs.
pub fn favourites(lib: &Library) -> Vec<(String, String)> {
    let key_of = |n: &str| fold(&canonical_artist(n.trim()));
    let mut out: Vec<(String, String)> = Vec::new();
    for n in strs(&RULES.prefs["favourites"]) {
        let (k, name) = (key_of(&n), canonical_artist(&n));
        match out.iter_mut().find(|(x, _)| *x == k) {
            Some(e) => e.1 = name,
            None => out.push((k, name)),
        }
    }
    for (n, fav) in lib.lib()["favouriteArtists"]
        .as_object()
        .into_iter()
        .flatten()
    {
        let k = key_of(n);
        if k.is_empty() {
            continue;
        }
        if fav == false {
            out.retain(|(x, _)| *x != k);
        } else if fav == true && !out.iter().any(|(x, _)| *x == k) {
            out.push((k, canonical_artist(n.trim())));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artist_is_the_first_folder_two_levels_up() {
        let src = vec!["/L/".to_string(), "/L/Sub".to_string()];
        assert_eq!(
            artist_of("/L/Ann/Scene/v.mp4", &src).as_deref(),
            Some("Ann")
        );
        assert_eq!(artist_of("/L/Ann/v.mp4", &src), None);
        assert_eq!(
            artist_of("/L/Sub/Bob/S/v.mp4", &src).as_deref(),
            Some("Bob")
        );
        assert_eq!(artist_of("/l/pmv/S/v.mp4", &src), None, "NOT_AN_ARTIST");
        assert_eq!(
            artist_of("/L/RondoudouMedia/S/v.mp4", &src).as_deref(),
            Some("Rondoudou"),
            "alias"
        );
        assert_eq!(artist_of("/Other/Ann/S/v.mp4", &src), None);
    }

    #[test]
    fn favourites_apply_the_users_changes() {
        let mut lib = Library::default();
        lib.lib_mut("favouriteArtists");
        lib.config["settings"]["library"]["favouriteArtists"] =
            json!({"ScyllaHMV": false, "rondoudou media": true, "agapilord": true});
        let f: Vec<String> = favourites(&lib).into_iter().map(|x| x.1).collect();
        assert!(!f.contains(&"ScyllaHMV".to_string()));
        assert!(f.contains(&"Rondoudou".to_string()));
        assert_eq!(f.iter().filter(|n| fold(n) == "agapilord").count(), 1);
    }

    #[test]
    fn titles_and_collation() {
        assert_eq!(list_title("/a/My__Scene_X.Final.mp4"), "my scene x.final");
        assert_eq!(locale_cmp("Scene 9", "scene 10", true), Ordering::Less);
        assert_eq!(locale_cmp("Scene 9", "scene 10", false), Ordering::Greater);
        assert_eq!(locale_cmp("ABC", "abc", false), Ordering::Equal);
        assert_eq!(
            locale_cmp("A - ✨B", "a - victim", false),
            Ordering::Less,
            "symbols first"
        );
        assert_eq!(locale_cmp("a_1", "a1", false), Ordering::Less);
    }

    #[test]
    fn refresh_runs_only_after_a_complete_scan() {
        let v: Video =
            serde_json::from_value(json!({"id": "x", "name": "n", "path": "/L/A/s/v.mp4",
            "ext": ".mp4", "funscriptPath": null, "fuzzyMatched": false, "subtitlePath": null,
            "variants": [], "axes": [], "sourceName": "s", "dateAdded": 0.0, "size": 0,
            "eroscripts": {"topicId": 5, "url": null, "tags": ["creampie", "monster"]}}))
            .unwrap();
        let mut snap = Snapshot {
            videos: vec![v],
            failed_paths: vec!["/gone".into()],
            ..Default::default()
        };
        let mut lib = Library::default();
        refresh_with(&mut lib, &snap, Some(&SideFiles::default()), 1, |_| None);
        assert!(lib.lib().is_null(), "an unreadable source: nothing touched");
        snap.failed_paths.clear();
        refresh_with(&mut lib, &snap, Some(&SideFiles::default()), 1, |_| None);
        assert_eq!(
            lib.lib()["scriptTags"]["/L/A/s/v.mp4"]["source"],
            "eroscripts-scout"
        );
        assert_eq!(
            lib.lib()["preferenceScores"]["/L/A/s/v.mp4"],
            1.5,
            "creampie 2, monster -0.5"
        );
        assert!(lib.config["categories"].as_array().unwrap().len() > 10);
        // After an edit (no side files): an empty playlist the user just made stays.
        lib.config["playlists"] = json!([{"id": "e", "name": "Mine", "videoPaths": []}]);
        refresh_with(&mut lib, &snap, None, 2, |_| None);
        assert_eq!(lib.config["playlists"][0]["name"], "Mine");
        refresh_with(&mut lib, &snap, Some(&SideFiles::default()), 3, |_| None);
        assert!(
            lib.config["playlists"].as_array().unwrap().is_empty(),
            "the scan's cleanup removes it"
        );
    }

    #[test]
    fn ids_are_uuid_shaped_and_distinct() {
        let (a, b) = (new_id(), new_id());
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(a.matches('-').count(), 4);
    }
}
