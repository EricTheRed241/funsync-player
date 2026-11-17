//! `--fetch`: download the scenes swiped right in the deck
//! (`<userData>/vrb-scout-swipes.json`) that aren't fetched or in the library
//! yet, best score first, within the library cap (`libraryMaxVideos`) and
//! `--limit`:
//!
//! - the script: the public toy script (no login), checked to be a
//!   funscript, saved first (it's small; a scene whose script fails isn't
//!   worth its video);
//! - the video: the member's signed link, read from the scene page right
//!   before the download (`browser::Members`, `nuxt::scene_links`), the best
//!   within the VR caps (`nuxt::choose`); curl into `<library
//!   parent>/_staging` as a `.part` (deleted when the download fails), then
//!   moved into the scene folder.
//!
//! Scenes go to `<Other>/<first performer>/<Performers> - <Title>
//! (<Site>)_<projection>/` (`slr_scout::fetch::stem` and `artist_folder`; the
//! `_180x180_3dh` projection suffix is what the headset reads), with a
//! `vrb.json` sidecar, then get generated twist / roll / pitch
//! (`scripts/upgrade-multi-axis.mjs`; hand-made axes win). `fetchedAt` is set
//! in vrb-scout.json; at the end the app is asked to rescan. `--dry-run`
//! signs in and reads the links but downloads nothing.
//!
//! A refused login on a site the account is a member of (`Site::member`)
//! stops the run (exit 3); on another site (VRConk) its scenes are skipped as
//! needing a membership, and so are they when the site signs in but shows
//! no download links.

use crate::browser::{LoginError, Members};
use crate::library::SIDECAR;
use crate::nuxt::{self, Link};
use crate::sites::{self, Site};
use crate::store::{self, Scene};
use serde_json::{Value, json};
use slr_scout::fetch::{artist_folder, stem};
use slr_scout::http::{Http, Request};
use slr_scout::library;
use slr_scout::prefs::Prefs;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use video_tags::timefmt;

pub const SWIPES_FILE: &str = "vrb-scout-swipes.json";
/// The library source scenes go to.
pub const FOLDER: &str = "Other";

/// The scene ids swiped right (`{version, swipes: {"<id>": {dir, at}}}`).
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

/// The scenes to download: swiped right, of the chosen sites, neither
/// fetched nor in the library, best score first (ties: newest first).
pub fn fetch_queue<'a>(scenes: &'a [Scene], right: &[String], sites: &[&Site]) -> Vec<&'a Scene> {
    let mut queue: Vec<&Scene> = scenes
        .iter()
        .filter(|s| {
            right.contains(&s.id)
                && s.fetched_at.is_none()
                && !s.in_library
                && sites.iter().any(|site| site.key == s.site)
        })
        .collect();
    queue.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(b.date.cmp(&a.date))
            .then(a.id.cmp(&b.id))
    });
    queue
}

/// The scene's folder and file stem: `<Performers> - <Title>
/// (<Site>)_<projection>`.
pub fn scene_name(s: &Scene, site: &Site, projection: &str) -> (String, String) {
    let base = stem(&s.actors, &s.title, Some(site.name));
    let name = if projection.is_empty() {
        base
    } else {
        format!("{base}_{projection}")
    };
    (artist_folder(&s.actors, Some(site.name)), name)
}

/// A site's sign-in state for this run.
enum Access {
    In,
    /// Its scenes are skipped with this reason.
    Skip(String),
}

/// What a scene's step ended in.
enum Stop {
    /// Skip this scene (reported).
    Skip(String),
    /// Stop the run with this exit code.
    Halt(String, i32),
}

/// A scene ready to download.
struct Planned {
    dir: PathBuf,
    stem: String,
    link: Link,
    script: String,
}

pub struct Fetch<'a> {
    pub http: &'a dyn Http,
    pub members: &'a mut dyn Members,
    pub user_data: &'a Path,
    pub prefs: &'a Prefs,
    pub sites: Vec<&'static Site>,
    pub dry_run: bool,
    /// Scenes to download this run at most.
    pub limit: Option<usize>,
    /// Canned network (tests): no upgrade run, no rescan.
    pub fake: bool,
}

fn io(e: std::io::Error) -> String {
    format!("output: {e}")
}

fn gb(bytes: Option<u64>) -> String {
    bytes.map_or("size unknown".into(), |b| {
        format!("{:.2} GB", b as f64 / 1_073_741_824.0)
    })
}

