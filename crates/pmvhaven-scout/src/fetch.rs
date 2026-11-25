//! `--fetch`: download the videos swiped right on the swipe page
//! (`<userData>/pmvhaven-scout-swipes.json`) that aren't fetched or in the
//! library yet, best score first. PMVHaven's files are public: the scripts
//! (the picked one and its axis companions) and then the mp4 go into
//! `<library parent>/_staging` as `.part` files (deleted when a download
//! fails, never resumed), then into `<Other>/<artist>/<Artist> - <Title>
//! (PMVHaven)<VR suffix>/` with a `pmvhaven.json` sidecar. Single-axis
//! scenes then get generated twist / roll / pitch
//! (`scripts/upgrade-multi-axis.mjs`; hand-made axes win). The library cap
//! (`libraryMaxVideos`) is checked before every video, and so is room on the
//! library drive (`slr_scout::disk`: `swipeDeck.minFreeGb` must stay free
//! after the download; a video that doesn't fit is left for later, exit 2,
//! and the next one tried); `fetchedAt` is set in pmvhaven-scout.json, and
//! at the end the app is asked to rescan. `--dry-run` prints the plan (and
//! the disk check) only.

use crate::library;
use crate::store::{self, Scene};
use serde_json::{Value, json};
use slr_scout::disk;
use slr_scout::fetch::{artist_folder, move_file, stem};
use slr_scout::http::{Http, Pacer, Request};
use slr_scout::library as slr_library;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use video_tags::timefmt;

pub const SWIPES_FILE: &str = "pmvhaven-scout-swipes.json";
/// The library source the videos go to.
pub const FOLDER: &str = "Other";
/// The studio part of a scene's name: `<Artist> - <Title> (PMVHaven)`.
pub const LABEL: &str = "PMVHaven";

/// The ids swiped right (`{version, swipes: {"<id>": {dir, at}}}`).
pub fn right_swipes(user_data: &Path) -> Result<Vec<String>, String> {
    let path = user_data.join(SWIPES_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let v: Value = serde_json::from_str(&raw)
        .map_err(|e| format!("{} is not a swipes file ({e})", path.display()))?;
    let mut ids: Vec<String> = v["swipes"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, s)| s["dir"] == "right")
        .map(|(id, _)| id.clone())
        .collect();
    ids.sort();
    Ok(ids)
}

/// The scenes to download: swiped right, not fetched, not in the library,
/// with a script; highest score first, then most likes.
pub fn queue<'a>(scenes: &'a [Scene], right: &[String]) -> Vec<&'a Scene> {
    let mut q: Vec<&Scene> = scenes
        .iter()
        .filter(|s| {
            right.contains(&s.id) && s.fetched_at.is_none() && !s.in_library && s.script.is_some()
        })
        .collect();
    q.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(b.likes.cmp(&a.likes))
            .then(a.id.cmp(&b.id))
    });
    q
}

/// Who the scene is filed under: the studio (original creator, else the
/// uploader), else `PMVHaven`.
pub fn artist(s: &Scene) -> String {
    s.studio
        .clone()
        .or_else(|| Some(s.uploader.clone()))
        .filter(|a| !a.trim().is_empty())
        .unwrap_or_else(|| LABEL.to_string())
}

/// `(artist folder, scene name)`: `<Artist> - <Title> (PMVHaven)` plus the
/// VR suffix, with slr-scout's naming rules (safe characters, STEM_BYTES).
pub fn names(s: &Scene) -> (String, String) {
    let a = vec![artist(s)];
    (
        artist_folder(&a, Some(LABEL)),
        format!("{}{}", stem(&a, &s.title, Some(LABEL)), s.vr_suffix),
    )
}

/// The file extension of the video URL (`mp4` unless it says otherwise).
fn video_ext(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((_, ext)) if video_tags::library::is_video_name(name) => ext.to_ascii_lowercase(),
        _ => "mp4".into(),
    }
}

