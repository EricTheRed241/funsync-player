//! Deletes from disk, only while this server owns the library
//! (`data::owns_library()`): triage (a rating at or below
//! `ratings.deleteAtOrBelow`, not a favourite) and duplicates. Port of
//! `electron/rejected-videos.js` + `renderer/js/reject-rule.js`: the video
//! with its scripts, axis companions and subtitles; a scene folder left with
//! only its scout sidecar goes too, and an emptied creator folder. EroScripts
//! / FapTap scenes are remembered in `<old userData>/eroscripts-scout-rejected.json`
//! so the scouts never fetch them again.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use unicode_normalization::UnicodeNormalization;

use crate::data::{Library, write_atomic};
use crate::scan::{AXIS_SUFFIXES, SUBTITLE_EXTS, VIDEO_EXTS, is_apple_double};

pub const REJECTED_FILE: &str = "eroscripts-scout-rejected.json";
const LEFTOVERS: [&str; 3] = ["eroscripts.json", "faptap.json", ".ds_store"];

fn fold(s: &str) -> String {
    s.nfc().collect::<String>().to_lowercase()
}

/// `new Date(ms).toISOString()`.
fn iso_ms(ms: u64) -> String {
    let (secs, millis) = (ms / 1000, ms % 1000);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// `ratingVerdict(entry) === 'delete'`.
pub fn rejected(lib: &Library, path: &str) -> bool {
    let (rating, fav) = crate::remote::rating_entry(lib, path);
    let at = crate::prefs::content_preferences()["ratings"]["deleteAtOrBelow"]
        .as_f64()
        .unwrap_or(0.0);
    rating > 0.0 && rating <= at && !fav
}

/// Add one entry to the rejection list, replacing one with the same key.
fn record_rejection(user_data: &Path, mut entry: Value) -> Result<(), String> {
    let file = user_data.join(REJECTED_FILE);
    let mut data: Value = fs::read(&file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .filter(|d: &Value| d["version"] == 1 && d["entries"].is_array())
        .unwrap_or_else(|| json!({"version": 1, "entries": []}));
    let key = match entry["faptapId"].as_str() {
        Some(id) => format!("faptap|{id}"),
        None => format!(
            "{}|{}",
            entry["topicId"],
            entry["video"].as_str().unwrap_or("")
        ),
    };
    entry["key"] = json!(key);
    let entries = data["entries"].as_array_mut().unwrap();
    entries.retain(|e| e["key"] != key.as_str());
    entries.push(entry);
    let mut text = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
    text.push('\n');
    write_atomic(&file, text.as_bytes()).map_err(|e| e.to_string())
}

/// A file and its AppleDouble `._` sidecar, in either Unicode form.
fn remove_file(dir: &Path, name: &str) {
    for n in [name.to_string(), format!("._{name}")] {
        for form in [n.clone(), n.nfc().collect()] {
            let _ = fs::remove_file(dir.join(form));
        }
    }
}

/// Remove `dir` when only `keep`-able leftovers remain.
fn remove_dir_if_only(dir: &Path, keep: impl Fn(&str) -> bool) -> bool {
    let Ok(rd) = fs::read_dir(dir) else {
        return false;
    };
    let names: Vec<String> = rd
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    if names.iter().any(|n| !is_apple_double(n) && !keep(n)) {
        return false;
    }
    for n in &names {
        remove_file(dir, n);
    }
    if fs::remove_dir(dir).is_err() {
        return false;
    }
    if let (Some(p), Some(n)) = (dir.parent(), dir.file_name()) {
        remove_file(p, &format!("._{}", n.to_string_lossy()));
    }
    true
}

/// `deleteRejectedVideo`: the deleted files; nothing outside a source.
pub fn delete_video(
    path: &str,
    sources: &[String],
    user_data: &Path,
    now_ms: u64,
    reason: &str,
) -> Result<Vec<PathBuf>, String> {
    let video = Path::new(path);
    let roots: Vec<String> = sources
        .iter()
        .map(|s| s.trim_end_matches('/').to_string())
        .collect();
    if !roots
        .iter()
        .any(|r| fold(path).starts_with(&format!("{}/", fold(r))))
    {
        return Err(format!("not inside a library source: {path}"));
    }
    let ext = video
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    if !VIDEO_EXTS.contains(&ext.as_str()) {
        return Err(format!("not a video: {path}"));
    }
    let (Some(dir), Some(name)) = (video.parent(), video.file_name()) else {
        return Err(format!("not a video: {path}"));
    };
    let name = name.to_string_lossy();
    let stem = &name[..name.len() - ext.len()];
    let mut wanted: HashSet<String> = [name.to_string(), format!("{stem}.funscript")]
        .into_iter()
        .chain(SUBTITLE_EXTS.iter().map(|e| format!("{stem}{e}")))
        .chain(AXIS_SUFFIXES.iter().flat_map(|a| {
            [
                format!("{stem}.{a}.funscript"),
                format!("{stem}.generated.{a}.funscript"),
            ]
        }))
        .collect();
    wanted = wanted.iter().map(|n| fold(n)).collect();
    let Ok(rd) = fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let hits: Vec<String> = rd
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !is_apple_double(n) && wanted.contains(&fold(n)))
        .collect();
    if !hits.iter().any(|n| fold(n) == fold(&name)) {
        return Ok(Vec::new());
    }
    let sidecar = |n: &str| -> Value {
        fs::read(dir.join(n))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null)
    };
    let at = iso_ms(now_ms);
    let es = sidecar("eroscripts.json");
    if !es["topicId"].is_null() && es["video"].as_str().is_some_and(|v| !v.is_empty()) {
        record_rejection(
            user_data,
            json!({"topicId": es["topicId"], "video": es["video"], "title": es.get("title").filter(|t| !t.is_null()),
                "url": es.get("url").filter(|t| !t.is_null()), "reason": reason, "path": path, "at": at}),
        )?;
    } else if es["topicId"].is_null() {
        let ft = sidecar("faptap.json");
        let id = match &ft["faptapId"] {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        };
        if let Some(id) = id {
            record_rejection(
                user_data,
                json!({"source": "faptap", "faptapId": id, "title": ft.get("title").filter(|t| !t.is_null()),
                    "url": ft.get("url").filter(|t| !t.is_null()), "reason": reason, "path": path, "at": at}),
            )?;
        }
    }
    let mut deleted = Vec::new();
    for n in &hits {
        remove_file(dir, n);
        deleted.push(dir.join(n));
    }
    let is_root = |d: &Path| roots.iter().any(|r| fold(r) == fold(&d.to_string_lossy()));
    if !is_root(dir)
        && remove_dir_if_only(dir, |n| LEFTOVERS.contains(&fold(n).as_str()))
        && let Some(parent) = dir.parent().filter(|p| !is_root(p))
    {
        remove_dir_if_only(parent, |n| fold(n) == ".ds_store");
    }
    Ok(deleted)
}

