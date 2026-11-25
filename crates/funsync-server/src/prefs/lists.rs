//! Categories, playlists and collections the app keeps up to date, each with
//! the renderer's bookkeeping (made once, a video added once, so the user's
//! deletions and removals stick):
//!
//! - auto categories from post tags, VR and the axis kind
//!   (`library.autoCategories`: `{created, assigned}`);
//! - artist playlists and "Favourite artists" (`library.artistPlaylists`:
//!   `{created, added}`), then the playlist cleanup;
//! - auto collections (`library.autoCollections`: `{created, added}`) and the
//!   score lists (same `created`), which hold exactly their band;
//! - smart playlists (`library.smartPlaylists`: `{created}`), which hold
//!   exactly their members.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value, json};

use super::{
    Ctx, RULES, arr_mut, artist_of, axis_kind, fold, list_title, locale_cmp, name_key, new_id,
    obj_mut, strs, tags::FAPTAP,
};
use crate::data::Library;
use crate::remote::rating_entry;

const FAVOURITES_KEY: &str = ":favourites";

fn defs(key: &str) -> &'static Vec<Value> {
    RULES.mirror[key].as_array().unwrap()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// Whether `list` (a bookkeeping map's entry) holds `key`.
fn has(map: &Map<String, Value>, path: &str, key: &str) -> bool {
    map.get(path)
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|k| k == key))
}

fn push_key(map: &mut Map<String, Value>, path: &str, key: &str) {
    arr_mut(map.entry(path).or_insert(json!([]))).push(json!(key));
}

fn add_created(created: &mut Vec<String>, key: &str) {
    if !created.iter().any(|k| k == key) {
        created.push(key.to_string());
    }
}

/// Append the paths a list doesn't hold yet.
fn append(list: &mut Value, paths: &[String]) {
    let have = arr_mut(&mut list["videoPaths"]);
    for p in paths {
        if !have.iter().any(|h| h == p.as_str()) {
            have.push(json!(p));
        }
    }
}

fn paths_of(list: &Value) -> Vec<String> {
    strs(&list["videoPaths"])
}

/// The last of `items[..end]` named `name` under `key` (a JS Map by name).
fn by_name(items: &[Value], name: &str, key: fn(&str) -> String) -> Option<usize> {
    let k = key(name);
    items.iter().rposition(|x| key(s(&x["name"])) == k)
}

fn new_playlist(name: &str, paths: &[String], now: u64) -> Value {
    let mut pl = json!({"id": new_id(), "name": name, "createdAt": now, "videoPaths": []});
    append(&mut pl, paths);
    pl
}

/// Flat by the user's own choice (VR Format `flat`, else Mark as Flat): the
/// only flat decision taken as final until the frames are looked at.
fn marked_flat(lib: &Library, path: &str) -> bool {
    let l = lib.lib();
    match l["vrFormat"][path]["projection"].as_str() {
        Some(p) => p == "flat",
        None => l["manualVRType"][path] == "flat",
    }
}

