//! Upkeep of the per-video maps after a complete scan, as the old app's
//! library does: entries of a video that MOVED follow it
//! (`renderer/js/association-rehome.js`: only an unambiguous 1:1 file-name
//! match, never robbing a live video, never touching an offline source),
//! and an association whose video is provably gone is dropped
//! (`renderer/js/association-dead.js`: only when a folder above it answers
//! and the OS says the video and its folder don't exist; an unplugged drive
//! never qualifies).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::data::Library;
use crate::scan::Video;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathState {
    Present,
    Absent,
    Unknown,
}

/// Only ENOENT / ENOTDIR prove a path isn't there; any other error says
/// nothing (`electron/path-state.js`).
pub fn path_state(p: &str) -> PathState {
    match std::fs::symlink_metadata(p) {
        Ok(_) => PathState::Present,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => PathState::Absent,
        Err(e) if e.raw_os_error() == Some(20) => PathState::Absent, // ENOTDIR
        Err(_) => PathState::Unknown,
    }
}

/// `splitVolumePath`: (lead, segments, segments that make up the volume root).
fn split_volume(p: &str) -> Option<(&'static str, Vec<&str>, usize)> {
    let rest = p.strip_prefix('/')?;
    let parts: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    let depth = match parts.as_slice() {
        ["Volumes" | "mnt", ..] => 2,
        ["media" | "net", ..] => 3,
        ["run", "media", ..] => 4,
        _ => 1,
    };
    Some(("/", parts, depth))
}

/// `isVideoLocationGone`: a folder between the video and its volume root is
/// present, and the video and its own folder are absent.
pub fn is_location_gone(video: &str, state: impl Fn(&str) -> PathState) -> bool {
    let Some((lead, parts, root)) = split_volume(video) else {
        return false;
    };
    if parts.len() < root + 3 {
        return false;
    }
    let at = |n: usize| format!("{lead}{}", parts[..n].join("/"));
    let answers = (root + 1..=parts.len() - 2)
        .rev()
        .any(|n| state(&at(n)) == PathState::Present);
    answers && state(video) == PathState::Absent && state(&at(parts.len() - 1)) == PathState::Absent
}

fn base_name(p: &str) -> String {
    p.rsplit(['/', '\\']).next().unwrap_or("").to_lowercase()
}

/// `planAssociationRehome`: `(from, to)` for each unambiguous move.
pub fn plan_rehome(
    stored: &[String],
    scanned: &[String],
    unavailable: impl Fn(&str) -> bool,
) -> Vec<(String, String)> {
    let scanned_set: HashSet<&str> = scanned.iter().map(String::as_str).collect();
    let stored_set: HashSet<&str> = stored.iter().map(String::as_str).collect();
    let mut orphans: HashMap<String, Vec<&str>> = HashMap::new();
    for p in stored {
        if !scanned_set.contains(p.as_str()) && !unavailable(p) && !base_name(p).is_empty() {
            orphans.entry(base_name(p)).or_default().push(p);
        }
    }
    let mut needy: HashMap<String, Vec<&str>> = HashMap::new();
    for p in scanned {
        if !stored_set.contains(p.as_str()) && !base_name(p).is_empty() {
            needy.entry(base_name(p)).or_default().push(p);
        }
    }
    let mut moves: Vec<(String, String)> = orphans
        .iter()
        .filter_map(
            |(name, from)| match (from.as_slice(), needy.get(name).map(Vec::as_slice)) {
                ([from], Some([to])) => Some((from.to_string(), to.to_string())),
                _ => None,
            },
        )
        .collect();
    moves.sort();
    moves
}

