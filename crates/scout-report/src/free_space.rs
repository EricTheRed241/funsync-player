//! The swipe deck's Free space tab (`stack=free`): library videos to delete when a drive fills
//! up, the most GB per preference-score point first. Videos come from the running app's backend
//! (`GET /api/remote/videos`, `/api/remote/artists`, `/api/media/axis-flags`, at the port in
//! `config.json`), their sizes from the files.
//!
//! Never offered: favourites, videos rated `ratings.topRatedMin` (4) or more, a favourite
//! artist's videos, hand-made multi-axis scenes (and videos whose axes the backend hasn't
//! checked yet), videos already on their way out (rated `ratings.deleteAtOrBelow` or lower),
//! and files not inside a library source. Both thresholds come from
//! `renderer/js/content-preferences.json`.
//!
//! A swipe only marks: left = delete, right = keep (hidden from the tab), in
//! `free-space-swipes.json` by the backend's video id. `POST /api/free-space/delete` then rates
//! each marked video `deleteAtOrBelow` through the backend, and the desktop app deletes it the
//! way triage does (scripts, axes, sidecar, empty folders; the scouts never fetch it again).

use crate::swipes::{Dir, Store, Swipes};
use age_screen::backend::{Backend, Error as BackendError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The cards' `source`, and the `exclude` prefix (`library:<id>`).
pub const SOURCE: &str = "library";

/// Marks by backend video id (a hash of the path).
pub const SWIPES: Store = Store {
    file: "free-space-swipes.json",
    backup: "free-space-swipes.json.bak",
};

/// A rating sent while the app restarts waits this long for it, not age-screen's minutes:
/// someone is waiting on the page.
const WRITE_WAIT: Duration = Duration::from_secs(10);
const WRITE_RETRY_EVERY: Duration = Duration::from_secs(2);

const SHARED: &str = include_str!("../../../renderer/js/content-preferences.json");

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ratings {
    /// Rated this or lower (above 0): the app deletes the video.
    pub delete_at_or_below: f64,
    /// Rated this or more: kept.
    pub top_rated_min: f64,
}

/// The embedded `ratings` thresholds.
pub fn ratings() -> Ratings {
    #[derive(Deserialize)]
    struct Shared {
        ratings: Ratings,
    }
    serde_json::from_str::<Shared>(SHARED)
        .expect("renderer/js/content-preferences.json: ratings")
        .ratings
}

/// What the backend says about the library, one request each.
#[derive(Debug, Default)]
pub struct Library {
    /// `/api/remote/videos`' `videos`.
    pub videos: Vec<Value>,
    /// Ids of favourite artists' videos (`/api/remote/artists`, aliases applied there).
    pub favourite_artist_ids: HashSet<String>,
    /// Path → `handmade` / `generated` / `single` (`/api/media/axis-flags`; a video still being
    /// checked is missing).
    pub axes: HashMap<String, String>,
}

impl Library {
    pub fn fetch(backend: &Backend) -> Result<Library, BackendError> {
        let videos = backend.get("/api/remote/videos")?;
        let artists = backend.get("/api/remote/artists")?;
        let flags = backend.get("/api/media/axis-flags")?;
        Ok(Library::from_json(&videos, &artists, &flags))
    }

    pub fn from_json(videos: &Value, artists: &Value, flags: &Value) -> Library {
        let videos = videos
            .get("videos")
            .unwrap_or(videos)
            .as_array()
            .cloned()
            .unwrap_or_default();
        let favourite_artist_ids = artists
            .get("artists")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|a| a.get("favourite").and_then(Value::as_bool) == Some(true))
            .flat_map(|a| {
                a.get("videoIds")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter_map(id_text)
            .collect();
        let axes = flags
            .get("flags")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(path, kind)| Some((path.clone(), kind.as_str()?.to_string())))
            .collect();
        Library {
            videos,
            favourite_artist_ids,
            axes,
        }
    }
}

fn id_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Why a video is never offered (None = it may be).
pub fn protection(v: &Value, lib: &Library, r: Ratings) -> Option<&'static str> {
    let id = v.get("id").and_then(id_text).unwrap_or_default();
    let path = v.get("path").and_then(Value::as_str).unwrap_or("");
    let rating = v.get("rating").and_then(Value::as_f64).unwrap_or(0.0);
    if v.get("favorite").and_then(Value::as_bool) == Some(true) {
        Some("favourite")
    } else if rating >= r.top_rated_min {
        Some("top rated")
    } else if rating > 0.0 && rating <= r.delete_at_or_below {
        Some("being deleted")
    } else if lib.favourite_artist_ids.contains(&id) {
        Some("favourite artist")
    } else {
        match lib.axes.get(path).map(String::as_str) {
            Some("handmade") => Some("hand-made axes"),
            None => Some("axes not checked yet"),
            Some(_) => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeCard {
    /// Always SOURCE.
    pub source: &'static str,
    pub id: String,
    pub title: String,
    /// The artist folder (first folder under the library source).
    pub author: Option<String>,
    pub path: String,
    /// The video file's size in bytes.
    pub size: u64,
    pub score: Option<f64>,
    /// 0 = unrated.
    pub rating: f64,
    pub vr: bool,
    pub runtime_ms: Option<u64>,
    /// The backend's thumbnail path (`/api/media/thumb/<id>?v=…`); the page puts the backend's
    /// origin in front.
    pub thumb: Option<String>,
    /// The desktop's categories.
    pub tags: Vec<String>,
    /// File date, ms since 1970 (the scan's `dateAdded`).
    pub added_ms: Option<u64>,
    /// What deleting it buys: GB per score point (a score below 1 counts as 1).
    pub gb_per_point: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drive {
    pub mount: String,
    /// Bytes.
    pub free: u64,
    pub total: u64,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeDeck {
    /// Videos left to swipe (all of them, not just the ones returned).
    pub remaining: usize,
    /// Videos marked for deletion, still in the library and not protected since.
    pub wanted: usize,
    pub marked_bytes: u64,
    /// Videos never offered, by reason.
    pub protected: HashMap<&'static str, usize>,
    /// The backend's port, for thumbnails.
    pub backend_port: Option<u16>,
    /// The drives holding library sources.
    pub drives: Vec<Drive>,
    pub problems: Vec<String>,
    pub cards: Vec<FreeCard>,
}

/// The tab from the backend's answers: offered cards most GB per score point first (ties:
/// larger first), marks counted. `size_of` gives a file's size (None: not on disk), `sources`
/// the library sources (a video elsewhere is never offered).
pub fn build(
    lib: &Library,
    marks: &Swipes,
    r: Ratings,
    sources: &[PathBuf],
    size_of: impl Fn(&str) -> Option<u64>,
) -> FreeDeck {
    let mut deck = FreeDeck::default();
    for v in &lib.videos {
        let (Some(id), Some(path)) = (
            v.get("id").and_then(id_text),
            v.get("path").and_then(Value::as_str),
        ) else {
            continue;
        };
        let Some(source) = sources.iter().find(|s| Path::new(path).starts_with(s)) else {
            continue;
        };
        if let Some(why) = protection(v, lib, r) {
            *deck.protected.entry(why).or_insert(0) += 1;
            continue;
        }
        let Some(size) = size_of(path) else { continue };
        match marks.dir(&id) {
            Some(Dir::Left) => {
                deck.wanted += 1;
                deck.marked_bytes += size;
                continue;
            }
            Some(Dir::Right) => continue,
            None => {}
        }
        deck.cards.push(card(v, id, path, source, size));
    }
    deck.remaining = deck.cards.len();
    deck.cards.sort_by(|a, b| {
        b.gb_per_point
            .total_cmp(&a.gb_per_point)
            .then(b.size.cmp(&a.size))
            .then_with(|| a.id.cmp(&b.id))
    });
    deck
}

fn card(v: &Value, id: String, path: &str, source: &Path, size: u64) -> FreeCard {
    let p = Path::new(path);
    let score = v.get("score").and_then(Value::as_f64);
    let title = v
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| Some(p.file_stem()?.to_string_lossy().into_owned()))
        .unwrap_or_else(|| id.clone());
    let author = p.strip_prefix(source).ok().and_then(|rest| {
        let mut parts = rest.components();
        let first = parts.next()?;
        parts
            .next()
            .map(|_| first.as_os_str().to_string_lossy().into_owned())
    });
    let duration = v.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
    FreeCard {
        source: SOURCE,
        title,
        author,
        path: path.to_string(),
        size,
        score,
        rating: v.get("rating").and_then(Value::as_f64).unwrap_or(0.0),
        vr: v.get("isVR").and_then(Value::as_bool).unwrap_or(false),
        runtime_ms: (duration > 0.0).then(|| (duration * 1000.0).round() as u64),
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
        added_ms: v
            .get("dateAdded")
            .and_then(Value::as_u64)
            .filter(|&n| n > 0),
        gb_per_point: size as f64 / 1e9 / score.unwrap_or(1.0).max(1.0),
        id,
    }
}

/// The app's sources and backend from `config.json`. Without a readable one the backend isn't
/// asked (it couldn't be the app's).
fn app(user_data: &Path) -> Result<(Vec<PathBuf>, Backend, u16), String> {
    let config = video_tags::config::read(user_data)?;
    let port = config.backend_port;
    let backend = Backend::new(&format!("http://127.0.0.1:{port}"))?
        .with_write_wait(WRITE_WAIT, WRITE_RETRY_EVERY);
    Ok((config.sources, backend, port))
}

fn backend_problem(e: BackendError) -> String {
    match e {
        BackendError::Down(m) => format!("FunSync isn't answering, start the app: {m}"),
        BackendError::Bad(m) => m,
    }
}

fn file_size(path: &str) -> Option<u64> {
    std::fs::metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len())
}

/// `GET /api/deck?stack=free`: the next `limit` cards not in `exclude` (`library:<id>`).
pub fn load(user_data: &Path, limit: usize, exclude: &HashSet<String>) -> FreeDeck {
    let mut problems = Vec::new();
    let (sources, backend, port) = match app(user_data) {
        Ok(a) => a,
        Err(e) => {
            return FreeDeck {
                problems: vec![e],
                ..FreeDeck::default()
            };
        }
    };
    let marks = SWIPES.read(user_data).unwrap_or_else(|e| {
        problems.push(e);
        Swipes::default()
    });
    let mut deck = match Library::fetch(&backend) {
        Ok(lib) => build(&lib, &marks, ratings(), &sources, file_size),
        Err(e) => {
            problems.push(backend_problem(e));
            FreeDeck::default()
        }
    };
    deck.cards
        .retain(|c| !exclude.contains(&format!("{SOURCE}:{}", c.id)));
    deck.cards.truncate(limit);
    deck.backend_port = Some(port);
    deck.drives = drives(&sources);
    deck.problems.extend(problems);
    deck
}

/// The drives holding `sources` (`df -Pk`), each once; sources not mounted are left out.
pub fn drives(sources: &[PathBuf]) -> Vec<Drive> {
    let existing: Vec<&PathBuf> = sources.iter().filter(|s| s.is_dir()).collect();
    if existing.is_empty() {
        return Vec::new();
    }
    Command::new("df")
        .arg("-Pk")
        .args(existing)
        .output()
        .map(|o| parse_df(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// `df -P -k` lines (after the header): blocks, used, available in KiB, the mount point last
/// (it may hold spaces). One entry per mount point.
pub fn parse_df(out: &str) -> Vec<Drive> {
    let mut drives: Vec<Drive> = Vec::new();
    for line in out.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 6 {
            continue;
        }
        let (Ok(total), Ok(free)) = (fields[1].parse::<u64>(), fields[3].parse::<u64>()) else {
            continue;
        };
        let mount = fields[5..].join(" ");
        if drives.iter().any(|d| d.mount == mount) {
            continue;
        }
        drives.push(Drive {
            mount,
            free: free * 1024,
            total: total * 1024,
        });
    }
    drives
}

/// `POST /api/free-space/swipe {id, dir}`: mark a video the backend lists.
pub fn swipe(user_data: &Path, id: &str, dir: Dir) -> Result<Value, (u16, String)> {
    let (_, backend, _) = app(user_data).map_err(|e| (500, e))?;
    let lib = Library::fetch(&backend).map_err(|e| (503, backend_problem(e)))?;
    if !lib
        .videos
        .iter()
        .any(|v| v.get("id").and_then(id_text).as_deref() == Some(id))
    {
        return Err((400, format!("{id} is not in the library")));
    }
    let s = SWIPES
        .set(user_data, id, dir, video_tags::timefmt::now_iso())
        .map_err(|e| (500, e))?;
    Ok(json!({"id": id, "dir": s.dir, "at": s.at}))
}

/// What a delete does with each mark: rate these ids, drop these marks (videos gone from the
/// library), keep the rest (protected since they were marked, or not on disk).
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// (id, bytes)
    pub delete: Vec<(String, u64)>,
    pub stale: Vec<String>,
    pub skipped: Vec<(String, &'static str)>,
}

pub fn plan(
    lib: &Library,
    marks: &Swipes,
    r: Ratings,
    sources: &[PathBuf],
    size_of: impl Fn(&str) -> Option<u64>,
) -> Plan {
    let by_id: HashMap<String, &Value> = lib
        .videos
        .iter()
        .filter_map(|v| Some((v.get("id").and_then(id_text)?, v)))
        .collect();
    let mut p = Plan::default();
    for (id, s) in &marks.swipes {
        if s.dir != Dir::Left {
            continue;
        }
        let Some(v) = by_id.get(id) else {
            p.stale.push(id.clone());
            continue;
        };
        let path = v.get("path").and_then(Value::as_str).unwrap_or("");
        if !sources.iter().any(|s| Path::new(path).starts_with(s)) {
            p.skipped.push((id.clone(), "not in a library source"));
        } else if let Some(why) = protection(v, lib, r) {
            p.skipped.push((id.clone(), why));
        } else if let Some(size) = size_of(path) {
            p.delete.push((id.clone(), size));
        } else {
            p.skipped.push((id.clone(), "not on disk"));
        }
    }
    p
}

/// `POST /api/free-space/delete`: rate every marked video `deleteAtOrBelow` (the app deletes
/// it); its mark goes once the backend took the rating, and so do marks of videos no longer
/// in the library.
pub fn delete(user_data: &Path) -> Result<Value, (u16, String)> {
    let (sources, backend, _) = app(user_data).map_err(|e| (500, e))?;
    let marks = SWIPES.read(user_data).map_err(|e| (500, e))?;
    let lib = Library::fetch(&backend).map_err(|e| (503, backend_problem(e)))?;
    let r = ratings();
    let p = plan(&lib, &marks, r, &sources, file_size);
    let mut deleted = 0usize;
    let mut bytes = 0u64;
    let mut errors = Vec::new();
    for id in &p.stale {
        SWIPES.undo(user_data, id).map_err(|e| (500, e))?;
    }
    for (id, size) in &p.delete {
        match backend.rate(&Value::String(id.clone()), r.delete_at_or_below) {
            Ok(()) => {
                deleted += 1;
                bytes += size;
                SWIPES.undo(user_data, id).map_err(|e| (500, e))?;
            }
            Err(e) => errors.push(format!("{id}: {}", backend_problem(e))),
        }
    }
    Ok(json!({
        "deleted": deleted,
        "bytes": bytes,
        "skipped": p.skipped.iter().map(|(id, why)| json!({"id": id, "why": why})).collect::<Vec<_>>(),
        "errors": errors,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swipes::Swipe;

    const R: Ratings = Ratings {
        delete_at_or_below: 2.0,
        top_rated_min: 4.0,
    };

    fn video(id: &str, path: &str, score: f64) -> Value {
        json!({"id": id, "path": path, "score": score, "rating": 0, "favorite": false,
               "isVR": true, "duration": 600, "thumbUrl": format!("/api/media/thumb/{id}?v=1"),
               "categories": ["VR"], "dateAdded": 1_700_000_000_000u64})
    }

    fn lib(videos: Vec<Value>) -> Library {
        let axes = videos
            .iter()
            .map(|v| {
                (
                    v["path"].as_str().unwrap().to_string(),
                    "generated".to_string(),
                )
            })
            .collect();
        Library {
            videos,
            favourite_artist_ids: HashSet::new(),
            axes,
        }
    }

    fn marks(list: &[(&str, Dir)]) -> Swipes {
        let mut s = Swipes::default();
        for (id, dir) in list {
            s.swipes.insert(
                id.to_string(),
                Swipe {
                    dir: *dir,
                    at: "2026-11-18T00:00:00.000Z".into(),
                },
            );
        }
        s
    }

    fn sources() -> Vec<PathBuf> {
        vec![
            PathBuf::from("/lib/EroScripts"),
            PathBuf::from("/lib/Other"),
        ]
    }

    /// Size from the file name's last number, in GB: `/lib/…/a-8.mp4` is 8 GB; none = not on disk.
    fn size(path: &str) -> Option<u64> {
        let stem = Path::new(path).file_stem()?.to_str()?;
        Some(stem.rsplit('-').next()?.parse::<u64>().ok()? * 1_000_000_000)
    }

    #[test]
    fn most_gb_per_score_point_first() {
        let l = lib(vec![
            video("a", "/lib/EroScripts/Art/S/a-8.mp4", 20.0), // 0.4 GB/pt
            video("b", "/lib/EroScripts/Art/S/b-2.mp4", 2.0),  // 1
            video("c", "/lib/Other/c-3.mp4", -2.0),            // 3 (a score below 1 counts as 1)
            video("d", "/lib/EroScripts/Art/S/d-10.mp4", 10.0), // 1, larger than b
        ]);
        let deck = build(&l, &Swipes::default(), R, &sources(), size);
        let ids: Vec<&str> = deck.cards.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["c", "d", "b", "a"]);
        assert_eq!(deck.remaining, 4);
        assert_eq!(deck.cards[1].author.as_deref(), Some("Art"));
        // A video right in the source folder has no artist folder.
        assert_eq!(deck.cards[0].author, None);
    }

    #[test]
    fn never_offers_protected_videos() {
        let mut fav = video("fav", "/lib/Other/fav-9.mp4", 0.0);
        fav["favorite"] = json!(true);
        let mut top = video("top", "/lib/Other/top-9.mp4", 0.0);
        top["rating"] = json!(4);
        let mut doomed = video("doomed", "/lib/Other/doomed-9.mp4", 0.0);
        doomed["rating"] = json!(2);
        let mut ok = video("ok", "/lib/Other/ok-1.mp4", 3.5);
        ok["rating"] = json!(3.5);
        let mut l = lib(vec![
            fav,
            top,
            doomed,
            video("artist", "/lib/Other/artist-9.mp4", 0.0),
            video("hand", "/lib/Other/hand-9.mp4", 0.0),
            video("pending", "/lib/Other/pending-9.mp4", 0.0),
            video("elsewhere", "/Volumes/X/elsewhere-9.mp4", 0.0),
            video("gone", "/lib/Other/gone.mp4", 0.0),
            ok,
        ]);
        l.favourite_artist_ids.insert("artist".into());
        l.axes
            .insert("/lib/Other/hand-9.mp4".into(), "handmade".into());
        l.axes.remove("/lib/Other/pending-9.mp4");
        let deck = build(&l, &Swipes::default(), R, &sources(), size);
        let ids: Vec<&str> = deck.cards.iter().map(|c| c.id.as_str()).collect();
        // Watched at 3.5★ (below top rated) may go.
        assert_eq!(ids, ["ok"]);
        for why in [
            "favourite",
            "top rated",
            "being deleted",
            "favourite artist",
            "hand-made axes",
            "axes not checked yet",
        ] {
            assert_eq!(deck.protected.get(why), Some(&1), "{why}");
        }
    }

    #[test]
    fn marks_leave_the_tab_and_count_the_left_ones() {
        let l = lib(vec![
            video("a", "/lib/Other/a-8.mp4", 1.0),
            video("b", "/lib/Other/b-2.mp4", 1.0),
            video("c", "/lib/Other/c-3.mp4", 1.0),
        ]);
        let deck = build(
            &l,
            &marks(&[("a", Dir::Left), ("b", Dir::Right), ("zz", Dir::Left)]),
            R,
            &sources(),
            size,
        );
        assert_eq!(deck.cards.len(), 1);
        assert_eq!(deck.remaining, 1);
        assert_eq!((deck.wanted, deck.marked_bytes), (1, 8_000_000_000));
    }

    #[test]
    fn delete_plan_rechecks_protection_and_drops_stale_marks() {
        let mut since_fav = video("fav", "/lib/Other/fav-5.mp4", 1.0);
        since_fav["favorite"] = json!(true);
        let l = lib(vec![
            video("a", "/lib/Other/a-8.mp4", 1.0),
            video("keep", "/lib/Other/keep-8.mp4", 1.0),
            video("missing", "/lib/Other/missing.mp4", 1.0),
            since_fav,
        ]);
        let p = plan(
            &l,
            &marks(&[
                ("a", Dir::Left),
                ("keep", Dir::Right),
                ("missing", Dir::Left),
                ("fav", Dir::Left),
                ("gone", Dir::Left),
            ]),
            R,
            &sources(),
            size,
        );
        assert_eq!(p.delete, vec![("a".to_string(), 8_000_000_000)]);
        assert_eq!(p.stale, vec!["gone".to_string()]);
        assert_eq!(
            p.skipped,
            vec![
                ("fav".to_string(), "favourite"),
                ("missing".to_string(), "not on disk")
            ]
        );
    }

    #[test]
    fn reads_df_with_spaces_in_mount_points_once_each() {
        let out = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
                   /dev/disk4s3 1952544896 1934000256 18544640 100% /Volumes/2TB-AGENTS\n\
                   /dev/disk4s3 1952544896 1934000256 18544640 100% /Volumes/2TB-AGENTS\n\
                   /dev/disk6s1 468000000 207000000 261000000 45% /Volumes/Extra Downloads\n";
        let d = parse_df(out);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].free, 18_544_640 * 1024);
        assert_eq!(d[1].mount, "/Volumes/Extra Downloads");
    }

    #[test]
    fn library_from_backend_json() {
        let l = Library::from_json(
            &json!({"videos": [video("a", "/lib/Other/a-1.mp4", 1.0)]}),
            &json!({"artists": [{"favourite": true, "videoIds": ["a", "b"]},
                                {"favourite": false, "videoIds": ["c"]}]}),
            &json!({"flags": {"/lib/Other/a-1.mp4": "single"}, "pending": 0}),
        );
        assert_eq!(l.videos.len(), 1);
        assert_eq!(
            l.favourite_artist_ids,
            HashSet::from(["a".to_string(), "b".to_string()])
        );
        assert_eq!(l.axes["/lib/Other/a-1.mp4"], "single");
    }
}