/// Post-tag categories, then VR (`vr` the app's, `vr-manual` the user's),
/// then the axis kind.
pub(super) fn categories(lib: &mut Library, cx: &Ctx) {
    let state = lib.lib()["autoCategories"].clone();
    let mut created = strs(&state["created"]);
    let mut assigned = state["assigned"].as_object().cloned().unwrap_or_default();
    let mut vc = std::mem::take(&mut lib.config["videoCategories"]);
    let vc = obj_mut(&mut vc);
    let defs = defs("autoCategories");
    let cats = lib.top_array_mut("categories");
    for d in defs {
        if !created.iter().any(|k| k == s(&d["key"]))
            && by_name(cats, s(&d["name"]), name_key).is_none()
        {
            cats.push(json!({"id": new_id(), "name": d["name"], "color": d["color"]}));
        }
    }
    let ids: Vec<Option<String>> = defs
        .iter()
        .map(|d| by_name(cats, s(&d["name"]), name_key).map(|i| s(&cats[i]["id"]).to_string()))
        .collect();
    for d in defs {
        add_created(&mut created, s(&d["key"]));
    }
    let def_tags: Vec<Vec<String>> = defs
        .iter()
        .map(|d| match d["tagScoreGroup"].as_str() {
            Some(g) => RULES.prefs["tagScoreGroups"]
                .as_array()
                .unwrap()
                .iter()
                .find(|x| x["name"] == g)
                .map(|x| strs(&x["tags"]))
                .unwrap_or_default(),
            None => strs(&d["tags"]),
        })
        .collect();
    let in_cat = |vc: &Map<String, Value>, path: &str, id: &str| has(vc, path, id);
    let assign = |vc: &mut Map<String, Value>, path: &str, id: &str| {
        if !in_cat(vc, path, id) {
            push_key(vc, path, id);
        }
    };
    let unassign = |vc: &mut Map<String, Value>, path: &str, id: &str| {
        if let Some(a) = vc.get_mut(path).and_then(Value::as_array_mut) {
            a.retain(|x| x != id);
            if a.is_empty() {
                vc.remove(path);
            }
        }
    };
    let script_tags = &lib.lib()["scriptTags"];
    for v in cx.videos {
        let tags: HashSet<String> = strs(&script_tags[&v.path]["tags"]).into_iter().collect();
        if tags.is_empty() {
            continue;
        }
        for (i, d) in defs.iter().enumerate() {
            let key = s(&d["key"]);
            if has(&assigned, &v.path, key) || !def_tags[i].iter().any(|t| tags.contains(t)) {
                continue;
            }
            let Some(id) = &ids[i] else { continue };
            assign(vc, &v.path, id);
            push_key(&mut assigned, &v.path, key);
        }
    }

    let id_of = |key: &str| {
        defs.iter()
            .position(|d| d["key"] == key)
            .and_then(|i| ids[i].clone())
    };
    if let Some(id) = id_of("vr") {
        for v in cx.videos {
            let (path, done) = (v.path.as_str(), |a: &Map<String, Value>, k| {
                has(a, &v.path, k)
            });
            if cx.vr[path] {
                if done(&assigned, "vr") || done(&assigned, "vr-manual") {
                    continue;
                }
                let key = if in_cat(vc, path, &id) {
                    "vr-manual"
                } else {
                    "vr"
                };
                assign(vc, path, &id);
                push_key(&mut assigned, path, key);
            } else if done(&assigned, "vr") && in_cat(vc, path, &id) && marked_flat(lib, path) {
                unassign(vc, path, &id);
                let rest = arr_mut(assigned.get_mut(path).unwrap());
                rest.retain(|k| k != "vr");
                if rest.is_empty() {
                    assigned.remove(path);
                }
            }
        }
    }

    let axis: Vec<(&str, &str, String)> = defs
        .iter()
        .filter_map(|d| Some((s(&d["key"]), d["axisKind"].as_str()?, id_of(s(&d["key"]))?)))
        .collect();
    for v in cx.videos {
        let Some(kind) = axis_kind(v, cx.embedded) else {
            continue;
        };
        let path = v.path.as_str();
        let mut done = strs(assigned.get(path).unwrap_or(&Value::Null));
        for (key, k, id) in &axis {
            let had = done.iter().any(|x| x == key);
            if *k == kind && !had {
                assign(vc, path, id);
                done.push(key.to_string());
            } else if *k != kind && had {
                unassign(vc, path, id);
                done.retain(|x| x != key);
            }
        }
        if done.is_empty() {
            assigned.remove(path);
        } else {
            assigned.insert(path.to_string(), json!(done));
        }
    }

    let vc = std::mem::take(vc);
    lib.config["videoCategories"] = Value::Object(vc);
    let next = json!({"created": created, "assigned": assigned});
    if lib.lib()["autoCategories"] != next {
        *lib.lib_mut("autoCategories") = next;
    }
}

/// An artist folder: its fold, its spellings with their counts, its videos.
type Artist = (String, Vec<(String, usize)>, Vec<String>);