/// After a complete scan: move `associations` and `ratings` entries of moved
/// videos, then drop associations whose video is provably gone. Paths under
/// a disabled or missing source are left alone. True when anything changed.
pub fn upkeep(lib: &mut Library, videos: &[Video], state: impl Fn(&str) -> PathState) -> bool {
    let offline: Vec<String> = lib
        .sources()
        .into_iter()
        .filter(|s| !s.enabled || !Path::new(&s.path).is_dir())
        .map(|s| format!("{}/", s.path.trim_end_matches('/')))
        .collect();
    let unavailable = |p: &str| offline.iter().any(|o| p.starts_with(o.as_str()));
    let scanned: Vec<String> = videos.iter().map(|v| v.path.clone()).collect();
    let mut changed = false;
    for key in ["associations", "ratings"] {
        let Some(map) = lib.lib_mut(key).as_object_mut() else {
            continue;
        };
        let stored: Vec<String> = map.keys().cloned().collect();
        for (from, to) in plan_rehome(&stored, &scanned, unavailable) {
            if let Some(v) = map.remove(&from) {
                map.insert(to, v);
                changed = true;
            }
        }
    }
    let live: HashSet<&str> = scanned.iter().map(String::as_str).collect();
    if let Some(map) = lib.lib_mut("associations").as_object_mut() {
        let gone: Vec<String> = map
            .keys()
            .filter(|p| {
                !live.contains(p.as_str()) && !unavailable(p) && is_location_gone(p, &state)
            })
            .cloned()
            .collect();
        for p in gone {
            map.remove(&p);
            changed = true;
        }
        for (video, e) in map.iter_mut() {
            if !unavailable(video) {
                changed |= heal_scripts(video, e, &state);
            }
        }
    }
    changed
}

/// The renderer's per-slot recovery: a `single`, `multi.main` or custom
/// main-route script that is absent (the OS said so; an unreachable one is
/// left alone) is re-pointed at the file of the same name next to the
/// video, when that one is present. Nothing is pruned. True when healed.
pub fn heal_scripts(
    video: &str,
    e: &mut serde_json::Value,
    state: impl Fn(&str) -> PathState,
) -> bool {
    let Some(dir) = Path::new(video).parent() else {
        return false;
    };
    let fix = |slot: &mut serde_json::Value| -> bool {
        let Some(p) = slot.as_str() else {
            return false;
        };
        let name = p.rsplit(['/', '\\']).next().unwrap_or("");
        let cand = dir.join(name).to_string_lossy().into_owned();
        if name.is_empty()
            || cand == p
            || state(p) != PathState::Absent
            || state(&cand) != PathState::Present
        {
            return false;
        }
        *slot = serde_json::json!(cand);
        true
    };
    // `get_mut`, not `[]`: indexing a missing slot would create it.
    let mut healed = e.get_mut("single").is_some_and(&fix);
    if let Some(main) = e.get_mut("multi").and_then(|m| m.get_mut("main")) {
        healed |= fix(main);
    }
    if let Some(main) = e
        .get_mut("custom")
        .and_then(|c| c.get_mut("routes"))
        .and_then(|r| r.as_array_mut())
        .and_then(|r| r.iter_mut().find(|r| r["role"] == "main"))
        .and_then(|m| m.get_mut("scriptPath"))
    {
        healed |= fix(main);
    }
    if healed {
        // Keep the mirror fields in step (`buildAssociationEntry`).
        let old = e.clone();
        *e = crate::associate::entry(
            &old,
            old["active"].as_str(),
            old["single"].as_str(),
            Some(old["multi"].clone()),
        );
    }
    healed
}

