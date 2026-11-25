//! Parity with the Electron app on real data (ignored by default; reads only):
//! `FUNSYNC_PARITY_DIR=<data dir with library.json from import + scan.json>
//! cargo test -p funsync-server --test prefs_parity -- --ignored --nocapture`.
//! Runs `prefs::refresh_with` on the imported state (what the old app computed)
//! and from scratch, and reports how far each agrees with the imported values.

use std::collections::{BTreeMap, BTreeSet};

use funsync_server::data::{self, Library};
use funsync_server::prefs::{SideFiles, refresh_with};
use funsync_server::server::Snapshot;
use serde_json::{Value, json};

type Lists = BTreeMap<String, BTreeSet<String>>;

fn named(items: &Value) -> Lists {
    items
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| {
            let paths = x["videoPaths"].as_array().into_iter().flatten();
            let name = x["name"].as_str().unwrap_or("").to_string();
            (
                name,
                paths.filter_map(|p| p.as_str().map(String::from)).collect(),
            )
        })
        .collect()
}

fn categories(lib: &Library) -> Lists {
    let mut out = Lists::new();
    for c in lib.config["categories"].as_array().into_iter().flatten() {
        let id = &c["id"];
        let members = lib.config["videoCategories"]
            .as_object()
            .into_iter()
            .flatten();
        let set = members
            .filter(|(_, ids)| ids.as_array().is_some_and(|a| a.contains(id)))
            .map(|(p, _)| p.clone())
            .collect();
        out.insert(c["name"].as_str().unwrap_or("").to_string(), set);
    }
    out
}

fn compare(what: &str, old: &Lists, new: &Lists, scanned: &BTreeSet<String>) {
    for (name, want) in old {
        let want: BTreeSet<_> = want.intersection(scanned).cloned().collect();
        let got: BTreeSet<_> = new
            .get(name)
            .cloned()
            .unwrap_or_default()
            .intersection(scanned)
            .cloned()
            .collect();
        let same = want.intersection(&got).count();
        let missing: Vec<_> = want.difference(&got).take(3).collect();
        let extra: Vec<_> = got.difference(&want).take(3).collect();
        println!(
            "{what} {name:?}: old {} new {} common {same}{}",
            want.len(),
            got.len(),
            if missing.is_empty() && extra.is_empty() {
                String::new()
            } else {
                format!("\n    missing {missing:?}\n    extra {extra:?}")
            }
        );
    }
    for name in new.keys().filter(|n| !old.contains_key(*n)) {
        println!("{what} {name:?}: only new ({})", new[name].len());
    }
}