/// Rename, or copy and remove across volumes.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to)?;
    fs::remove_file(from)
}

impl Fetch<'_> {
    /// Run it; returns the exit code.
    pub fn run(&mut self, out: &mut dyn Write) -> Result<i32, String> {
        let file_path = store::file_path(self.user_data);
        let mut file = store::read(&file_path)?.ok_or_else(|| {
            format!(
                "{} doesn't exist yet; run npm run vrb:scout first",
                file_path.display()
            )
        })?;
        let right = right_swipes(self.user_data)?;
        let queue: Vec<Scene> = fetch_queue(&file.scenes, &right, &self.sites)
            .into_iter()
            .cloned()
            .collect();
        let missing = right
            .iter()
            .filter(|id| !file.scenes.iter().any(|s| &s.id == *id))
            .count();
        let done = file
            .scenes
            .iter()
            .filter(|s| {
                right.contains(&s.id)
                    && (s.fetched_at.is_some() || s.in_library)
                    && self.sites.iter().any(|site| site.key == s.site)
            })
            .count();
        writeln!(
            out,
            "{} scene(s) swiped right: {} to fetch, {done} fetched or in the library{}.",
            right.len(),
            queue.len(),
            if missing > 0 {
                format!(", {missing} no longer in vrb-scout.json")
            } else {
                String::new()
            }
        )
        .map_err(io)?;
        if queue.is_empty() {
            return Ok(0);
        }
        let config = video_tags::config::read_json(self.user_data)?;
        let user = library::from_json(&config);
        let (other, is_source) = library::download_folder(&user.sources, FOLDER)
            .ok_or("config.json lists no library source to put the scenes beside")?;
        if !is_source {
            writeln!(
                out,
                "Warning: {} isn't a library source; add it in the app to see these scenes.",
                other.display()
            )
            .map_err(io)?;
        }
        let staging = other
            .parent()
            .map_or_else(|| other.join("_staging"), |p| p.join("_staging"));
        let cap = self.prefs.library_max_videos;
        let mut held = library::video_count(&user.sources);
        let mut access: HashMap<&'static str, Access> = HashMap::new();
        let (mut fetched, mut failed, mut skipped, mut code) = (0, 0, 0, 0);
        let mut taken = 0;
        for scene in &queue {
            if self.limit.is_some_and(|n| taken >= n) {
                writeln!(
                    out,
                    "Stopped at --limit {}; the rest waits for the next run.",
                    taken
                )
                .map_err(io)?;
                code = code.max(2);
                break;
            }
            if held >= cap {
                writeln!(
                    out,
                    "Library holds {held} videos, the cap is {cap} (libraryMaxVideos): nothing more downloaded. Delete scenes or raise the cap."
                )
                .map_err(io)?;
                code = code.max(2);
                break;
            }
            let Some(site) = sites::by_key(&scene.site) else {
                continue;
            };
            writeln!(out, "{} #{} {}", site.name, scene.id, scene.title).map_err(io)?;
            let planned = match self.plan(scene, site, &mut access, &other) {
                Ok(p) => p,
                Err(Stop::Skip(why)) => {
                    writeln!(out, "  skipped: {why}").map_err(io)?;
                    skipped += 1;
                    continue;
                }
                Err(Stop::Halt(why, c)) => {
                    writeln!(out, "  stopped: {why}").map_err(io)?;
                    code = c;
                    break;
                }
            };
            taken += 1;
            writeln!(
                out,
                "  video: {} {} {} {} → {}",
                planned.link.quality,
                planned.link.projection,
                planned
                    .link
                    .size
                    .clone()
                    .unwrap_or_else(|| gb(planned.link.bytes)),
                nuxt::redact(&planned.link.url),
                planned.dir.join(format!("{}.mp4", planned.stem)).display()
            )
            .map_err(io)?;
            writeln!(out, "  script: {}", planned.script).map_err(io)?;
            held += 1;
            if self.dry_run {
                continue;
            }
            match self.download(&planned, &staging, out) {
                Ok(()) => {
                    let at = timefmt::now_iso();
                    write_sidecar(scene, site, &planned, &at)?;
                    if let Some(s) = file
                        .scenes
                        .iter_mut()
                        .find(|s| s.site == scene.site && s.id == scene.id)
                    {
                        s.fetched_at = Some(at);
                    }
                    store::write(self.user_data, &file)?;
                    self.upgrade(&planned.dir, out)?;
                    fetched += 1;
                    writeln!(out, "  ✓ fetched").map_err(io)?;
                }
                Err(why) => {
                    writeln!(out, "  ✗ {why}").map_err(io)?;
                    failed += 1;
                }
            }
        }
        if self.dry_run {
            writeln!(out, "Dry run: nothing downloaded.").map_err(io)?;
        } else {
            writeln!(
                out,
                "Fetched {fetched}, {failed} failed, {skipped} skipped."
            )
            .map_err(io)?;
        }
        if fetched > 0 {
            self.rescan(&config, out)?;
        }
        Ok(if code == 0 && failed > 0 { 1 } else { code })
    }

    /// Sign in (once per site), read the scene's links, pick one.
    fn plan(
        &mut self,
        scene: &Scene,
        site: &'static Site,
        access: &mut HashMap<&'static str, Access>,
        other: &Path,
    ) -> Result<Planned, Stop> {
        let membership = format!("needs a {} membership", site.name);
        if !access.contains_key(site.key) {
            let state = match self.members.login(site) {
                Ok(()) => Access::In,
                Err(LoginError::Refused(m)) if site.member => {
                    return Err(Stop::Halt(format!("not logged in: {m}"), 3));
                }
                Err(LoginError::Failed(m)) if site.member => {
                    return Err(Stop::Halt(m, 1));
                }
                Err(e) => Access::Skip(format!("{membership} ({})", e.message())),
            };
            access.insert(site.key, state);
        }
        if let Some(Access::Skip(why)) = access.get(site.key) {
            return Err(Stop::Skip(why.clone()));
        }
        let script = scene
            .script
            .as_ref()
            .map(|s| s.url.clone())
            .ok_or_else(|| Stop::Skip("no toy script".into()))?;
        let data = self
            .members
            .nuxt_data(site, &scene.slug)
            .map_err(Stop::Skip)?
            .unwrap_or(Value::Null);
        let links = nuxt::scene_links(site, &data, &scene.slug, scene.short_name.as_deref());
        if links.is_empty() {
            if site.member {
                return Err(Stop::Skip(format!(
                    "no download links on the scene page (signed out, or {} doesn't let this account download it)",
                    site.name
                )));
            }
            // Signed in, but the site shows no downloads: not a member.
            access.insert(site.key, Access::Skip(membership.clone()));
            return Err(Stop::Skip(membership));
        }
        let caps = &self.prefs.video_caps;
        let link = nuxt::choose(&links, scene.fps, caps)
            .cloned()
            .ok_or_else(|| {
                let offered: Vec<&str> = links.iter().map(|l| l.quality.as_str()).collect();
                Stop::Skip(format!(
                    "nothing within the VR caps (≤ {}K, ≤ {} fps); offered {}",
                    nuxt::max_k(caps),
                    caps.vr_max_fps,
                    offered.join(", ")
                ))
            })?;
        let (artist, stem) = scene_name(scene, site, &link.projection);
        Ok(Planned {
            dir: other.join(artist).join(&stem),
            stem,
            link,
            script,
        })
    }

    /// Script and video into `_staging`, then both into the scene folder, so
    /// a failed download leaves no half scene in the library (one would count
    /// as in the library at the next scan).
    fn download(&mut self, p: &Planned, staging: &Path, out: &mut dyn Write) -> Result<(), String> {
        let script_dir = staging.join(format!("{}.script", p.stem));
        let result = self.download_into(p, staging, &script_dir, out);
        let _ = fs::remove_dir_all(&script_dir);
        result
    }

    fn download_into(
        &mut self,
        p: &Planned,
        staging: &Path,
        script_dir: &Path,
        out: &mut dyn Write,
    ) -> Result<(), String> {
        fs::create_dir_all(script_dir).map_err(|e| format!("staging: {e}"))?;
        let script_part = staging.join(format!("{}.funscript.part", p.stem));
        let got = self
            .http
            .download(&Request::get(&p.script), &script_part)
            .map_err(|e| format!("script download: {e}"));
        let placed = got.and_then(|()| {
            slr_scout::fetch::place_script(&script_part, script_dir, &p.stem, staging)
                .map_err(|e| e.replace("SLR's", "the toy"))
        });
        let _ = fs::remove_file(&script_part);
        placed?;
        let video = p.dir.join(format!("{}.mp4", p.stem));
        let have = fs::metadata(&video).map(|m| m.len()).ok();
        let part = staging.join(format!("{}.mp4.part", p.stem));
        if have.is_some() && have == p.link.bytes {
            let _ = writeln!(out, "  video already there");
        } else {
            let _ = writeln!(out, "  downloading {} ...", gb(p.link.bytes));
            if let Err(e) = self.http.download(&Request::get(&p.link.url), &part) {
                let _ = fs::remove_file(&part);
                // curl's message can quote the URL: keep the token out of logs.
                return Err(format!(
                    "video download: {}",
                    e.replace(&p.link.url, &nuxt::redact(&p.link.url))
                ));
            }
        }
        fs::create_dir_all(&p.dir).map_err(|e| format!("scene folder: {e}"))?;
        if part.exists() {
            move_file(&part, &video).map_err(|e| format!("moving the video: {e}"))?;
        }
        let script = format!("{}.funscript", p.stem);
        move_file(&script_dir.join(&script), &p.dir.join(&script))
            .map_err(|e| format!("moving the script: {e}"))
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
                "The app wasn't asked to rescan ({e}); it picks the scenes up at its next scan."
            )
            .map_err(io),
        }
    }
}