/// One playlist per artist folder with ARTIST_PLAYLIST_MIN videos, and
/// "Favourite artists" with every favourite's videos; a former favourite's
/// videos the app put there are taken out.
pub(super) fn artist_playlists(lib: &mut Library, cx: &Ctx) {
    let favourites: HashSet<String> = super::favourites(lib).into_iter().map(|f| f.0).collect();
    let mut groups: Vec<Artist> = Vec::new();
    for v in cx.videos {
        let Some(artist) = artist_of(&v.path, &cx.sources) else {
            continue;
        };
        let key = fold(&artist);
        let i = match groups.iter().position(|g| g.0 == key) {
            Some(i) => i,
            None => {
                groups.push((key, Vec::new(), Vec::new()));
                groups.len() - 1
            }
        };
        let g = &mut groups[i];
        match g.1.iter_mut().find(|x| x.0 == artist) {
            Some(x) => x.1 += 1,
            None => g.1.push((artist, 1)),
        }
        g.2.push(v.path.clone());
    }
    groups.sort_by(|a, b| locale_cmp(&a.0, &b.0, false));
    let base = |p: &str| p.rsplit(['/', '\\']).next().unwrap_or(p).to_string();
    for g in &mut groups {
        g.1.sort_by(|a, b| b.1.cmp(&a.1));
        g.2.sort_by(|a, b| locale_cmp(&base(a), &base(b), true));
    }

    let state = lib.lib()["artistPlaylists"].clone();
    let mut created = strs(&state["created"]);
    let mut added = state["added"].as_object().cloned().unwrap_or_default();
    let fav_name = s(&RULES.mirror["favouritesPlaylist"]);
    let min = RULES.mirror["artistPlaylistMin"].as_u64().unwrap() as usize;
    let now = cx.now;
    let playlists = lib.top_array_mut("playlists");
    let n0 = playlists.len();
    let mut plan = |key: &str, name: &str, paths: &[String], playlists: &mut Vec<Value>| {
        let existing = by_name(&playlists[..n0], name, fold);
        if existing.is_none() && created.iter().any(|k| k == key) {
            return;
        }
        add_created(&mut created, key);
        let in_list: HashSet<String> = existing
            .map(|i| paths_of(&playlists[i]))
            .unwrap_or_default()
            .into_iter()
            .collect();
        let fresh: Vec<String> = paths
            .iter()
            .filter(|p| !in_list.contains(*p) && !has(&added, p, key))
            .cloned()
            .collect();
        for p in paths {
            if !has(&added, p, key) {
                push_key(&mut added, p, key);
            }
        }
        match existing {
            None => playlists.push(new_playlist(name, paths, now)),
            Some(i) => append(&mut playlists[i], &fresh),
        }
    };
    let fav_paths: Vec<String> = groups
        .iter()
        .filter(|g| favourites.contains(&g.0))
        .flat_map(|g| g.2.clone())
        .collect();
    if !fav_paths.is_empty() {
        plan(FAVOURITES_KEY, fav_name, &fav_paths, playlists);
    }
    for g in groups.iter().filter(|g| g.2.len() >= min) {
        plan(&g.0, &g.1[0].0, &g.2, playlists);
    }
    let withdrawn: Vec<&String> = groups
        .iter()
        .filter(|g| !favourites.contains(&g.0))
        .flat_map(|g| &g.2)
        .filter(|p| has(&added, p, FAVOURITES_KEY))
        .collect();
    for p in &withdrawn {
        let keys = arr_mut(added.get_mut(p.as_str()).unwrap());
        keys.retain(|k| k != FAVOURITES_KEY);
        if keys.is_empty() {
            added.remove(p.as_str());
        }
    }
    if let Some(i) = by_name(&playlists[..n0], fav_name, fold) {
        arr_mut(&mut playlists[i]["videoPaths"])
            .retain(|p| !withdrawn.iter().any(|w| p == w.as_str()));
    }
    let next = json!({"created": created, "added": added});
    if lib.lib()["artistPlaylists"] != next {
        *lib.lib_mut("artistPlaylists") = next;
    }
}

fn is_smart_name(name: &str) -> bool {
    let k = fold(name);
    !k.is_empty()
        && defs("smartPlaylists")
            .iter()
            .any(|d| fold(s(&d["name"])) == k)
}

