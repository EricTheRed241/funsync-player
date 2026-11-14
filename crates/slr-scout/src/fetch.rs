//! `--fetch`: download the scenes swiped right on the swipe page's SLR tab
//! (`<userData>/slr-scout-swipes.json`) that aren't in the library yet, with
//! the user's SLR account:
//!
//! - the video: SLR's download options, `maxResolution` when it's within
//!   the caps (`videoCaps`), else `optimalResolution`, never the trailer;
//!   a fresh download URL each time, curl into `<library parent>/_staging`
//!   as a `.part`, then moved into the scene folder;
//! - the script, from `scriptSource`: the free EroScripts post's stroke
//!   script and its axis companions (forum attachments), else SLR's
//!   hand-crafted script, else its AI script (each SLR one only while its
//!   monthly quota lasts). A scene with no script to take isn't downloaded.
//!
//! Scenes go to `<Other>/<first performer>/<Performers> - <Title>
//! (<Studio>)<vrSuffix>/` with an `slr.json` sidecar, then get generated
//! twist / roll / pitch (`scripts/upgrade-multi-axis.mjs`; hand-made axes
//! win). The video quota is checked before every video and the library cap
//! (`libraryMaxVideos`) before every scene. `fetchedAt` is set in
//! slr-scout.json; at the end the app is asked to rescan. `--dry-run`
//! prints the plan and sends no download request.

use crate::forum::{self, Attachment};
use crate::http::{Http, NetError, Pacer};
use crate::library;
use crate::prefs::{Prefs, VideoCaps};
use crate::quota::{self, QuotaError};
use crate::slr::{self, API};
use crate::store::{self, Handmade, Quota, Scene, ScoutFile, ScriptSource};
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use video_tags::timefmt;

pub const SWIPES_FILE: &str = "slr-scout-swipes.json";
/// The library source SLR scenes go to.
pub const FOLDER: &str = "Other";
pub const SIDECAR: &str = "slr.json";
/// Scene stems stay under the 255-byte name limit with
/// `.generated.twist.funscript` (STEM_BYTES in eroscripts-scout-lib.mjs).
pub const STEM_BYTES: usize = 180;

/// The scene ids swiped right (`{version, swipes: {"<id>": {dir, at}}}`).
pub fn right_swipes(user_data: &Path) -> Result<Vec<u64>, String> {
    let path = user_data.join(SWIPES_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let v: Value = serde_json::from_str(&raw)
        .map_err(|e| format!("{} is not a swipes file ({e})", path.display()))?;
    let mut ids: Vec<u64> = v["swipes"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, s)| s["dir"] == "right")
        .filter_map(|(id, _)| id.parse().ok())
        .collect();
    ids.sort_unstable();
    Ok(ids)
}