fn write_sidecar(scene: &Scene, site: &Site, p: &Planned, at: &str) -> Result<(), String> {
    let v = json!({
        "site": site.key,
        "id": scene.id,
        "slug": scene.slug,
        "url": scene.url,
        "title": scene.title,
        "studio": site.name,
        "performers": scene.actors,
        "file": format!("{}.mp4", p.stem),
        "script": p.script,
        "video": {"quality": p.link.quality, "projection": p.link.projection, "file": p.link.file, "size": p.link.bytes},
        "fetchedAt": at,
    });
    let path = p.dir.join(SIDECAR);
    let text = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    fs::write(&path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::scene;

    #[test]
    fn reads_right_swipes() {
        let dir = std::env::temp_dir().join(format!("vrb-scout-swipes-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(right_swipes(&dir).unwrap().is_empty());
        fs::write(
            dir.join(SWIPES_FILE),
            r#"{"version": 1, "swipes": {"6b": {"dir": "right", "at": "x"},
                "6a": {"dir": "right"}, "7": {"dir": "left"}}}"#,
        )
        .unwrap();
        assert_eq!(right_swipes(&dir).unwrap(), ["6a", "6b"]);
        fs::write(dir.join(SWIPES_FILE), "nope").unwrap();
        assert!(right_swipes(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn queues_best_first_and_skips_what_is_there() {
        let s = |id: &str, site: &str, score: f64| {
            let mut s = scene(id);
            s.site = site.into();
            s.score = score;
            s
        };
        let mut fetched = s("f", "vrbangers", 50.0);
        fetched.fetched_at = Some("x".into());
        let mut there = s("l", "vrbangers", 40.0);
        there.in_library = true;
        let scenes = [
            s("a", "vrbangers", 3.0),
            s("b", "vrconk", 9.0),
            s("c", "vrbangers", 20.0),
            s("unswiped", "vrbangers", 30.0),
            fetched,
            there,
        ];
        let right: Vec<String> = ["a", "b", "c", "f", "l"].map(String::from).to_vec();
        let all: Vec<&Site> = sites::SITES.iter().collect();
        let ids: Vec<&str> = fetch_queue(&scenes, &right, &all)
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(ids, ["c", "b", "a"]);
        let only = [sites::by_key("vrbangers").unwrap()];
        let ids: Vec<&str> = fetch_queue(&scenes, &right, &only)
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(ids, ["c", "a"]);
    }

    #[test]
    fn names_scenes_for_the_headset() {
        let site = sites::by_key("vrbangers").unwrap();
        let mut s = scene("a");
        s.actors = vec!["Angel Youngs".into(), "Scarlette Moon".into()];
        s.title = "Wet & Wild: In Malibu?".into();
        assert_eq!(
            scene_name(&s, site, "180x180_3dh"),
            (
                "Angel Youngs".to_string(),
                "Angel Youngs, Scarlette Moon - Wet & Wild In Malibu (VRBangers)_180x180_3dh"
                    .to_string()
            )
        );
        s.actors.clear();
        let conk = sites::by_key("vrconk").unwrap();
        assert_eq!(
            scene_name(&s, conk, ""),
            (
                "VRConk".to_string(),
                "VRConk - Wet & Wild In Malibu (VRConk)".to_string()
            )
        );
    }
}