#[cfg(test)]
mod heal_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scripts_found_next_to_the_video() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().to_string_lossy().into_owned();
        std::fs::write(dir.path().join("a.funscript"), b"{}").unwrap();
        let video = format!("{d}/a.mp4");
        let mut e = json!({"active": "multi", "single": "/gone/x/a.funscript",
            "multi": {"main": "/gone/x/a.funscript", "axes": {}}, "custom": null});
        assert!(heal_scripts(&video, &mut e, path_state));
        let want = format!("{d}/a.funscript");
        assert_eq!(
            (e["single"].as_str(), e["multi"]["main"].as_str()),
            (Some(&*want), Some(&*want))
        );
        assert_eq!(e["main"].as_str(), Some(&*want), "mirror field follows");
        // Nothing to find, or an unreachable script: left alone.
        let mut e = json!({"active": "single", "single": "/gone/x/b.funscript"});
        assert!(!heal_scripts(&video, &mut e, path_state));
        let mut e = json!({"active": "single", "single": "/gone/x/a.funscript"});
        assert!(!heal_scripts(&video, &mut e, |_: &str| PathState::Unknown));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn rehome_only_unambiguous_moves() {
        let stored = s(&[
            "/old/a.mp4",
            "/old/intro.mp4",
            "/x/intro.mp4",
            "/live/b.mp4",
            "/off/c.mp4",
        ]);
        let scanned = s(&[
            "/new/a.mp4",
            "/new/intro.mp4",
            "/live/b.mp4",
            "/other/b.mp4",
            "/new/c.mp4",
        ]);
        let moves = plan_rehome(&stored, &scanned, |p| p.starts_with("/off/"));
        // intro: two orphans (ambiguous); b: still live; c: offline source.
        assert_eq!(
            moves,
            [("/old/a.mp4".to_string(), "/new/a.mp4".to_string())]
        );
    }

    #[test]
    fn gone_only_when_the_drive_answers() {
        let present = |set: &'static [&'static str]| {
            move |p: &str| {
                if set.contains(&p) {
                    PathState::Present
                } else {
                    PathState::Absent
                }
            }
        };
        let v = "/Volumes/Disk/Lib/Artist/Scene/a.mp4";
        assert!(is_location_gone(v, present(&["/Volumes/Disk/Lib"])));
        // Unplugged drive: nothing above answers.
        assert!(!is_location_gone(v, present(&[])));
        // Only the volume root answers (a stale mount point): not enough.
        assert!(!is_location_gone(v, present(&["/Volumes/Disk"])));
        // The folder still exists: not gone.
        assert!(!is_location_gone(
            v,
            present(&["/Volumes/Disk/Lib", "/Volumes/Disk/Lib/Artist/Scene"])
        ));
        // Any doubt keeps it.
        assert!(!is_location_gone(
            v,
            |p: &str| if p == "/Volumes/Disk/Lib" {
                PathState::Present
            } else {
                PathState::Unknown
            }
        ));
    }

    #[test]
    fn upkeep_on_disk() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("src");
        fs::create_dir_all(src.join("Artist/New")).unwrap();
        let src_s = src.to_string_lossy().to_string();
        let moved_to = format!("{src_s}/Artist/New/a.mp4");
        let gone = format!("{src_s}/Artist/Deleted/b.mp4");
        let offline = "/Volumes/Gone/Lib/Artist/Scene/c.mp4".to_string();
        let mut lib = Library::default();
        *lib.lib_mut("sources") = json!([
            {"name": "s", "path": src_s, "enabled": true},
            {"name": "o", "path": "/Volumes/Gone/Lib", "enabled": true}]);
        *lib.lib_mut("associations") = json!({
            format!("{src_s}/Artist/Old/a.mp4"): {"single": "x"}, gone.clone(): {"single": "y"}, offline.clone(): {"single": "z"}});
        *lib.lib_mut("ratings") = json!({format!("{src_s}/Artist/Old/a.mp4"): {"rating": 4}});
        let video: Video = serde_json::from_value(json!({
            "id": "1", "name": "a.mp4", "path": moved_to, "ext": ".mp4", "fuzzyMatched": false,
            "variants": [], "axes": [], "sourceName": "", "dateAdded": 0.0, "size": 0}))
        .unwrap();
        assert!(upkeep(&mut lib, &[video], path_state));
        let a = lib.lib()["associations"].as_object().unwrap();
        let mut keys: Vec<&String> = a.keys().collect();
        keys.sort();
        assert_eq!(keys, [&offline, &moved_to]);
        assert_eq!(lib.lib()["ratings"][&moved_to]["rating"], 4);
    }
}
