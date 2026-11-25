//! Parity of the frame check with the old backend (ignored by default; only
//! reads the library and the old app's folder):
//! `FUNSYNC_FRAMES_DIR=<scratch dir> cargo test --release -p funsync-server
//! --test frames_parity -- --ignored --nocapture`. Looks at every video in
//! the old `vr-stereo-cache.json` whose file is unchanged and compares the
//! layouts; the results stay in `<scratch dir>/vr-stereo-cache.json`.

use std::os::unix::fs::MetadataExt;

use funsync_server::media::Media;
use funsync_server::stereo;
use serde_json::Value;

#[test]
#[ignore]
fn frame_check_matches_the_old_backend() {
    let dir =
        std::path::PathBuf::from(std::env::var("FUNSYNC_FRAMES_DIR").expect("FUNSYNC_FRAMES_DIR"));
    let old_dir = funsync_server::data::default_old_user_data().unwrap();
    let old: Value =
        serde_json::from_slice(&std::fs::read(old_dir.join(stereo::CACHE_FILE)).unwrap()).unwrap();
    stereo::load(&dir);
    let media = Media::new(dir.join("thumbs"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut targets = Vec::new();
    let mut expected = Vec::new();
    for (path, e) in old["entries"].as_object().unwrap() {
        let Ok(m) = std::fs::metadata(path) else {
            continue;
        };
        let ns = m.mtime() as u64 * 1_000_000_000 + m.mtime_nsec() as u64;
        if e["size"] != m.len() || e["mtimeNs"] != ns {
            continue;
        }
        let duration = rt
            .block_on(media.metadata(std::path::Path::new(path)))
            .map_or(0.0, |m| m.duration);
        targets.push((path.clone(), duration));
        expected.push((path.clone(), e["layout"].as_str().map(String::from)));
    }
    let looked = stereo::check(&media, &targets);
    let mut same = 0;
    for (path, want) in &expected {
        let got = stereo::layout(path);
        if got == want.as_deref() {
            same += 1;
        } else {
            println!("differs: {path}: old {want:?} new {got:?}");
        }
    }
    println!(
        "{same} / {} the same ({looked} looked at now)",
        expected.len()
    );
}

/// What the poster import would fetch on the real library right after an
/// import (no network): the old app already tried every link it had.
#[test]
#[ignore]
fn poster_plan_after_an_import() {
    let old_dir = funsync_server::data::default_old_user_data().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    funsync_server::import::import(&old_dir, tmp.path(), true).unwrap();
    let lib = funsync_server::data::load_library(tmp.path()).unwrap();
    let sources: Vec<funsync_server::scan::Source> = lib
        .sources()
        .into_iter()
        .filter(|s| s.enabled)
        .map(|s| funsync_server::scan::Source {
            name: s.name,
            path: s.path,
        })
        .collect();
    let videos = funsync_server::scan::scan_library(&sources).videos;
    let jobs = funsync_server::posters::plan(&lib, &videos, funsync_server::data::now_ms());
    let covers = videos
        .iter()
        .filter(|v| {
            v.eroscripts
                .as_ref()
                .is_some_and(|e| e.poster_url.is_some())
        })
        .count();
    let faptap = videos
        .iter()
        .filter(|v| lib.lib()["scriptTags"][&v.path]["source"] == "faptap-scout")
        .count();
    println!(
        "{} videos, {faptap} FapTap scenes, {covers} rule34video covers: {} to try",
        videos.len(),
        jobs.len()
    );
    for j in &jobs {
        let tried = &lib.lib()[match j.kind {
            funsync_server::posters::Kind::Faptap(_) => "faptapThumbnails",
            funsync_server::posters::Kind::Rule34(_) => "rule34videoThumbnails",
        }][&j.path];
        println!("    {} {} (old attempt {tried})", j.path, j.url);
    }
}

/// The pHash match against the old app's own verdicts: every poster it set
/// by itself (EroScripts previews, FapTap thumbnails, all accepted, with
/// their `matchScore`) is checked again here, reading only.
#[test]
#[ignore]
fn poster_match_agrees_with_the_old_app() {
    let old_dir = funsync_server::data::default_old_user_data().unwrap();
    let cfg: Value =
        serde_json::from_slice(&std::fs::read(old_dir.join("config.json")).unwrap()).unwrap();
    let cache: Value = std::fs::read(old_dir.join("preview-match-cache.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let tmp = tempfile::tempdir().unwrap();
    let media = Media::new(tmp.path().join("thumbs"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mut same, mut kept, mut total) = (0, 0, 0);
    for (path, e) in cfg["settings"]["library"]["customThumbnails"]
        .as_object()
        .unwrap()
    {
        let (Some(img), Some(old)) = (e["imagePath"].as_str(), e["matchScore"].as_u64()) else {
            continue;
        };
        if !std::path::Path::new(path).is_file() || !std::path::Path::new(img).is_file() {
            continue;
        }
        let _ = rt.block_on(media.metadata(std::path::Path::new(path)));
        total += 1;
        let got = funsync_server::previews::distance(&media, path, std::path::Path::new(img));
        if got == Some(old as u32) {
            same += 1;
        }
        if got.is_some_and(|d| d <= funsync_server::posters::MATCH_THRESHOLD) {
            kept += 1;
        } else {
            println!("would remove: {path} (old {old}, new {got:?})");
        }
        if got != Some(old as u32) {
            // The same poster file against the old app's own frame hashes
            // (`preview-match-cache.json`): tells the image side from the video side.
            let old_frames: Vec<u64> = cache["entries"][path.as_str()]["hashes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|h| u64::from_str_radix(h["hash"].as_str()?, 16).ok())
                .collect();
            let image = funsync_server::posters::image_hashes(&media, std::path::Path::new(img));
            let against_old = funsync_server::posters::closest(&image, &old_frames);
            println!(
                "distance: old {old} new {got:?}, file vs the old frames {against_old:?} {} {}",
                e["source"], e["imageUrl"]
            );
        }
    }
    println!("{kept} / {total} kept, {same} with the same distance");
}