/// A playlist with no video in the library goes (smart playlists stay); an
/// artist playlist the app made goes below ARTIST_PLAYLIST_MIN and its
/// artist is forgotten, so it's made again at the threshold.
pub(super) fn playlist_cleanup(lib: &mut Library, cx: &Ctx) {
    let in_library: HashSet<&str> = cx.videos.iter().map(|v| v.path.as_str()).collect();
    let state = lib.lib()["artistPlaylists"].clone();
    let created = strs(&state["created"]);
    let fav = fold(s(&RULES.mirror["favouritesPlaylist"]));
    let min = RULES.mirror["artistPlaylistMin"].as_u64().unwrap() as usize;
    let mut forget = HashSet::new();
    let mut remove = HashSet::new();
    for pl in lib.config["playlists"].as_array().into_iter().flatten() {
        let name = s(&pl["name"]);
        if is_smart_name(name) {
            continue;
        }
        let videos = paths_of(pl)
            .iter()
            .filter(|p| in_library.contains(p.as_str()))
            .count();
        let key = if fold(name) == fav {
            FAVOURITES_KEY.to_string()
        } else {
            fold(name)
        };
        let artist_made = created.contains(&key);
        let need = if !artist_made || key == FAVOURITES_KEY {
            1
        } else {
            min
        };
        if videos < need {
            remove.insert(pl["id"].clone());
            if artist_made {
                forget.insert(key);
            }
        }
    }
    if !remove.is_empty() {
        lib.top_array_mut("playlists")
            .retain(|pl| !remove.contains(&pl["id"]));
    }
    if forget.is_empty() {
        return;
    }
    let mut next = state;
    next["created"] = json!(
        created
            .iter()
            .filter(|k| !forget.contains(*k))
            .collect::<Vec<_>>()
    );
    let mut added = Map::new();
    for (p, keys) in next["added"].as_object().into_iter().flatten() {
        let rest: Vec<&Value> = keys
            .as_array()
            .into_iter()
            .flatten()
            .filter(|k| !forget.contains(s(k)))
            .collect();
        if !rest.is_empty() {
            added.insert(p.clone(), json!(rest));
        }
    }
    next["added"] = Value::Object(added);
    *lib.lib_mut("artistPlaylists") = next;
}

/// AUTO_COLLECTIONS: by post tag, title words, length (the post's `len-*`
/// tags while the duration is unknown) or FapTap as the source.
pub(super) fn auto_collections(lib: &mut Library, cx: &Ctx) {
    let l = lib.lib();
    let matches = |d: &Value, path: &str| {
        let record = &l["scriptTags"][path];
        let tags: HashSet<String> = strs(&record["tags"]).into_iter().collect();
        if d["fapTap"] == true {
            return record["source"] == FAPTAP;
        }
        let any = |k: &str| strs(&d[k]).iter().any(|t| tags.contains(t));
        if d["length"].is_object() {
            let dur = l["durationCache"][path].as_f64().unwrap_or(0.0);
            return match (dur > 0.0, d["length"]["under"].as_f64()) {
                (false, _) => any("lengthTags"),
                (true, Some(under)) => dur < under,
                (true, None) => dur >= d["length"]["atLeast"].as_f64().unwrap_or(f64::INFINITY),
            };
        }
        any("tags")
            || RULES
                .collection_titles
                .get(s(&d["key"]))
                .is_some_and(|re| re.is_match(&list_title(path)))
    };
    let state = l["autoCollections"].clone();
    let mut created = strs(&state["created"]);
    let mut added = state["added"].as_object().cloned().unwrap_or_default();
    let mut plans: Vec<(&Value, Option<usize>, Vec<String>)> = Vec::new();
    let collections = l["collections"].as_array().cloned().unwrap_or_default();
    for d in defs("autoCollections") {
        let key = s(&d["key"]);
        let existing = by_name(&collections, s(&d["name"]), name_key);
        if existing.is_none() && created.iter().any(|k| k == key) {
            continue;
        }
        let fresh: Vec<String> = cx
            .videos
            .iter()
            .filter(|v| !has(&added, &v.path, key) && matches(d, &v.path))
            .map(|v| v.path.clone())
            .collect();
        if existing.is_none() && fresh.is_empty() {
            continue;
        }
        add_created(&mut created, key);
        for p in &fresh {
            push_key(&mut added, p, key);
        }
        let in_list: HashSet<String> = existing
            .map(|i| paths_of(&collections[i]))
            .unwrap_or_default()
            .into_iter()
            .collect();
        plans.push((
            d,
            existing,
            fresh.into_iter().filter(|p| !in_list.contains(p)).collect(),
        ));
    }
    let cols = arr_mut(lib.lib_mut("collections"));
    for (d, existing, to_add) in plans {
        match existing {
            None => cols.push(json!({"id": new_id(), "name": d["name"], "videoPaths": to_add})),
            Some(i) => {
                arr_mut(&mut cols[i]["videoPaths"]).extend(to_add.into_iter().map(Value::from))
            }
        }
    }
    let next = json!({"created": created, "added": added});
    if lib.lib()["autoCollections"] != next {
        *lib.lib_mut("autoCollections") = next;
    }
}

fn score_of(lib: &Library, path: &str) -> Option<f64> {
    lib.lib()["preferenceScores"][path]
        .as_f64()
        .filter(|x| x.is_finite())
}