/// A downloaded file is a funscript (JSON with `actions`).
fn check_funscript(path: &Path) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("script: {e}"))?;
    let v: Value =
        serde_json::from_slice(&bytes).map_err(|_| "the script isn't a funscript".to_string())?;
    if v["actions"].is_array() {
        Ok(())
    } else {
        Err("the script isn't a funscript (no actions)".into())
    }
}

pub struct Fetch<'a> {
    pub http: &'a dyn Http,
    pub user_data: &'a Path,
    pub library_max_videos: usize,
    pub dry_run: bool,
    pub limit: Option<usize>,
    /// The bucket (scripts, videos).
    pub files: Pacer,
    /// Canned network (tests): no upgrade run, no rescan, the free space from
    /// `slr_scout::disk::FAKE_FREE_ENV`.
    pub fake: bool,
}

struct Planned {
    dir: PathBuf,
    name: String,
    video: String,
}

fn gb(bytes: Option<u64>) -> String {
    bytes.map_or("size unknown".into(), |b| {
        format!("{:.2} GB", b as f64 / 1_073_741_824.0)
    })
}

fn io(e: std::io::Error) -> String {
    format!("output: {e}")
}

impl Fetch<'_> {
    pub fn run(&mut self, out: &mut dyn Write) -> Result<i32, String> {
        let mut file = store::read(self.user_data)?.ok_or_else(|| {
            format!(
                "{} doesn't exist yet; run npm run pmvhaven:scout first",
                store::file_path(self.user_data).display()
            )
        })?;
        let right = right_swipes(self.user_data)?;
        let config = video_tags::config::read_json(self.user_data)?;
        let user = slr_library::from_json(&config);
        // The library as it is now: the scan's inLibrary may be older.
        let known = library::known(&config, &user.sources);
        for s in &mut file.scenes {
            s.in_library |= known.has(&s.id, &s.title, s.duration_s);
        }
        let missing = right.iter().filter(|id| file.scene(id).is_none()).count();
        let queue: Vec<String> = queue(&file.scenes, &right)
            .into_iter()
            .map(|s| s.id.clone())
            .collect();
        writeln!(
            out,
            "{} video(s) swiped right: {} to fetch, {} fetched or in the library{}.",
            right.len(),
            queue.len(),
            right.len() - missing - queue.len(),
            if missing > 0 {
                format!(", {missing} no longer in {}", store::FILE_NAME)
            } else {
                String::new()
            }
        )
        .map_err(io)?;
        if queue.is_empty() {
            return Ok(0);
        }
        let (other, is_source) = slr_library::download_folder(&user.sources, FOLDER)
            .ok_or("config.json lists no library source to put PMVHaven videos beside")?;
        if !is_source {
            writeln!(
                out,
                "Warning: {} isn't a library source; add it in the app to see these videos.",
                other.display()
            )
            .map_err(io)?;
        }
        let staging = other
            .parent()
            .map_or_else(|| other.join("_staging"), |p| p.join("_staging"));
        let cap = self.library_max_videos;
        let mut held = slr_library::video_count(&user.sources);
        let (mut fetched, mut failed, mut skipped, mut code) = (0, 0, 0, 0);
        for (planned, id) in queue.iter().enumerate() {
            let scene = file.scene(id).cloned().ok_or("scene vanished")?;
            if held >= cap {
                writeln!(
                    out,
                    "Library holds {held} videos, the cap is {cap} (libraryMaxVideos): nothing more downloaded. Delete scenes or raise the cap."
                )
                .map_err(io)?;
                code = 2;
                break;
            }
            if self.limit.is_some_and(|l| planned >= l) {
                writeln!(
                    out,
                    "--limit {planned} reached; {} left for the next run.",
                    queue.len() - planned
                )
                .map_err(io)?;
                code = 2;
                break;
            }
            let (folder, name) = names(&scene);
            let p = Planned {
                dir: other.join(folder).join(&name),
                video: format!("{name}.{}", video_ext(&scene.video_url)),
                name,
            };
            let script = scene.script.as_ref().ok_or("scene has no script")?;
            writeln!(out, "{id} {}", scene.title).map_err(io)?;
            writeln!(
                out,
                "  video: {} → {}",
                gb(scene.file_size),
                p.dir.join(&p.video).display()
            )
            .map_err(io)?;
            writeln!(
                out,
                "  script: {} by {}{}{}",
                script.source,
                script.by,
                if script.generated { " (generated)" } else { "" },
                if script.axes.is_empty() {
                    String::new()
                } else {
                    format!(" + {}", script.axes.join(", "))
                }
            )
            .map_err(io)?;
            // Room on the drive for the video (none needed when it's there already).
            let there = scene.file_size.is_some()
                && fs::metadata(p.dir.join(&p.video)).map(|m| m.len()).ok() == scene.file_size;
            if !there
                && !disk::gate(&other, scene.file_size, self.fake, self.dry_run, out).map_err(io)?
            {
                skipped += 1;
                code = 2;
                continue;
            }
            held += 1;
            if self.dry_run {
                continue;
            }
            match self.download(&scene, &p, &staging, out) {
                Ok(()) => {
                    let at = timefmt::now_iso();
                    write_sidecar(&scene, &p, &at)?;
                    if let Some(s) = file.scenes.iter_mut().find(|s| s.id == *id) {
                        s.fetched_at = Some(at);
                    }
                    store::write(self.user_data, &file)?;
                    if script.axes.is_empty() {
                        self.upgrade(&p.dir, out)?;
                    }
                    fetched += 1;
                    writeln!(out, "  ✓ fetched").map_err(io)?;
                }
                Err(why) => {
                    held -= 1;
                    failed += 1;
                    writeln!(out, "  ✗ {why}").map_err(io)?;
                }
            }
        }
        if self.dry_run {
            writeln!(out, "Dry run: nothing downloaded.").map_err(io)?;
        } else {
            writeln!(
                out,
                "Fetched {fetched}, {failed} failed{}.",
                if skipped > 0 {
                    format!(", {skipped} left for later (disk space)")
                } else {
                    String::new()
                }
            )
            .map_err(io)?;
        }
        if fetched > 0 {
            self.rescan(&config, out)?;
        }
        Ok(if failed > 0 { 1 } else { code })
    }

    /// One file into `staging/<part>`; the `.part` goes when it fails.
    fn get(&mut self, url: &str, staging: &Path, part: &str) -> Result<PathBuf, String> {
        fs::create_dir_all(staging).map_err(|e| format!("{}: {e}", staging.display()))?;
        let path = staging.join(part);
        let _ = fs::remove_file(&path);
        self.files.wait();
        match self.http.download(&Request::get(url), &path) {
            Ok(()) => Ok(path),
            Err(e) => {
                let _ = fs::remove_file(&path);
                Err(e)
            }
        }
    }

    fn download(
        &mut self,
        scene: &Scene,
        p: &Planned,
        staging: &Path,
        out: &mut dyn Write,
    ) -> Result<(), String> {
        let script = scene.script.as_ref().ok_or("no script")?;
        // Scripts first: small, and a video without one is no use.
        let mut scripts = vec![(None, script.url.clone())];
        scripts.extend(
            script
                .companions
                .iter()
                .map(|c| (Some(c.axis.clone()), c.url.clone())),
        );
        let mut parts: Vec<(PathBuf, String)> = Vec::new();
        let cleanup = |parts: &[(PathBuf, String)]| {
            for (part, _) in parts {
                let _ = fs::remove_file(part);
            }
        };
        for (axis, url) in scripts {
            let file = match &axis {
                Some(a) => format!("{}.{a}.funscript", p.name),
                None => format!("{}.funscript", p.name),
            };
            let got = self
                .get(&url, staging, &format!("{file}.part"))
                .map_err(|e| format!("script download: {e}"))
                .and_then(|part| match check_funscript(&part) {
                    Ok(()) => Ok(part),
                    Err(e) => {
                        let _ = fs::remove_file(&part);
                        Err(e)
                    }
                });
            match got {
                Ok(part) => parts.push((part, file)),
                Err(e) if axis.is_some() => {
                    // A companion is a bonus; the stroke script is the scene.
                    let _ = writeln!(out, "  {} skipped: {e}", axis.unwrap_or_default());
                }
                Err(e) => {
                    cleanup(&parts);
                    return Err(e);
                }
            }
        }
        let video = p.dir.join(&p.video);
        let have = fs::metadata(&video).map(|m| m.len()).ok();
        let video_part = if have.is_some() && have == scene.file_size {
            let _ = writeln!(out, "  video already there");
            None
        } else {
            let _ = writeln!(out, "  downloading {} ...", gb(scene.file_size));
            match self.get(&scene.video_url, staging, &format!("{}.part", p.video)) {
                Ok(part) => Some(part),
                Err(e) => {
                    cleanup(&parts);
                    return Err(format!("video download: {e}"));
                }
            }
        };
        fs::create_dir_all(&p.dir).map_err(|e| format!("{}: {e}", p.dir.display()))?;
        if let Some(part) = video_part {
            move_file(&part, &video).map_err(|e| format!("moving the video: {e}"))?;
        }
        for (part, name) in &parts {
            move_file(part, &p.dir.join(name)).map_err(|e| format!("moving {name}: {e}"))?;
        }
        Ok(())
    }

    /// Generated twist / roll / pitch for the new scene (hand-made axes win).
    fn upgrade(&self, dir: &Path, out: &mut dyn Write) -> Result<(), String> {
        if self.fake {
            return Ok(());
        }
        // The repo root is the working directory under `npm run` and in the jobs image;
        // else this crate's checkout.
        let local = Path::new("scripts/upgrade-multi-axis.mjs");
        let script = if local.is_file() {
            local.to_path_buf()
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/upgrade-multi-axis.mjs")
        };
        let status = Command::new("node")
            .arg(&script)
            .arg(dir)
            .stdout(std::process::Stdio::null())
            .status();
        match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => writeln!(
                out,
                "  generated axes failed ({s}); run npm run upgrade:axes"
            )
            .map_err(io),
            Err(e) => writeln!(out, "  generated axes not run: {e}").map_err(io),
        }
    }

    fn rescan(&self, config: &Value, out: &mut dyn Write) -> Result<(), String> {
        if self.fake {
            return writeln!(out, "Rescan not requested (canned network).").map_err(io);
        }
        let port = video_tags::config::from_json(self.user_data, config)
            .map(|c| c.backend_port)
            .unwrap_or(video_tags::config::DEFAULT_BACKEND_PORT);
        let base = format!("http://127.0.0.1:{port}");
        match video_tags::rescan::request_rescan(&base, video_tags::rescan::TIMEOUT) {
            Ok(_) => writeln!(out, "Asked the app to rescan ({base}).").map_err(io),
            Err(e) => writeln!(
                out,
                "The app wasn't asked to rescan ({e}); it picks the videos up at its next scan."
            )
            .map_err(io),
        }
    }
}