fn scores(lib: &Library) -> BTreeMap<String, f64> {
    let s = lib.lib()["preferenceScores"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    s.into_iter()
        .filter_map(|(k, v)| Some((k, v.as_f64()?)))
        .collect()
}

#[test]
#[ignore]
fn parity_with_the_electron_app() {
    let dir =
        std::path::PathBuf::from(std::env::var("FUNSYNC_PARITY_DIR").expect("FUNSYNC_PARITY_DIR"));
    let old = data::load_library(&dir).unwrap();
    // The frame check's results, when the data dir has them (frames_parity).
    funsync_server::stereo::load(&dir);
    let mut snap: Snapshot =
        serde_json::from_slice(&std::fs::read(dir.join(data::SCAN_FILE)).unwrap()).unwrap();
    snap.embedded_axes = funsync_server::prefs::embedded_axes(&snap.videos);
    println!("embedded axes: {} videos", snap.embedded_axes.len());
    let side = SideFiles::load(funsync_server::prefs::side_dir(&old).as_deref());
    let scanned: BTreeSet<String> = snap.videos.iter().map(|v| v.path.clone()).collect();
    let now = data::now_ms();
    // Frame sizes as the old backend has them cached (ffprobe), for the VR rule.
    let media = funsync_server::media::Media::new(std::env::temp_dir().join("prefs-parity-thumbs"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dims: BTreeMap<String, (u32, u32)> = snap
        .videos
        .iter()
        .filter(|v| !funsync_server::vr::is_vr(&old, v))
        .filter_map(|v| {
            let m = rt
                .block_on(media.metadata(std::path::Path::new(&v.path)))
                .ok()?;
            Some((v.path.clone(), (m.width, m.height)))
        })
        .collect();
    let dims = |p: &str| dims.get(p).copied();

    // 1. On the old app's own state: what would change.
    let mut lib = old.clone();
    refresh_with(&mut lib, &snap, Some(&side), now, dims);
    let (a, b) = (&old.lib()["scriptTags"], &lib.lib()["scriptTags"]);
    let changed: Vec<_> = b
        .as_object()
        .unwrap()
        .iter()
        .filter(|(k, v)| a[k.as_str()] != **v)
        .map(|(k, _)| k.clone())
        .collect();
    let gone = a
        .as_object()
        .unwrap()
        .keys()
        .filter(|k| b.get(k.as_str()).is_none())
        .count();
    println!(
        "scriptTags: {} records, {} changed, {} removed {:?}",
        b.as_object().unwrap().len(),
        changed.len(),
        gone,
        changed.iter().take(3).collect::<Vec<_>>()
    );
    let (so, sn) = (scores(&old), scores(&lib));
    let mut diff: Vec<_> = sn
        .iter()
        .filter(|(k, v)| so.get(*k).is_none_or(|o| (o - **v).abs() > 0.001))
        .collect();
    println!(
        "preferenceScores: {} scored, {} equal, {} differ",
        sn.len(),
        sn.len() - diff.len(),
        diff.len()
    );
    diff.sort_by(|x, y| x.0.cmp(y.0));
    let vr_only = diff
        .iter()
        .filter(|(k, v)| so.get(*k).is_some_and(|o| (o - **v - 3.0).abs() < 0.001))
        .count();
    println!("    {vr_only} of them exactly 3 lower (the VR weight)");
    for (k, v) in diff
        .iter()
        .filter(|(k, v)| so.get(*k).is_none_or(|o| (o - **v - 3.0).abs() >= 0.001))
        .take(40)
    {
        println!("    {k}: old {:?} new {v}", so.get(*k));
    }
    for key in [
        "autoCategories",
        "artistPlaylists",
        "autoCollections",
        "smartPlaylists",
    ] {
        println!(
            "{key} bookkeeping unchanged: {}",
            old.lib()[key] == lib.lib()[key]
        );
    }
    let ss = old.lib()["speedStatsCache"]
        .as_object()
        .map_or(0, |m| m.len());
    println!(
        "speedStatsCache: {} → {}",
        ss,
        lib.lib()["speedStatsCache"]
            .as_object()
            .map_or(0, |m| m.len())
    );
    compare("category", &categories(&old), &categories(&lib), &scanned);
    compare(
        "playlist",
        &named(&old.config["playlists"]),
        &named(&lib.config["playlists"]),
        &scanned,
    );
    compare(
        "collection",
        &named(&old.lib()["collections"]),
        &named(&lib.lib()["collections"]),
        &scanned,
    );
    let order = |l: &Library, n: &str| {
        l.config["playlists"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == n)
            .map(|p| p["videoPaths"].clone())
    };
    for n in ["VR picks", "Quick ones", "Favorites", "Score 20+"] {
        let o = order(&old, n).or_else(|| {
            old.lib()["collections"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == n)
                .map(|p| p["videoPaths"].clone())
        });
        let w = order(&lib, n).or_else(|| {
            lib.lib()["collections"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == n)
                .map(|p| p["videoPaths"].clone())
        });
        let keep = |v: Option<Value>| -> Vec<String> {
            v.into_iter()
                .flat_map(|a| a.as_array().cloned().unwrap_or_default())
                .filter_map(|p| p.as_str().map(String::from))
                .filter(|p| scanned.contains(p))
                .collect()
        };
        let (o, w) = (keep(o), keep(w));
        let first = o.iter().zip(&w).position(|(a, b)| a != b);
        println!(
            "list {n:?} same order: {} (first difference at {first:?})",
            o == w
        );
        if let Some(i) = first {
            println!(
                "    old {:?}\n    new {:?}",
                &o[i..(i + 3).min(o.len())],
                &w[i..(i + 3).min(w.len())]
            );
        }
    }

    // 2. From scratch (no lists, no bookkeeping): what the rules make.
    let mut fresh = old.clone();
    for k in [
        "autoCategories",
        "artistPlaylists",
        "autoCollections",
        "smartPlaylists",
        "collections",
        "preferenceScores",
    ] {
        fresh.config["settings"]["library"]
            .as_object_mut()
            .unwrap()
            .remove(k);
    }
    fresh.config["categories"] = json!([]);
    fresh.config["videoCategories"] = json!({});
    fresh.config["playlists"] = json!([]);
    refresh_with(&mut fresh, &snap, Some(&side), now, dims);
    println!("--- from scratch");
    compare("category", &categories(&old), &categories(&fresh), &scanned);
    compare(
        "playlist",
        &named(&old.config["playlists"]),
        &named(&fresh.config["playlists"]),
        &scanned,
    );
    compare(
        "collection",
        &named(&old.lib()["collections"]),
        &named(&fresh.lib()["collections"]),
        &scanned,
    );
}