/// Highest score first, then title, then path.
fn by_score(paths: &mut [String], scores: &HashMap<String, Option<f64>>) {
    let titles: HashMap<String, String> =
        paths.iter().map(|p| (p.clone(), list_title(p))).collect();
    let sc = |p: &String| scores[p].unwrap_or(f64::NEG_INFINITY);
    paths.sort_by(|a, b| {
        sc(b)
            .partial_cmp(&sc(a))
            .unwrap_or(Ordering::Equal)
            .then_with(|| locale_cmp(&titles[a], &titles[b], false))
            .then_with(|| a.cmp(b))
    });
}

/// A list that holds exactly `want`: an existing one under its name or a
/// former name is taken over (and renamed); otherwise made once, when it
/// has something to hold. Returns the index of the list it wrote, if any.
fn keep_exact(
    lists: &mut Vec<Value>,
    created: &mut Vec<String>,
    d: &Value,
    want: Vec<String>,
    key: fn(&str) -> String,
    make: impl FnOnce(&str, &[String]) -> Value,
) {
    let names =
        std::iter::once(&d["name"]).chain(d["formerNames"].as_array().into_iter().flatten());
    let n = lists.len();
    let existing = names
        .filter_map(|name| by_name(&lists[..n], s(name), key))
        .next();
    let Some(i) = existing else {
        if !created.iter().any(|k| k == s(&d["key"])) && !want.is_empty() {
            add_created(created, s(&d["key"]));
            lists.push(make(s(&d["name"]), &want));
        }
        return;
    };
    add_created(created, s(&d["key"]));
    if key(s(&lists[i]["name"])) != key(s(&d["name"])) {
        lists[i]["name"] = d["name"].clone();
    }
    if paths_of(&lists[i]) != want {
        lists[i]["videoPaths"] = json!(want);
    }
}

/// SCORE_BANDS: `min <= score < below`, each a collection of exactly its
/// band, highest score first.
pub(super) fn score_bands(lib: &mut Library, cx: &Ctx) {
    let scores: HashMap<String, Option<f64>> = cx
        .videos
        .iter()
        .map(|v| (v.path.clone(), score_of(lib, &v.path)))
        .collect();
    let state = lib.lib()["autoCollections"].clone();
    let mut created = strs(&state["created"]);
    let mut cols = std::mem::take(arr_mut(lib.lib_mut("collections")));
    for band in defs("scoreBands") {
        let mut want: Vec<String> = cx
            .videos
            .iter()
            .filter(|v| {
                scores[&v.path].is_some_and(|x| {
                    band["min"].as_f64().is_none_or(|m| x >= m)
                        && band["below"].as_f64().is_none_or(|b| x < b)
                })
            })
            .map(|v| v.path.clone())
            .collect();
        by_score(&mut want, &scores);
        keep_exact(
            &mut cols,
            &mut created,
            band,
            want,
            name_key,
            |name, paths| json!({"id": new_id(), "name": name, "videoPaths": paths}),
        );
    }
    *lib.lib_mut("collections") = Value::Array(cols);
    if strs(&state["created"]) != created {
        let mut next = state;
        next["created"] = json!(created);
        *lib.lib_mut("autoCollections") = next;
    }
}

