//! `library.scriptTags`, rebuilt on every scan like the renderer does:
//! the scan's sidecar posts (`mergeScoutPosts`), the `eroscripts:sync`
//! links (`mergeLinksIntoScriptTags`), then the video tags and pornstar
//! tags files (`combineTagFiles` + `mergeVideoTags`, `addedTags`
//! bookkeeping). The side files are the old app's, read-only.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde_json::{Map, Value, json};

use super::{RULES, obj_mut, strs};
use crate::data::Library;
use crate::scan::Video;

/// `source` of a record: the app's own download, the EroScripts scout's
/// sidecar, the FapTap scout's sidecar, a sync link, the video tags file.
const APP: &str = "eroscripts";
const SCOUT: &str = "eroscripts-scout";
pub(super) const FAPTAP: &str = "faptap-scout";
const SYNC: &str = "eroscripts-sync";
const VIDEO_TAGS: &str = "video-tags";

/// The old app's side files, as its main process reads them.
#[derive(Default)]
pub struct SideFiles {
    /// `eroscripts-links.json` `links`: empty when the file is missing (its
    /// sync records then go), none when unreadable or foreign (no change).
    pub links: Option<Map<String, Value>>,
    /// `video-tags.json`: version 1 with a `videos` map.
    pub video_tags: Option<Value>,
    /// `pornstar-tags.json`: version 1 with `pornstars` and `videos` maps.
    pub pornstar_tags: Option<Value>,
}

impl SideFiles {
    pub fn load(dir: Option<&Path>) -> SideFiles {
        let Some(dir) = dir else {
            return SideFiles::default();
        };
        let read = |name: &str| std::fs::read(dir.join(name));
        let parse = |bytes: Vec<u8>, keys: &[&str]| {
            serde_json::from_slice::<Value>(&bytes)
                .ok()
                .filter(|d| d["version"] == 1 && keys.iter().all(|k| d[*k].is_object()))
        };
        let links = match read("eroscripts-links.json") {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(Map::new()),
            Err(_) => None,
            Ok(b) => parse(b, &[]).map(|d| d["links"].as_object().cloned().unwrap_or_default()),
        };
        SideFiles {
            links,
            video_tags: read("video-tags.json")
                .ok()
                .and_then(|b| parse(b, &["videos"])),
            pornstar_tags: read("pornstar-tags.json")
                .ok()
                .and_then(|b| parse(b, &["pornstars", "videos"])),
        }
    }
}

pub(super) fn merge(lib: &mut Library, videos: &[Video], side: &SideFiles, now: u64) {
    let map = obj_mut(lib.lib_mut("scriptTags"));
    scout_posts(map, videos, now);
    if let Some(links) = &side.links {
        merge_links(map, links, now);
    }
    video_tags(map, side, now);
}

/// A record's own tags: `tags` without those the video tags file added.
fn base_tags(r: &Value) -> Vec<Value> {
    let tags = r["tags"].as_array().cloned().unwrap_or_default();
    match r["addedTags"].as_array() {
        Some(added) if !added.is_empty() => {
            tags.into_iter().filter(|t| !added.contains(t)).collect()
        }
        _ => tags,
    }
}

/// JS `value || null`.
fn or_null(v: &Value) -> Value {
    match v {
        Value::Bool(false) => Value::Null,
        Value::String(s) if s.is_empty() => Value::Null,
        Value::Number(n) if n.as_f64() == Some(0.0) => Value::Null,
        _ => v.clone(),
    }
}

fn source(r: Option<&Value>) -> Option<&str> {
    r.and_then(|r| r["source"].as_str())
}