/// The file-name suffix the headset reads the projection from
/// (`slrVrSuffix` in scripts/slr-host.mjs): `screenType` 6 `_MKX200`, 7
/// `_MKX220`, 8 `_VRCA220`, 10 `_RF52`, 9 / 11 `_FISHEYE190`; flat (3 or
/// `isFlatScreenType`) none; otherwise equirectangular, 360 for a sphere
/// (5) or a 360° view angle, else 180, laid out by `stereomode`: `_180_LR`,
/// `_180_TB`, `_MONO180`.
pub fn vr_suffix(d: &Value) -> String {
    let screen = d["screenType"].as_u64().unwrap_or(0);
    if d["isFlatScreenType"].as_bool() == Some(true) || screen == 3 {
        return String::new();
    }
    let fixed = match screen {
        6 => "_MKX200",
        7 => "_MKX220",
        8 => "_VRCA220",
        10 => "_RF52",
        9 | 11 => "_FISHEYE190",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.to_string();
    }
    let angle = if screen == 5 || d["viewAngle"].as_f64().is_some_and(|a| a >= 360.0) {
        360
    } else {
        180
    };
    let mode = d["stereomode"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if mode == "mono" {
        format!("_MONO{angle}")
    } else if mode.starts_with("tb") || mode.starts_with("ou") {
        format!("_{angle}_TB")
    } else {
        format!("_{angle}_LR")
    }
}

/// A name part safe on any volume: `/ \ : * ? " < > |` removed, whitespace
/// collapsed, trimmed.
pub fn safe_name(s: &str) -> String {
    let kept: String = s
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `<Performers> - <Title> (<Studio>)`: performers joined by `, ` (the
/// studio when there are none; `SexLikeReal` without a studio), dropped
/// from the end while the stem is over STEM_BYTES (one always stays), then
/// cut to STEM_BYTES on a character boundary without a trailing
/// ` -–—,;.(` or whitespace; an empty title is `Scene`
/// (`slrSceneName` in scripts/slr-host.mjs). The VR suffix goes after it.
pub fn stem(performers: &[String], title: &str, studio: Option<&str>) -> String {
    let title = Some(safe_name(title))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "Scene".into());
    let studio = studio.map(safe_name).filter(|s| !s.is_empty());
    let studio = studio.as_deref().unwrap_or("SexLikeReal");
    let mut who: Vec<String> = performers
        .iter()
        .map(|p| safe_name(p))
        .filter(|p| !p.is_empty())
        .collect();
    let make = |who: &[String]| {
        let names = if who.is_empty() {
            studio.to_string()
        } else {
            who.join(", ")
        };
        format!("{names} - {title} ({studio})")
    };
    let mut s = make(&who);
    while s.len() > STEM_BYTES && who.len() > 1 {
        who.pop();
        s = make(&who);
    }
    if s.len() > STEM_BYTES {
        let mut cut = STEM_BYTES;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        let trimmed = s
            .trim_end_matches(|c: char| {
                c.is_whitespace() || matches!(c, '-' | '–' | '—' | ',' | ';' | '.' | '(')
            })
            .len();
        s.truncate(trimmed);
    }
    s
}

/// The artist folder: the first performer, else the studio.
pub fn artist_folder(performers: &[String], studio: Option<&str>) -> String {
    performers
        .iter()
        .chain(studio.map(str::to_string).as_ref())
        .map(|n| safe_name(n))
        .find(|n| !n.is_empty())
        .unwrap_or_else(|| "SexLikeReal".into())
}

/// One of SLR's download options for a video.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoOption {
    /// `maxResolution`, `optimalResolution`, `original`, `trailer`.
    pub kind: String,
    pub height: Option<u64>,
    pub width: Option<u64>,
    pub size: Option<u64>,
    pub codec: Option<String>,
    /// Offered with the subscription (not only through a purchase).
    pub downloadable: bool,
}

/// The options of a `download-options` answer (`data.options`: a list of
/// `{type, …}`, or an object keyed by type).
pub fn parse_options(v: &Value) -> Vec<VideoOption> {
    let one = |kind: &str, o: &Value| VideoOption {
        kind: kind.to_string(),
        height: o["height"].as_u64(),
        width: o["width"].as_u64(),
        size: o["size"].as_u64(),
        codec: o["codec"].as_str().map(str::to_string),
        downloadable: o["isDownloadable"].as_bool() != Some(false)
            && o["isAvailableThroughPurchase"].as_bool() != Some(true),
    };
    let options = &v["data"]["options"];
    match options {
        Value::Array(list) => list
            .iter()
            .filter_map(|o| Some(one(o["type"].as_str()?, o)))
            .collect(),
        Value::Object(map) => map
            .iter()
            .filter(|(_, o)| o.is_object())
            .map(|(k, o)| one(k, o))
            .collect(),
        _ => Vec::new(),
    }
}

/// The option to download: `maxResolution` when its height is within the
/// cap (VR or flat), else `optimalResolution` within it; never the trailer
/// or the original.
pub fn choose(options: &[VideoOption], vr: bool, caps: &VideoCaps) -> Result<VideoOption, String> {
    let cap = if vr {
        caps.vr_max_height
    } else {
        caps.flat_max_height
    };
    let get = |kind: &str| options.iter().find(|o| o.kind == kind && o.downloadable);
    if let Some(o) = get("maxResolution").filter(|o| o.height.is_some_and(|h| h <= cap)) {
        return Ok(o.clone());
    }
    match get("optimalResolution") {
        Some(o) if o.height.is_none_or(|h| h <= cap) => Ok(o.clone()),
        Some(o) => Err(format!(
            "no download within the {cap}p cap (optimal is {}p)",
            o.height.unwrap_or(0)
        )),
        None => Err("SLR offers no maxResolution / optimalResolution download".into()),
    }
}

/// Where the script comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptPlan {
    /// A free EroScripts post's stroke script and its axis companions.
    Forum {
        topic_id: u64,
        main: Attachment,
        companions: Vec<(&'static str, Attachment)>,
    },
    /// An SLR script by id.
    Slr { script_id: u64, ai: bool },
}

impl ScriptPlan {
    pub fn source(&self) -> ScriptSource {
        match self {
            ScriptPlan::Forum { .. } => ScriptSource::Eroscripts,
            ScriptPlan::Slr { ai: false, .. } => ScriptSource::SlrHandcrafted,
            ScriptPlan::Slr { ai: true, .. } => ScriptSource::SlrAi,
        }
    }

    fn describe(&self) -> String {
        match self {
            ScriptPlan::Forum {
                topic_id,
                main,
                companions,
            } => {
                let axes: Vec<&str> = companions.iter().map(|(a, _)| *a).collect();
                format!(
                    "EroScripts #{topic_id} \"{}\"{}",
                    main.name,
                    if axes.is_empty() {
                        String::new()
                    } else {
                        format!(" + {}", axes.join(", "))
                    }
                )
            }
            ScriptPlan::Slr { script_id, ai } => format!(
                "SLR {} script #{script_id}",
                if *ai { "AI" } else { "hand-crafted" }
            ),
        }
    }
}

/// SLR's script of a kind in the scene's details: its id.
pub fn slr_script_id(detail: &Value, ai: bool) -> Option<u64> {
    detail["scripts"]
        .as_array()?
        .iter()
        .find(|s| s["scriptAI"].as_bool() == Some(ai))
        .and_then(|s| s["id"].as_u64())
}

/// The order to try script sources in: the scene's own first, then the
/// ones after it.
pub fn source_order(first: ScriptSource) -> Vec<ScriptSource> {
    let all = [
        ScriptSource::Eroscripts,
        ScriptSource::SlrHandcrafted,
        ScriptSource::SlrAi,
    ];
    let at = all.iter().position(|s| *s == first).unwrap_or(0);
    all[at..].to_vec()
}

/// Why a scene's download stops the run or is skipped.
enum Stop {
    /// Skip this scene (reported); `true` when it could come later (a quota).
    Skip(String, bool),
    /// Stop the run with this exit code.
    Halt(String, i32),
}

impl From<NetError> for Stop {
    fn from(e: NetError) -> Stop {
        match e {
            NetError::RateLimited(m) => Stop::Halt(format!("rate limited ({m})"), 2),
            NetError::Failed(m) => Stop::Skip(m, false),
        }
    }
}

impl From<QuotaError> for Stop {
    fn from(e: QuotaError) -> Stop {
        match e {
            QuotaError::NoSession | QuotaError::Expired => Stop::Halt(
                format!(
                    "SLR {}; sign in with npm run slr:scout -- --login",
                    e.message()
                ),
                3,
            ),
            QuotaError::RateLimited(m) => Stop::Halt(format!("rate limited ({m})"), 2),
            QuotaError::Failed(m) => Stop::Skip(m, false),
        }
    }
}

/// Everything a fetch run works with.
pub struct Fetch<'a> {
    pub http: &'a dyn Http,
    pub user_data: &'a Path,
    pub prefs: &'a Prefs,
    pub dry_run: bool,
    pub slr: Pacer,
    pub forum: Pacer,
    /// Canned network (tests): no upgrade run, no rescan.
    pub fake: bool,
}