fn write_sidecar(scene: &Scene, p: &Planned, at: &str) -> Result<(), String> {
    let script = scene.script.as_ref();
    let v = json!({
        "site": "pmvhaven",
        "id": scene.id,
        "url": scene.url,
        "title": scene.title,
        "studio": scene.studio,
        "uploader": scene.uploader,
        "performers": scene.actors,
        "tags": scene.tags,
        "vr": scene.vr,
        "durationS": scene.duration_s,
        "file": p.video,
        "videoUrl": scene.video_url,
        "script": script.map(|s| json!({
            "url": s.url, "source": s.source, "by": s.by,
            "generated": s.generated, "axes": s.axes,
        })),
        "fetchedAt": at,
    });
    let path = p.dir.join(library::SIDECAR);
    let text = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    fs::write(&path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

/// Carry `fetchedAt` over from the previous file.
pub fn keep_fetched(previous: Option<&store::ScoutFile>, scenes: &mut [Scene]) {
    if let Some(p) = previous {
        for s in scenes {
            if let Some(old) = p.scene(&s.id) {
                s.fetched_at = old.fetched_at.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::Script;

    pub fn scene(id: &str, score: f64) -> Scene {
        Scene {
            id: id.into(),
            site: "pmvhaven".into(),
            url: format!("https://pmvhaven.com/video/{id}"),
            title: "Shrooms: Q / HotShot".into(),
            studio: Some("Vassago".into()),
            actors: vec![],
            date: None,
            duration_s: Some(293),
            thumbnail: None,
            images: vec![],
            vr: false,
            vr_suffix: String::new(),
            fps: None,
            passthrough: false,
            creampie: false,
            pmv: true,
            compilation: false,
            likes: 1,
            views: 1,
            tags: vec![],
            tag_scores: Default::default(),
            favourite: None,
            script: Some(Script {
                url: "https://s3/s.funscript".into(),
                axes: vec![],
                source: "uploader".into(),
                by: "Vassago".into(),
                generated: false,
                companions: vec![],
            }),
            handmade: vec![],
            in_library: false,
            fetched_at: None,
            video_url: "https://s3/v.mp4".into(),
            file_size: None,
            uploader: "Scruffy6".into(),
            score,
            score_parts: Default::default(),
        }
    }

    #[test]
    fn names_scenes() {
        let s = scene("a", 1.0);
        assert_eq!(
            names(&s),
            (
                "Vassago".to_string(),
                "Vassago - Shrooms Q HotShot (PMVHaven)".to_string()
            )
        );
        let mut s = scene("a", 1.0);
        s.studio = None;
        s.vr_suffix = "_180_LR".into();
        assert_eq!(
            names(&s).1,
            "Scruffy6 - Shrooms Q HotShot (PMVHaven)_180_LR"
        );
        s.uploader = String::new();
        assert_eq!(names(&s).0, "PMVHaven");
        assert_eq!(video_ext("https://s3/a.b/v.MKV?x=1"), "mkv");
        assert_eq!(video_ext("https://s3/v"), "mp4");
    }

    #[test]
    fn queues_right_swipes_best_first() {
        let mut fetched = scene("f", 50.0);
        fetched.fetched_at = Some("x".into());
        let mut held = scene("h", 40.0);
        held.in_library = true;
        let mut bare = scene("n", 30.0);
        bare.script = None;
        let scenes = [
            scene("a", 1.0),
            scene("b", 9.0),
            scene("c", 20.0),
            fetched,
            held,
            bare,
        ];
        let right: Vec<String> = ["a", "b", "f", "h", "n", "gone"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let q: Vec<&str> = queue(&scenes, &right)
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(q, ["b", "a"]);
    }

    #[test]
    fn reads_right_swipes() {
        let dir = std::env::temp_dir().join(format!("pmvhaven-swipes-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(right_swipes(&dir).unwrap().is_empty());
        fs::write(
            dir.join(SWIPES_FILE),
            r#"{"version": 1, "swipes": {"6afc": {"dir": "right", "at": "x"},
                "68f9": {"dir": "right"}, "7": {"dir": "left"}}}"#,
        )
        .unwrap();
        assert_eq!(right_swipes(&dir).unwrap(), ["68f9", "6afc"]);
        fs::write(dir.join(SWIPES_FILE), "nope").unwrap();
        assert!(right_swipes(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