/// `forgetDeletedVideo`: out of every playlist, no rating, no association.
pub fn forget(lib: &mut Library, path: &str) {
    for p in lib.top_array_mut("playlists") {
        if let Some(v) = p["videoPaths"].as_array_mut() {
            v.retain(|x| x != path);
        }
    }
    for key in ["ratings", "associations"] {
        if let Some(m) = lib.lib_mut(key).as_object_mut() {
            m.remove(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletes_the_scene_and_remembers_it() {
        let root = tempfile::tempdir().unwrap();
        let ud = tempfile::tempdir().unwrap();
        let src = root.path().join("src");
        let scene = src.join("Artist/Scene");
        fs::create_dir_all(&scene).unwrap();
        for n in [
            "Scene.mp4",
            "Scene.funscript",
            "Scene.twist.funscript",
            "Scene.generated.roll.funscript",
            "Scene.srt",
            "._Scene.mp4",
        ] {
            fs::write(scene.join(n), b"x").unwrap();
        }
        fs::write(
            scene.join("eroscripts.json"),
            r#"{"topicId": 7, "video": "Scene.mp4", "title": "T"}"#,
        )
        .unwrap();
        let other = src.join("Artist/Other");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("Keep.mp4"), b"x").unwrap();
        let sources = vec![src.to_string_lossy().into_owned()];
        let path = scene.join("Scene.mp4").to_string_lossy().into_owned();
        let out = delete_video(&path, &sources, ud.path(), 0, "low-rating").unwrap();
        assert_eq!(out.len(), 5);
        assert!(!scene.exists(), "scene folder with only its sidecar goes");
        assert!(other.join("Keep.mp4").exists());
        let rej: Value =
            serde_json::from_slice(&fs::read(ud.path().join(REJECTED_FILE)).unwrap()).unwrap();
        assert_eq!(rej["entries"][0]["key"], "7|Scene.mp4");
        assert_eq!(rej["entries"][0]["reason"], "low-rating");
        // The emptied creator folder goes too.
        let keep = other.join("Keep.mp4").to_string_lossy().into_owned();
        delete_video(&keep, &sources, ud.path(), 0, "duplicate").unwrap();
        assert!(!src.join("Artist").exists() && src.exists());
        // Outside a source: refused.
        assert!(delete_video("/etc/x.mp4", &sources, ud.path(), 0, "low-rating").is_err());
    }

    #[test]
    fn iso_dates() {
        assert_eq!(iso_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_ms(1_763_290_000_123), "2025-11-16T10:46:40.123Z");
    }

    #[test]
    fn verdict_and_forget() {
        let mut l = Library::default();
        crate::edits::set_rating(&mut l, "/a", Some(2.0), None);
        assert!(rejected(&l, "/a"));
        crate::edits::set_rating(&mut l, "/a", None, Some(true));
        assert!(!rejected(&l, "/a"));
        crate::edits::edit_playlist(&mut l, "/a", None, Some("P"), true);
        forget(&mut l, "/a");
        assert!(l.lib()["ratings"].get("/a").is_none());
        assert_eq!(l.config["playlists"][0]["videoPaths"], json!([]));
    }
}