struct Ctx {
    token: String,
    quota: Quota,
    cookie: Option<String>,
    other: PathBuf,
    staging: PathBuf,
}

/// A scene ready to download.
struct Planned {
    dir: PathBuf,
    name: String,
    option: VideoOption,
    script: ScriptPlan,
}

fn mb(bytes: Option<u64>) -> String {
    bytes.map_or("size unknown".into(), |b| {
        format!("{:.2} GB", b as f64 / 1_073_741_824.0)
    })
}

impl Fetch<'_> {
    /// Run it; returns the exit code.
    pub fn run(&mut self, out: &mut dyn Write) -> Result<i32, String> {
        let file_path = store::file_path(self.user_data);
        let mut file = store::read(&file_path)?.ok_or_else(|| {
            format!(
                "{} doesn't exist yet; run npm run slr:scout first",
                file_path.display()
            )
        })?;
        let right = right_swipes(self.user_data)?;
        let missing: Vec<u64> = right
            .iter()
            .copied()
            .filter(|id| !file.scenes.iter().any(|s| s.id == *id))
            .collect();
        let queue: Vec<u64> = file
            .scenes
            .iter()
            .filter(|s| right.contains(&s.id) && s.fetched_at.is_none())
            .map(|s| s.id)
            .collect();
        let done = right.len() - missing.len() - queue.len();
        writeln!(
            out,
            "{} scene(s) swiped right: {} to fetch, {done} already fetched{}.",
            right.len(),
            queue.len(),
            if missing.is_empty() {
                String::new()
            } else {
                format!(", {} no longer in slr-scout.json", missing.len())
            }
        )
        .map_err(io)?;
        if queue.is_empty() {
            return Ok(0);
        }

        let config = video_tags::config::read_json(self.user_data)?;
        let user = library::from_json(&config);
        let (other, is_source) = library::download_folder(&user.sources, FOLDER)
            .ok_or("config.json lists no library source to put SLR scenes beside")?;
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
        let now = timefmt::now_iso();
        let session = quota::session_token(self.http, self.user_data)
            .and_then(|token| Ok((quota::overview(self.http, &token, &now)?, token)));
        let (quota, token) = match session {
            Ok(q) => q,
            Err(e) => {
                let (msg, code) = match Stop::from(e) {
                    Stop::Halt(m, c) => (m, c),
                    Stop::Skip(m, _) => (m, 1),
                };
                writeln!(out, "Nothing fetched: {msg}.").map_err(io)?;
                return Ok(code);
            }
        };
        writeln!(
            out,
            "Quota: videos {}/{}, hand-crafted scripts {}/{}, AI scripts {}/{} left.",
            quota.videos.remaining,
            quota.videos.total,
            quota.scripts.remaining,
            quota.scripts.total,
            quota.ai_scripts.remaining,
            quota.ai_scripts.total
        )
        .map_err(io)?;
        let mut ctx = Ctx {
            token,
            quota,
            cookie: forum::session_cookie(&config),
            other,
            staging,
        };
        let cap = self.prefs.library_max_videos;
        let mut held = library::video_count(&user.sources);
        let (mut fetched, mut failed, mut skipped, mut code) = (0, 0, 0, 0);
        for id in queue {
            let scene = file
                .scenes
                .iter()
                .find(|s| s.id == id)
                .cloned()
                .ok_or("scene vanished")?;
            if held >= cap {
                writeln!(
                    out,
                    "Library holds {held} videos, the cap is {cap} (libraryMaxVideos): nothing more downloaded. Delete scenes or raise the cap."
                )
                .map_err(io)?;
                code = 2;
                break;
            }
            writeln!(out, "#{id} {}", scene.title).map_err(io)?;
            let planned = match self.plan(&scene, &mut ctx) {
                Ok(p) => p,
                Err(Stop::Skip(why, later)) => {
                    writeln!(out, "  skipped: {why}").map_err(io)?;
                    skipped += 1;
                    if later {
                        code = 2;
                    }
                    continue;
                }
                Err(Stop::Halt(why, c)) => {
                    writeln!(out, "  stopped: {why}").map_err(io)?;
                    code = c;
                    break;
                }
            };
            writeln!(
                out,
                "  video: {} {}p {}{} → {}",
                planned.option.kind,
                planned.option.height.unwrap_or(0),
                mb(planned.option.size),
                planned
                    .option
                    .codec
                    .as_deref()
                    .map_or(String::new(), |c| format!(" {c}")),
                planned.dir.join(&planned.name).display()
            )
            .map_err(io)?;
            writeln!(out, "  script: {}", planned.script.describe()).map_err(io)?;
            spend(&mut ctx.quota, &planned.script);
            ctx.quota.videos.remaining -= 1;
            held += 1;
            if self.dry_run {
                continue;
            }
            match self.download(&scene, &planned, &mut ctx, out) {
                Ok(()) => {
                    let at = timefmt::now_iso();
                    if let Some(s) = file.scenes.iter_mut().find(|s| s.id == id) {
                        s.fetched_at = Some(at.clone());
                    }
                    write_sidecar(&scene, &planned, &at)?;
                    store::write(self.user_data, &file)?;
                    self.upgrade(&planned.dir, out)?;
                    fetched += 1;
                    writeln!(out, "  ✓ fetched").map_err(io)?;
                }
                Err(Stop::Skip(why, _)) => {
                    writeln!(out, "  ✗ {why}").map_err(io)?;
                    failed += 1;
                }
                Err(Stop::Halt(why, c)) => {
                    writeln!(out, "  stopped: {why}").map_err(io)?;
                    code = c;
                    break;
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
        Ok(if failed > 0 { 1 } else { code })
    }

    /// Re-read the scene, pick its script and video download; no download
    /// request.
    fn plan(&mut self, scene: &Scene, ctx: &mut Ctx) -> Result<Planned, Stop> {
        if ctx.quota.videos.remaining <= 0 {
            return Err(Stop::Halt(
                "the weekly video download quota is used up".into(),
                2,
            ));
        }
        let detail = slr::detail(self.http, &mut self.slr, &scene.label)?;
        let script = self.plan_script(scene, &detail, ctx)?;
        let caps = &self.prefs.video_caps;
        if let Some(fps) = detail["fps"].as_f64().filter(|f| *f > caps.vr_max_fps) {
            return Err(Stop::Skip(
                format!("{fps} fps, over the {} fps cap", caps.vr_max_fps),
                false,
            ));
        }
        self.slr.wait();
        let options = quota::authed_json(
            self.http,
            &quota::authed(
                format!("{API}/content/v1/videos/{}/download-options", scene.id),
                &ctx.token,
            ),
            "download options",
        )?;
        let option = choose(&parse_options(&options), scene.vr, caps)
            .map_err(|why| Stop::Skip(why, false))?;
        let stem = stem(&scene.actors, &scene.title, scene.studio.as_deref());
        let name = format!("{stem}{}", vr_suffix(&detail));
        let dir = ctx
            .other
            .join(artist_folder(&scene.actors, scene.studio.as_deref()))
            .join(&name);
        Ok(Planned {
            dir,
            name: format!("{name}.mp4"),
            option,
            script,
        })
    }

    fn plan_script(
        &mut self,
        scene: &Scene,
        detail: &Value,
        ctx: &Ctx,
    ) -> Result<ScriptPlan, Stop> {
        let Some(first) = scene.script_source.or_else(|| store::script_source(scene)) else {
            return Err(Stop::Skip("no script source".into(), false));
        };
        let mut why = Vec::new();
        let mut later = false;
        for source in source_order(first) {
            match source {
                ScriptSource::Eroscripts => {
                    let topic = scene.handmade.iter().find_map(|h| match h {
                        Handmade::Eroscripts {
                            topic_id,
                            free: true,
                            ..
                        } => Some(*topic_id),
                        _ => None,
                    });
                    let (Some(topic_id), Some(cookie)) = (topic, ctx.cookie.as_deref()) else {
                        if topic.is_some() {
                            why.push("no EroScripts session in config.json".to_string());
                        }
                        continue;
                    };
                    let scripts =
                        forum::topic_scripts(self.http, &mut self.forum, cookie, topic_id)?;
                    match forum::pick_scripts(&scripts, &scene.title) {
                        Some((main, companions)) => {
                            return Ok(ScriptPlan::Forum {
                                topic_id,
                                main: main.clone(),
                                companions: companions
                                    .into_iter()
                                    .map(|(a, x)| (a, x.clone()))
                                    .collect(),
                            });
                        }
                        None => why.push(format!(
                            "EroScripts #{topic_id} has no .funscript attachment for this scene"
                        )),
                    }
                }
                ScriptSource::SlrHandcrafted | ScriptSource::SlrAi => {
                    let ai = source == ScriptSource::SlrAi;
                    let Some(script_id) = slr_script_id(detail, ai) else {
                        continue;
                    };
                    let left = if ai {
                        ctx.quota.ai_scripts.remaining
                    } else {
                        ctx.quota.scripts.remaining
                    };
                    if left <= 0 {
                        why.push(format!(
                            "the monthly {} script quota is used up",
                            if ai { "AI" } else { "hand-crafted" }
                        ));
                        later = true;
                        continue;
                    }
                    return Ok(ScriptPlan::Slr { script_id, ai });
                }
            }
        }
        if why.is_empty() {
            why.push("no script source".into());
        }
        Err(Stop::Skip(why.join("; "), later))
    }

    fn download(
        &mut self,
        scene: &Scene,
        p: &Planned,
        ctx: &mut Ctx,
        out: &mut dyn Write,
    ) -> Result<(), Stop> {
        let fail = |what: &str, e: std::io::Error| Stop::Skip(format!("{what}: {e}"), false);
        let video = p.dir.join(&p.name);
        let have = fs::metadata(&video).map(|m| m.len()).ok();
        if have.is_some() && have == p.option.size {
            let _ = writeln!(out, "  video already there");
        } else {
            self.slr.wait();
            let v = quota::authed_json(
                self.http,
                &quota::authed(
                    format!(
                        "{API}/content/v1/videos/{}/download?type={}",
                        scene.id, p.option.kind
                    ),
                    &ctx.token,
                ),
                "video download",
            )?;
            let url = v["data"]["url"]
                .as_str()
                .or_else(|| v["file"]["url"].as_str())
                .ok_or_else(|| Stop::Skip("SLR gave no download URL".into(), false))?;
            if let Some(n) = v["data"]["availableDownloadsCount"].as_i64() {
                ctx.quota.videos.remaining = n;
            }
            fs::create_dir_all(&ctx.staging).map_err(|e| fail("staging", e))?;
            let part = ctx.staging.join(format!("{}.part", p.name));
            let _ = writeln!(out, "  downloading {} ...", mb(p.option.size));
            self.http
                .download(&crate::http::Request::get(url), &part)
                .map_err(|e| Stop::Skip(format!("video download: {e}"), false))?;
            fs::create_dir_all(&p.dir).map_err(|e| fail("scene folder", e))?;
            move_file(&part, &video).map_err(|e| fail("moving the video", e))?;
        }
        let stem = p.name.trim_end_matches(".mp4");
        match &p.script {
            ScriptPlan::Forum {
                main, companions, ..
            } => {
                let cookie = ctx.cookie.clone().unwrap_or_default();
                forum::download_upload(
                    self.http,
                    &mut self.forum,
                    &cookie,
                    &main.url,
                    &p.dir.join(format!("{stem}.funscript")),
                )?;
                for (axis, a) in companions {
                    forum::download_upload(
                        self.http,
                        &mut self.forum,
                        &cookie,
                        &a.url,
                        &p.dir.join(format!("{stem}.{axis}.funscript")),
                    )?;
                }
            }
            ScriptPlan::Slr { script_id, .. } => {
                self.slr.wait();
                let v = quota::authed_json(
                    self.http,
                    &quota::authed(
                        format!("{API}/content/v1/scripts/{script_id}/download"),
                        &ctx.token,
                    ),
                    "script download",
                )?;
                let link = v["data"]["link"]
                    .as_str()
                    .ok_or_else(|| Stop::Skip("SLR gave no script link".into(), false))?;
                fs::create_dir_all(&ctx.staging).map_err(|e| fail("staging", e))?;
                let part = ctx.staging.join(format!("{stem}.script.part"));
                self.http
                    .download(&crate::http::Request::get(link), &part)
                    .map_err(|e| Stop::Skip(format!("script download: {e}"), false))?;
                let placed = place_script(&part, &p.dir, stem, &ctx.staging);
                let _ = fs::remove_file(&part);
                placed.map_err(|e| Stop::Skip(e, false))?;
            }
        }
        Ok(())
    }

    /// Generated twist / roll / pitch for the new scene (hand-made axes win).
    fn upgrade(&self, dir: &Path, out: &mut dyn Write) -> Result<(), String> {
        if self.fake {
            return Ok(());
        }
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/upgrade-multi-axis.mjs");
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
        let base = video_tags::host::backend_url(port);
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

/// Count a planned script against its quota.
fn spend(quota: &mut Quota, script: &ScriptPlan) {
    match script {
        ScriptPlan::Slr { ai: true, .. } => quota.ai_scripts.remaining -= 1,
        ScriptPlan::Slr { ai: false, .. } => quota.scripts.remaining -= 1,
        ScriptPlan::Forum { .. } => {}
    }
}

/// Rename, or copy and remove across volumes.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to)?;
    fs::remove_file(from)
}

/// Put a downloaded SLR script in the scene folder as `<stem>.funscript`:
/// a funscript as is; a zip's scripts by their axis (stroke →
/// `<stem>.funscript`, `x.twist.funscript` → `<stem>.twist.funscript`).
pub fn place_script(file: &Path, dir: &Path, stem: &str, staging: &Path) -> Result<(), String> {
    let bytes = fs::read(file).map_err(|e| format!("script: {e}"))?;
    if bytes.starts_with(b"PK") {
        let tmp = staging.join(format!("{stem}.unzip"));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).map_err(|e| format!("script: {e}"))?;
        let ok = Command::new("unzip")
            .args(["-o", "-j", "-q"])
            .arg(file)
            .arg("*.funscript")
            .arg("-d")
            .arg(&tmp)
            .status()
            .is_ok_and(|s| s.success());
        let mut placed = 0;
        if ok {
            let mut names: Vec<String> = fs::read_dir(&tmp)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| !n.starts_with('.'))
                .collect();
            names.sort();
            for n in names {
                let Some((_, axis)) = forum::script_axis(&n) else {
                    continue;
                };
                let to = match axis {
                    Some(a) => dir.join(format!("{stem}.{a}.funscript")),
                    None => dir.join(format!("{stem}.funscript")),
                };
                if !to.exists() && fs::rename(tmp.join(&n), &to).is_ok() {
                    placed += 1;
                }
            }
        }
        let _ = fs::remove_dir_all(&tmp);
        return if placed > 0 && dir.join(format!("{stem}.funscript")).exists() {
            Ok(())
        } else {
            Err("SLR's script archive holds no stroke .funscript".into())
        };
    }
    let v: Value =
        serde_json::from_slice(&bytes).map_err(|_| "SLR's script isn't a funscript".to_string())?;
    if !v["actions"].is_array() {
        return Err("SLR's script isn't a funscript (no actions)".into());
    }
    fs::write(dir.join(format!("{stem}.funscript")), &bytes).map_err(|e| format!("script: {e}"))
}

fn write_sidecar(scene: &Scene, p: &Planned, at: &str) -> Result<(), String> {
    let script = match &p.script {
        ScriptPlan::Forum { topic_id, main, .. } => {
            json!({"topicId": topic_id, "url": format!("{}/t/{topic_id}", forum::FORUM), "file": main.name})
        }
        ScriptPlan::Slr { script_id, ai } => json!({"scriptId": script_id, "ai": ai}),
    };
    let v = json!({
        "sceneId": scene.id,
        "url": scene.url,
        "title": scene.title,
        "studio": scene.studio,
        "performers": scene.actors,
        "file": p.name,
        "scriptSource": p.script.source(),
        "script": script,
        "video": {"type": p.option.kind, "height": p.option.height, "size": p.option.size},
        "fetchedAt": at,
    });
    let path = p.dir.join(SIDECAR);
    let text = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    fs::write(&path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

fn io(e: std::io::Error) -> String {
    format!("output: {e}")
}

/// Carry `fetchedAt` over from the previous file.
pub fn keep_fetched(previous: Option<&ScoutFile>, scenes: &mut [Scene]) {
    if let Some(p) = previous {
        for s in scenes {
            s.fetched_at = p.fetched_at(s.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps() -> VideoCaps {
        crate::prefs::load().unwrap().video_caps
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn names_the_vr_projection() {
        let d = |v: Value| vr_suffix(&v);
        assert_eq!(
            d(json!({"projection": 0, "screenType": 0, "viewAngle": 180, "stereomode": "sbs2l"})),
            "_180_LR"
        );
        assert_eq!(
            d(json!({"screenType": 11, "viewAngle": 190, "stereomode": "sbs2l"})),
            "_FISHEYE190"
        );
        assert_eq!(d(json!({"screenType": 9})), "_FISHEYE190");
        assert_eq!(d(json!({"screenType": 6, "viewAngle": 200})), "_MKX200");
        assert_eq!(d(json!({"screenType": 7})), "_MKX220");
        assert_eq!(d(json!({"screenType": 8})), "_VRCA220");
        assert_eq!(d(json!({"screenType": 10})), "_RF52");
        assert_eq!(d(json!({"screenType": 5, "stereomode": "tb"})), "_360_TB");
        assert_eq!(
            d(json!({"screenType": 4, "viewAngle": 360, "stereomode": "mono"})),
            "_MONO360"
        );
        assert_eq!(
            d(json!({"screenType": 0, "stereomode": "weird"})),
            "_180_LR"
        );
        assert_eq!(
            d(json!({"screenType": 0, "isFlatScreenType": true, "stereomode": "mono"})),
            ""
        );
        assert_eq!(d(json!({"screenType": 3})), "");
    }

    #[test]
    fn names_scenes() {
        assert_eq!(
            stem(
                &strings(&["Miss Lexa", "Lexi: Luna"]),
                "Oiled up / creampied?",
                Some("Miss Lexa VR")
            ),
            "Miss Lexa, Lexi Luna - Oiled up creampied (Miss Lexa VR)"
        );
        assert_eq!(stem(&[], "T", Some("Studio")), "Studio - T (Studio)");
        assert_eq!(stem(&[], "T", None), "SexLikeReal - T (SexLikeReal)");
        // Too long: performers go from the end (eight fit), then the cut
        // when one alone is still too long.
        let many: Vec<String> = (0..20).map(|i| format!("Performer Number {i}")).collect();
        let s = stem(&many, "Title", Some("Studio"));
        assert!(s.starts_with("Performer Number 0, ") && s.len() <= STEM_BYTES);
        assert!(s.ends_with("Performer Number 7 - Title (Studio)"));
        let s = stem(&strings(&["P".repeat(200).as_str(), "Q"]), "T", Some("S"));
        assert_eq!(s, "P".repeat(STEM_BYTES));
        let long = "Ü".repeat(120);
        let s = stem(&strings(&["A"]), &long, Some("S"));
        assert!(s.len() <= STEM_BYTES && s.starts_with("A - ÜÜ"));
        let s = stem(
            &strings(&["A"]),
            &format!("{} - x", "y".repeat(175)),
            Some("S"),
        );
        assert!(!s.ends_with(' ') && !s.ends_with('-') && s.len() <= STEM_BYTES);
        assert_eq!(
            artist_folder(&strings(&["Miss Lexa"]), Some("S")),
            "Miss Lexa"
        );
        assert_eq!(artist_folder(&[], Some("S|x")), "Sx");
        assert_eq!(artist_folder(&[], None), "SexLikeReal");
    }

    fn opt(kind: &str, height: u64) -> Value {
        json!({"type": kind, "height": height, "width": height * 2, "size": height * 1_000_000,
               "codec": "h265", "isDownloadable": true, "isAvailableThroughPurchase": false})
    }

    #[test]
    fn chooses_the_download_within_the_caps() {
        let caps = caps();
        let v = json!({"data": {"options": [opt("trailer", 1080), opt("maxResolution", 4096),
            opt("optimalResolution", 2880),
            {"type": "original", "height": 4096, "isAvailableThroughPurchase": true}]}});
        let options = parse_options(&v);
        assert_eq!(options.len(), 4);
        assert!(!options[3].downloadable);
        assert_eq!(choose(&options, true, &caps).unwrap().kind, "maxResolution");
        // Over the VR cap: the optimal one.
        let v = json!({"data": {"options": [opt("maxResolution", 5760), opt("optimalResolution", 3840)]}});
        let c = choose(&parse_options(&v), true, &caps).unwrap();
        assert_eq!(
            (c.kind.as_str(), c.height),
            ("optimalResolution", Some(3840))
        );
        // Flat has its own cap.
        let v = json!({"data": {"options": [opt("maxResolution", 2880), opt("optimalResolution", 1080)]}});
        assert_eq!(
            choose(&parse_options(&v), false, &caps).unwrap().kind,
            "optimalResolution"
        );
        // Nothing within the caps, or only the trailer: no download.
        let v = json!({"data": {"options": [opt("maxResolution", 5760), opt("optimalResolution", 5760)]}});
        assert!(choose(&parse_options(&v), true, &caps).is_err());
        let v = json!({"data": {"options": [opt("trailer", 1080)]}});
        assert!(choose(&parse_options(&v), true, &caps).is_err());
        // The older shape: an object keyed by type.
        let v = json!({"data": {"options": {"maxResolution": {"height": 4096, "size": 9},
            "trailer": {"height": 1080}}}});
        assert_eq!(
            choose(&parse_options(&v), true, &caps).unwrap().size,
            Some(9)
        );
    }

    #[test]
    fn falls_back_through_script_sources() {
        assert_eq!(
            source_order(ScriptSource::Eroscripts),
            [
                ScriptSource::Eroscripts,
                ScriptSource::SlrHandcrafted,
                ScriptSource::SlrAi
            ]
        );
        assert_eq!(source_order(ScriptSource::SlrAi), [ScriptSource::SlrAi]);
        let d = json!({"scripts": [{"id": 1, "scriptAI": true}, {"id": 2, "scriptAI": false}]});
        assert_eq!(slr_script_id(&d, true), Some(1));
        assert_eq!(slr_script_id(&d, false), Some(2));
        assert_eq!(slr_script_id(&json!({}), false), None);
    }

    #[test]
    fn reads_right_swipes() {
        let dir = std::env::temp_dir().join(format!("slr-scout-swipes-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(right_swipes(&dir).unwrap().is_empty());
        fs::write(
            dir.join(SWIPES_FILE),
            r#"{"version": 1, "swipes": {"66249": {"dir": "right", "at": "x"},
                "65840": {"dir": "right"}, "7": {"dir": "left"}, "bad": {"dir": "right"}}}"#,
        )
        .unwrap();
        assert_eq!(right_swipes(&dir).unwrap(), [65840, 66249]);
        fs::write(dir.join(SWIPES_FILE), "nope").unwrap();
        assert!(right_swipes(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn places_slr_scripts() {
        let dir = std::env::temp_dir().join(format!("slr-scout-place-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("dl.part");
        fs::write(&file, r#"{"actions": [{"at": 0, "pos": 0}]}"#).unwrap();
        place_script(&file, &dir, "Scene_180_LR", &dir).unwrap();
        assert!(dir.join("Scene_180_LR.funscript").exists());
        fs::write(&file, "<html>").unwrap();
        assert!(place_script(&file, &dir, "Other", &dir).is_err());
        fs::write(&file, r#"{"version": 1}"#).unwrap();
        assert!(place_script(&file, &dir, "Other", &dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