/// The scan's `eroscripts.json` post replaces a sync, FapTap or video-tags
/// record (the app's own download wins); a `faptap.json` scene only fills
/// a gap or refreshes a FapTap record.
fn scout_posts(map: &mut Map<String, Value>, videos: &[Video], now: u64) {
    for v in videos {
        let existing = map.get(&v.path);
        let src = source(existing);
        let record = if let Some(post) = v.eroscripts.as_ref().filter(|p| !p.topic_id.is_null()) {
            let tags = json!(post.tags);
            if src == Some(APP)
                || existing.is_some_and(|e| {
                    src == Some(SCOUT)
                        && e["topicId"] == post.topic_id
                        && json!(base_tags(e)) == tags
                })
            {
                continue;
            }
            json!({"tags": tags, "topicId": post.topic_id, "topicUrl": or_null(&json!(post.url)),
                   "source": SCOUT, "savedAt": now})
        } else if let Some(scene) = v.faptap.as_ref().filter(|s| !s.faptap_id.is_empty()) {
            if existing.is_some() && src != Some(FAPTAP) && src != Some(VIDEO_TAGS) {
                continue;
            }
            let tags = json!(scene.tags);
            let url = or_null(&json!(scene.url));
            let preview = or_null(&json!(scene.thumbnail_url));
            if existing.is_some_and(|e| {
                src == Some(FAPTAP)
                    && e["faptapId"] == scene.faptap_id.as_str()
                    && or_null(&e["faptapUrl"]) == url
                    && or_null(&e["previewUrl"]) == preview
                    && json!(base_tags(e)) == tags
            }) {
                continue;
            }
            let mut r = json!({"tags": tags, "topicId": null, "topicUrl": null, "faptapId": scene.faptap_id,
                               "faptapUrl": url, "source": FAPTAP, "savedAt": now});
            if !preview.is_null() {
                r["previewUrl"] = preview;
            }
            r
        } else {
            continue;
        };
        map.insert(v.path.clone(), record);
    }
}

fn applied(link: &Value) -> bool {
    !link["topicId"].is_null()
        && matches!(link["confidence"].as_str(), Some("confirmed" | "probable"))
}