/// SMART_PLAYLISTS: unwatched videos (Favorites: every hearted one) that
/// meet the rules, best score first.
pub(super) fn smart_playlists(lib: &mut Library, cx: &Ctx) {
    let scores: HashMap<String, Option<f64>> = cx
        .videos
        .iter()
        .map(|v| (v.path.clone(), score_of(lib, &v.path)))
        .collect();
    let mut members = Vec::new();
    for d in defs("smartPlaylists") {
        let mut want: Vec<String> = cx
            .videos
            .iter()
            .filter(|v| {
                let (rating, favorite) = rating_entry(lib, &v.path);
                let duration = lib.lib()["durationCache"][&v.path].as_f64().unwrap_or(0.0);
                (if d["favorite"] == true {
                    favorite
                } else {
                    rating <= 0.0
                }) && (d["vr"] != true || cx.vr[v.path.as_str()])
                    && d["minScore"]
                        .as_f64()
                        .is_none_or(|m| scores[&v.path].is_some_and(|x| x >= m))
                    && d["maxSeconds"]
                        .as_f64()
                        .is_none_or(|m| duration > 0.0 && duration < m)
            })
            .map(|v| v.path.clone())
            .collect();
        by_score(&mut want, &scores);
        members.push(want);
    }
    let state = lib.lib()["smartPlaylists"].clone();
    let mut created = strs(&state["created"]);
    let now = cx.now;
    let playlists = lib.top_array_mut("playlists");
    for (d, want) in defs("smartPlaylists").iter().zip(members) {
        keep_exact(playlists, &mut created, d, want, fold, |name, paths| {
            new_playlist(name, paths, now)
        });
    }
    if strs(&state["created"]).len() != created.len() {
        let mut next = state;
        next["created"] = json!(created);
        *lib.lib_mut("smartPlaylists") = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::Video;

    fn video(path: &str) -> Video {
        serde_json::from_value(json!({"id": "x", "name": "n", "path": path, "ext": ".mp4",
            "funscriptPath": format!("{path}.funscript"), "fuzzyMatched": false, "subtitlePath": null,
            "variants": [], "axes": [], "sourceName": "s", "dateAdded": 0.0, "size": 0}))
        .unwrap()
    }

    fn library(settings: Value) -> Library {
        Library {
            config: json!({"settings": {"library": settings}}),
            ..Default::default()
        }
    }

    fn cx<'a>(videos: &'a [Video], vr: &[&str], emb: &'a HashSet<String>) -> Ctx<'a> {
        Ctx {
            videos,
            embedded: emb,
            vr: videos
                .iter()
                .map(|v| (v.path.as_str(), vr.contains(&v.path.as_str())))
                .collect(),
            sources: vec!["/L".into()],
            now: 1,
        }
    }

    fn list<'a>(items: &'a Value, name: &str) -> Option<&'a Value> {
        items.as_array()?.iter().find(|x| x["name"] == name)
    }

    fn members(items: &Value, name: &str) -> Vec<String> {
        list(items, name).map(paths_of).unwrap_or_default()
    }

    #[test]
    fn quick_ones_take_under_five_minutes_only() {
        let vs: Vec<Video> = ["/L/a", "/L/b", "/L/c", "/L/d"].map(video).into();
        let mut lib = library(json!({
            "preferenceScores": {"/L/a": 15, "/L/b": 16, "/L/c": 20, "/L/d": 14.5},
            "durationCache": {"/L/a": 299.9, "/L/b": 300, "/L/c": 10, "/L/d": 10},
            "ratings": {"/L/c": {"rating": 4}}
        }));
        let emb = HashSet::new();
        smart_playlists(&mut lib, &cx(&vs, &[], &emb));
        assert_eq!(
            members(&lib.config["playlists"], "Quick ones"),
            ["/L/a"],
            "299.9 s in, 300 s out, watched out, < 15 out"
        );
        assert!(
            list(&lib.config["playlists"], "VR picks").is_none(),
            "not made while empty"
        );
        assert_eq!(
            lib.lib()["smartPlaylists"]["created"],
            json!(["quick-ones"])
        );
    }

    #[test]
    fn smart_playlists_take_over_former_names_and_come_back_in_order() {
        let vs: Vec<Video> = ["/L/a", "/L/b", "/L/c"].map(video).into();
        let mut lib = library(json!({
            "preferenceScores": {"/L/a": 1, "/L/b": 9, "/L/c": 20},
            "ratings": {"/L/a": {"favorite": true}, "/L/b": {"rating": 5, "favorite": true}},
        }));
        lib.config["playlists"] =
            json!([{"id": "p", "name": "Fresh downloads", "videoPaths": ["/L/c"]}]);
        let emb = HashSet::new();
        smart_playlists(&mut lib, &cx(&vs, &["/L/c"], &emb));
        let pl = &lib.config["playlists"];
        assert_eq!(
            members(pl, "Favorites"),
            ["/L/b", "/L/a"],
            "hearted, watched or not, best first"
        );
        assert_eq!(list(pl, "Favorites").unwrap()["id"], "p");
        assert_eq!(members(pl, "VR picks"), ["/L/c"]);
        // Deleted by the user: not made again.
        lib.config["playlists"] = json!([]);
        smart_playlists(&mut lib, &cx(&vs, &["/L/c"], &emb));
        assert!(list(&lib.config["playlists"], "VR picks").is_none());
    }

    #[test]
    fn score_bands_split_at_their_edges() {
        let paths = ["/L/a", "/L/b", "/L/c", "/L/d", "/L/e", "/L/f", "/L/g"];
        let vs: Vec<Video> = paths.map(video).into();
        let mut lib = library(json!({
            "preferenceScores": {"/L/a": 20, "/L/b": 19.5, "/L/c": 15, "/L/d": 14.5, "/L/e": 10, "/L/f": 9.5, "/L/g": "x"},
            "collections": [{"id": "old", "name": "Score 15+", "videoPaths": ["/L/f"]}],
            "autoCollections": {"created": ["score-low"], "added": {"/L/z": ["music"]}}
        }));
        let emb = HashSet::new();
        score_bands(&mut lib, &cx(&vs, &[], &emb));
        let c = &lib.lib()["collections"];
        assert_eq!(members(c, "Score 20+"), ["/L/a"]);
        assert_eq!(members(c, "Score 15–19.5"), ["/L/b", "/L/c"]);
        assert_eq!(
            list(c, "Score 15–19.5").unwrap()["id"],
            "old",
            "renamed from a former name"
        );
        assert_eq!(members(c, "Score 10–14.5"), ["/L/d", "/L/e"]);
        assert!(
            list(c, "Score below 10").is_none(),
            "deleted by the user: not made again"
        );
        assert_eq!(
            lib.lib()["autoCollections"]["added"]["/L/z"],
            json!(["music"])
        );
    }

    #[test]
    fn artist_playlists_need_the_threshold_and_stay_deleted() {
        let mut paths: Vec<String> = (0..29).map(|i| format!("/L/Ann/s{i}/v.mp4")).collect();
        paths.extend((0..30).map(|i| format!("/L/Bob/s{i}/v.mp4")));
        paths.push("/L/Agapilord/s/v.mp4".into());
        let vs: Vec<Video> = paths.iter().map(|p| video(p)).collect();
        let mut lib = library(json!({}));
        let emb = HashSet::new();
        artist_playlists(&mut lib, &cx(&vs, &[], &emb));
        let pl = lib.config["playlists"].clone();
        assert!(list(&pl, "Ann").is_none(), "29 videos: no playlist");
        assert_eq!(members(&pl, "Bob").len(), 30);
        assert_eq!(
            members(&pl, "Favourite artists"),
            ["/L/Agapilord/s/v.mp4"],
            "a favourite from one video"
        );
        assert!(list(&pl, "Agapilord").is_none());
        assert_eq!(
            members(&pl, "Bob")[..2],
            ["/L/Bob/s0/v.mp4", "/L/Bob/s1/v.mp4"]
        );

        // The user takes a video out and deletes "Favourite artists": both stick.
        arr_mut(&mut lib.config["playlists"][1]["videoPaths"]).retain(|p| p != "/L/Bob/s3/v.mp4");
        lib.config["playlists"]
            .as_array_mut()
            .unwrap()
            .retain(|p| p["name"] != "Favourite artists");
        artist_playlists(&mut lib, &cx(&vs, &[], &emb));
        assert_eq!(members(&lib.config["playlists"], "Bob").len(), 29);
        assert!(list(&lib.config["playlists"], "Favourite artists").is_none());

        // Bob drops below the threshold: the cleanup removes and forgets it.
        let fewer = &vs[..vs.len() - 2];
        playlist_cleanup(&mut lib, &cx(fewer, &[], &emb));
        assert!(list(&lib.config["playlists"], "Bob").is_none());
        let state = &lib.lib()["artistPlaylists"];
        assert!(!strs(&state["created"]).contains(&"bob".to_string()));
        assert!(!has(
            state["added"].as_object().unwrap(),
            "/L/Bob/s0/v.mp4",
            "bob"
        ));
    }

    #[test]
    fn cleanup_removes_empty_playlists_but_not_smart_ones() {
        let vs = vec![video("/L/a")];
        let mut lib = library(json!({}));
        lib.config["playlists"] = json!([
            {"id": "1", "name": "Mine", "videoPaths": ["/gone"]},
            {"id": "2", "name": "Kept", "videoPaths": ["/L/a"]},
            {"id": "3", "name": "VR picks", "videoPaths": []}
        ]);
        let emb = HashSet::new();
        playlist_cleanup(&mut lib, &cx(&vs, &[], &emb));
        let names: Vec<&str> = lib.config["playlists"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| s(&p["name"]))
            .collect();
        assert_eq!(names, ["Kept", "VR picks"]);
    }

    #[test]
    fn categories_follow_tags_vr_and_axes_once() {
        let mut vs: Vec<Video> = ["/L/a", "/L/b", "/L/c"].map(video).into();
        vs[0].axes =
            serde_json::from_value(json!([{"axis": "twist", "path": "t", "generated": true}]))
                .unwrap();
        let mut lib = library(json!({
            "scriptTags": {"/L/a": {"tags": ["creampie", "male-pov"]}, "/L/b": {"tags": ["anal"]}},
            "autoCategories": {"created": ["anal"]}
        }));
        let emb: HashSet<String> = ["/L/b".to_string()].into();
        categories(&mut lib, &cx(&vs, &["/L/a"], &emb));
        let cat = |lib: &Library, name: &str| -> Vec<String> {
            let id = list(&lib.config["categories"], name)
                .map(|c| c["id"].clone())
                .unwrap_or_default();
            let vc = lib.config["videoCategories"].as_object().unwrap();
            vc.iter()
                .filter(|(_, ids)| ids.as_array().unwrap().contains(&id))
                .map(|(p, _)| p.clone())
                .collect()
        };
        assert_eq!(cat(&lib, "Creampie"), ["/L/a"]);
        assert_eq!(
            cat(&lib, "POV"),
            ["/L/a"],
            "POV's tags come from the POV tag-score group"
        );
        assert!(
            list(&lib.config["categories"], "Anal").is_none(),
            "deleted by the user: not made again"
        );
        assert_eq!(cat(&lib, "VR"), ["/L/a"]);
        assert_eq!(cat(&lib, "Generated axes"), ["/L/a"]);
        assert_eq!(
            cat(&lib, "Hand-made axes"),
            ["/L/b"],
            "embedded axes are hand-made"
        );
        assert_eq!(cat(&lib, "Single-axis"), ["/L/c"]);

        // Taken out by hand: stays out. Not VR any more but not marked flat: stays.
        let creampie = list(&lib.config["categories"], "Creampie").unwrap()["id"].clone();
        arr_mut(&mut lib.config["videoCategories"]["/L/a"]).retain(|x| *x != creampie);
        categories(&mut lib, &cx(&vs, &[], &emb));
        assert!(cat(&lib, "Creampie").is_empty());
        assert_eq!(cat(&lib, "VR"), ["/L/a"]);
        // Marked flat: the app takes out what it put in.
        lib.config["settings"]["library"]["manualVRType"] = json!({"/L/a": "flat"});
        categories(&mut lib, &cx(&vs, &[], &emb));
        assert!(cat(&lib, "VR").is_empty());
        // Hand-made now: moves from Generated to Hand-made.
        vs[0].axes[0].generated = false;
        categories(&mut lib, &cx(&vs, &[], &emb));
        assert_eq!(cat(&lib, "Hand-made axes"), ["/L/a", "/L/b"]);
        assert!(cat(&lib, "Generated axes").is_empty());
    }

    #[test]
    fn auto_collections_by_length_title_and_source() {
        let vs: Vec<Video> = [
            "/L/a/Mercy_Fun.mp4",
            "/L/b/Mercyful.mp4",
            "/L/c/x.mp4",
            "/L/d/y.mp4",
        ]
        .map(video)
        .into();
        let mut lib = library(json!({
            "scriptTags": {
                "/L/c/x.mp4": {"tags": ["len-2-5"], "source": "faptap-scout"},
                "/L/d/y.mp4": {"tags": ["len-2-5"]}
            },
            "durationCache": {"/L/d/y.mp4": 300, "/L/a/Mercy_Fun.mp4": 299},
            "collections": [{"id": "ow", "name": "overwatch", "videoPaths": []}]
        }));
        let emb = HashSet::new();
        auto_collections(&mut lib, &cx(&vs, &[], &emb));
        let c = &lib.lib()["collections"];
        assert_eq!(
            members(c, "overwatch"),
            ["/L/a/Mercy_Fun.mp4"],
            "whole words, existing name reused"
        );
        assert_eq!(
            members(c, "Quick (under 5 min)"),
            ["/L/a/Mercy_Fun.mp4", "/L/c/x.mp4"],
            "len tags only without a duration"
        );
        assert_eq!(members(c, "FapTap"), ["/L/c/x.mp4"]);
        assert!(
            list(c, "Hentai").is_none(),
            "made only when something matches"
        );
        // Taken out by hand: not added again.
        lib.config["settings"]["library"]["collections"][0]["videoPaths"] = json!([]);
        auto_collections(&mut lib, &cx(&vs, &[], &emb));
        assert!(members(&lib.lib()["collections"], "overwatch").is_empty());
    }
}