/// Confirmed and probable links replace sync, FapTap and video-tags
/// records; a sync record whose link is gone is removed.
fn merge_links(map: &mut Map<String, Value>, links: &Map<String, Value>, now: u64) {
    for (path, link) in links {
        if path.is_empty() || !applied(link) {
            continue;
        }
        let existing = map.get(path);
        if existing.is_some() && !matches!(source(existing), Some(SYNC | FAPTAP | VIDEO_TAGS)) {
            continue;
        }
        let tags: Vec<&Value> = link["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| t.as_str().is_some_and(|s| !s.trim().is_empty()))
            .collect();
        let mut next = json!({"tags": tags, "topicId": link["topicId"], "topicUrl": or_null(&link["topicUrl"]),
                              "source": SYNC, "savedAt": now, "confidence": link["confidence"],
                              "matchedAt": or_null(&link["matchedAt"])});
        if let Some(p) = link["previewUrl"].as_str().filter(|p| !p.is_empty()) {
            next["previewUrl"] = json!(p);
        }
        let same = |a: &Value| {
            ["topicId", "topicUrl", "confidence", "matchedAt"]
                .iter()
                .all(|k| a[*k] == next[*k])
                && or_null(&a["previewUrl"]) == next["previewUrl"]
                && json!(base_tags(a)) == next["tags"]
        };
        if !existing.is_some_and(same) {
            map.insert(path.clone(), next);
        }
    }
    map.retain(|path, r| r["source"] != SYNC || links.get(path).is_some_and(applied));
}

/// The video tags file's tags (VIDEO_TAGGER_TAGS) and the pornstar tags
/// file's applied performers, on top of each record's own tags; the ones
/// added are kept in `addedTags`.
fn video_tags(map: &mut Map<String, Value>, side: &SideFiles, now: u64) {
    if side.video_tags.is_none() && side.pornstar_tags.is_none() {
        return;
    }
    let tagger = strs(&RULES.mirror["videoTaggerTags"]);
    let mut file: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut add = |path: &str, tags: Vec<Value>| {
        if !tags.is_empty() {
            file.entry(path.to_string()).or_default().extend(tags);
        }
    };
    fn each(v: &Value) -> impl Iterator<Item = (&String, &Value)> {
        v.as_object().into_iter().flatten()
    }
    if let Some(vf) = &side.video_tags {
        for (p, e) in each(&vf["videos"]) {
            add(p, e["tags"].as_array().cloned().unwrap_or_default());
        }
    } else {
        let lower: HashSet<String> = tagger.iter().map(|t| t.to_lowercase()).collect();
        for (p, r) in map.iter() {
            let kept = r["addedTags"].as_array().into_iter().flatten();
            add(
                p,
                kept.filter(|t| t.as_str().is_some_and(|t| lower.contains(t)))
                    .cloned()
                    .collect(),
            );
        }
    }
    let mut allowed: HashSet<String> = tagger.iter().map(|t| t.to_lowercase()).collect();
    if let Some(pf) = &side.pornstar_tags {
        for (tag, p) in each(&pf["pornstars"]) {
            if p["applied"] == true {
                allowed.insert(tag.to_lowercase());
            }
        }
        for (p, tags) in each(&pf["videos"]) {
            add(p, tags.as_array().cloned().unwrap_or_default());
        }
    }

    let mut paths: Vec<String> = file.keys().cloned().collect();
    for (p, r) in map.iter() {
        let added = r["addedTags"].as_array().is_some_and(|a| !a.is_empty());
        if (r["source"] == VIDEO_TAGS || added) && !file.contains_key(p) {
            paths.push(p.clone());
        }
    }
    for path in paths.into_iter().filter(|p| !p.is_empty()) {
        let mut wanted: Vec<Value> = Vec::new();
        for t in file
            .get(&path)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let t = json!(t.trim().to_lowercase());
            if allowed.contains(t.as_str().unwrap()) && !wanted.contains(&t) {
                wanted.push(t);
            }
        }
        let Some(existing) = map.get(&path) else {
            if !wanted.is_empty() {
                map.insert(
                    path,
                    json!({"tags": wanted, "addedTags": wanted, "topicId": null, "topicUrl": null,
                                        "source": VIDEO_TAGS, "savedAt": now}),
                );
            }
            continue;
        };
        let base = base_tags(existing);
        let add: Vec<Value> = wanted.into_iter().filter(|t| !base.contains(t)).collect();
        if existing["source"] == VIDEO_TAGS && base.is_empty() && add.is_empty() {
            map.remove(&path);
            continue;
        }
        let tags: Vec<Value> = base.into_iter().chain(add.iter().cloned()).collect();
        let tags_now = existing["tags"].as_array().cloned().unwrap_or_default();
        let before = existing["addedTags"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if tags_now == tags
            && before == add
            && (!add.is_empty() || existing.get("addedTags").is_none())
        {
            continue;
        }
        let mut next = existing.clone();
        next["tags"] = json!(tags);
        if add.is_empty() {
            obj_mut(&mut next).remove("addedTags");
        } else {
            next["addedTags"] = json!(add);
        }
        map.insert(path, next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{EroscriptsRef, FaptapRef};

    fn video(path: &str) -> Video {
        serde_json::from_value(json!({"id": "x", "name": "n", "path": path, "ext": ".mp4",
            "funscriptPath": null, "fuzzyMatched": false, "subtitlePath": null, "variants": [],
            "axes": [], "sourceName": "s", "dateAdded": 0.0, "size": 0}))
        .unwrap()
    }

    #[test]
    fn scout_posts_respect_precedence() {
        let mut map = json!({
            "/a": {"tags": ["x"], "source": "eroscripts", "topicId": 1},
            "/b": {"tags": ["x"], "source": "eroscripts-sync", "topicId": 2},
            "/c": {"tags": ["old"], "source": "eroscripts-sync", "topicId": 3},
            "/d": {"tags": ["f"], "source": "faptap-scout", "faptapId": "9", "faptapUrl": null}
        });
        let map = map.as_object_mut().unwrap();
        let post = |id| EroscriptsRef {
            topic_id: json!(id),
            url: Some("u".into()),
            tags: vec!["new".into()],
            poster_url: None,
        };
        let scene = FaptapRef {
            faptap_id: "9".into(),
            url: None,
            tags: vec!["f".into()],
            thumbnail_url: None,
        };
        let mut vs: Vec<Video> = ["/a", "/b", "/c", "/d", "/e"]
            .iter()
            .map(|p| video(p))
            .collect();
        vs[0].eroscripts = Some(post(1));
        vs[1].eroscripts = Some(post(2));
        vs[2].faptap = Some(scene.clone());
        vs[3].faptap = Some(scene.clone());
        vs[4].faptap = Some(scene);
        scout_posts(map, &vs, 7);
        assert_eq!(
            map["/a"]["source"], "eroscripts",
            "the app's own download wins"
        );
        assert_eq!(map["/b"]["source"], SCOUT);
        assert_eq!(map["/b"]["tags"], json!(["new"]));
        assert_eq!(
            map["/c"]["source"], SYNC,
            "FapTap never replaces an EroScripts record"
        );
        assert_eq!(
            map["/d"].get("savedAt"),
            None,
            "unchanged FapTap record not rewritten"
        );
        assert_eq!(map["/e"]["source"], FAPTAP, "FapTap fills a gap");
    }

    #[test]
    fn links_add_update_and_withdraw_sync_records() {
        let mut map = json!({
            "/keep": {"tags": ["a"], "source": "eroscripts-scout", "topicId": 1},
            "/gone": {"tags": ["a"], "source": "eroscripts-sync", "topicId": 2},
            "/same": {"tags": ["a", "vr"], "addedTags": ["vr"], "source": "eroscripts-sync", "topicId": 3,
                      "topicUrl": "u", "confidence": "confirmed", "matchedAt": null, "savedAt": 1}
        });
        let links = json!({
            "/keep": {"topicId": 9, "confidence": "confirmed", "tags": ["b"]},
            "/new": {"topicId": 4, "confidence": "probable", "tags": ["b", " ", 5], "previewUrl": "p"},
            "/review": {"topicId": 5, "confidence": "possible", "tags": []},
            "/same": {"topicId": 3, "topicUrl": "u", "confidence": "confirmed", "tags": ["a"]}
        });
        let map = map.as_object_mut().unwrap();
        merge_links(map, links.as_object().unwrap(), 7);
        assert_eq!(map["/keep"]["topicId"], 1);
        assert!(!map.contains_key("/gone"));
        assert!(!map.contains_key("/review"));
        assert_eq!(map["/new"]["tags"], json!(["b"]));
        assert_eq!(map["/new"]["previewUrl"], "p");
        assert_eq!(map["/same"]["savedAt"], 1, "an added tag isn't a change");
    }

    #[test]
    fn video_and_pornstar_tags_are_added_on_top_and_withdrawn() {
        let mut map = json!({
            "/post": {"tags": ["riding", "x"], "source": "eroscripts-scout"},
            "/was": {"tags": ["x", "pov"], "addedTags": ["pov"], "source": "eroscripts-scout"},
            "/only": {"tags": ["vr"], "addedTags": ["vr"], "source": "video-tags"}
        });
        let side = SideFiles {
            links: None,
            video_tags: Some(json!({"version": 1, "videos": {
                "/post": {"tags": ["Riding", "creampie", "nope"]},
                "/fresh": {"tags": ["vr"]}
            }})),
            pornstar_tags: Some(json!({"version": 1,
                "pornstars": {"ann": {"applied": true}, "bea": {"applied": false}},
                "videos": {"/post": ["ann", "bea"]}})),
        };
        let map = map.as_object_mut().unwrap();
        video_tags(map, &side, 7);
        assert_eq!(
            map["/post"]["tags"],
            json!(["riding", "x", "creampie", "ann"])
        );
        assert_eq!(map["/post"]["addedTags"], json!(["creampie", "ann"]));
        assert_eq!(map["/was"]["tags"], json!(["x"]));
        assert!(map["/was"].get("addedTags").is_none());
        assert!(
            !map.contains_key("/only"),
            "a video-tags record left without tags goes"
        );
        assert_eq!(map["/fresh"]["source"], VIDEO_TAGS);

        // No video tags file: the tagger tags already added stay, pornstar tags go.
        let side = SideFiles {
            links: None,
            video_tags: None,
            pornstar_tags: Some(json!({"version": 1, "pornstars": {}, "videos": {}})),
        };
        video_tags(map, &side, 8);
        assert_eq!(map["/post"]["tags"], json!(["riding", "x", "creampie"]));
    }

    #[test]
    fn side_files_load_like_the_old_app() {
        let d = tempfile::tempdir().unwrap();
        let s = SideFiles::load(Some(d.path()));
        assert_eq!(s.links, Some(Map::new()), "missing links file = no links");
        assert!(s.video_tags.is_none());
        std::fs::write(
            d.path().join("eroscripts-links.json"),
            r#"{"version":2,"links":{}}"#,
        )
        .unwrap();
        std::fs::write(
            d.path().join("video-tags.json"),
            r#"{"version":1,"videos":[]}"#,
        )
        .unwrap();
        std::fs::write(
            d.path().join("pornstar-tags.json"),
            r#"{"version":1,"pornstars":{},"videos":{}}"#,
        )
        .unwrap();
        let s = SideFiles::load(Some(d.path()));
        assert!(s.links.is_none(), "foreign version: no change");
        assert!(s.video_tags.is_none(), "videos must be a map");
        assert!(s.pornstar_tags.is_some());
    }
}
