//! Preview frames for the swipe deck's SLR scene cards: after a scan, three JPEGs per scene
//! at 25, 50 and 75 % of SLR's free trailer (`trailerEncodings` in the scene details the scan
//! reads anyway, else the `videoPreview` clip; never the full video, which counts against the
//! weekly download quota), at most MAX_WIDTH wide, one eye for stereo VR (side-by-side and
//! fisheye: the left half; top-bottom: the top half; mono and flat: the whole frame), into
//! `<userData>/scout-frames/slr/<id>/{25,50,75}.jpg` (temp file + rename). ffmpeg seeks the
//! input, so a frame reads a few MB of the trailer. The scene records `frames` (kept across
//! runs while a file is on disk) or, when no frame came out, `framesFailed`, tried again
//! after RETRY_DAYS. At most `--frames N` scenes a run (default DEFAULT_PER_SCAN), highest
//! score first, paced; scenes the deck doesn't show are left out (downloaded, swiped, or
//! brought by a waiting EroScripts post, which shows its own frames). scout-report serves the
//! files and adds them to the cards' images.

use crate::http::Pacer;
use crate::store::{Scene, ScoutFile};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};
use video_tags::timefmt;

/// `<userData>/scout-frames/<source>/<id>/` (shared with the EroScripts scout).
pub const DIR: &str = "scout-frames";
/// Scenes given frames per scan unless `--frames N` says otherwise.
pub const DEFAULT_PER_SCAN: usize = 100;
/// A scene whose frames failed is tried again after this many days.
pub const RETRY_DAYS: u64 = 7;
/// Widest frame kept, in pixels (one eye).
pub const MAX_WIDTH: u64 = 1280;
/// Where in the trailer the frames are taken, in percent of its length.
pub const POINTS: [u32; 3] = [25, 50, 75];
/// Between two scenes' trailers (SLR's CDN, not its API).
pub const GAP: Duration = Duration::from_millis(500);
/// ffmpeg / ffprobe give up on a stalled read after this long (microseconds).
const NET_TIMEOUT_US: &str = "20000000";
/// A whole ffmpeg / ffprobe run is killed after this long.
const RUN_DEADLINE: Duration = Duration::from_secs(60);

/// A scene's frames in slr-scout.json: `{at, source, files}` (`source`: `slr: <file name>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frames {
    pub at: String,
    pub source: String,
    pub files: Vec<String>,
}

/// Why a scene has no frames: `{at, reason}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FramesFailed {
    pub at: String,
    pub reason: String,
}

/// The part of the frame one eye fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Crop {
    Whole,
    Left,
    Top,
}

impl Crop {
    /// From the scene details, through the projection suffix `--fetch` names files with:
    /// flat and mono whole, top-bottom the top half, everything else stereo side by side
    /// (`_LR`, fisheye, MKX, VRCA, RF52) the left half.
    pub fn of(d: &Value) -> Crop {
        let suffix = crate::fetch::vr_suffix(d);
        if suffix.is_empty() || suffix.starts_with("_MONO") {
            Crop::Whole
        } else if suffix.ends_with("_TB") {
            Crop::Top
        } else {
            Crop::Left
        }
    }

    /// The ffmpeg filter: the eye, then at most MAX_WIDTH wide (even height).
    pub fn filter(self) -> String {
        let scale = format!("scale='min({MAX_WIDTH},iw)':-2");
        match self {
            Crop::Whole => scale,
            Crop::Left => format!("crop=iw/2:ih:0:0,{scale}"),
            Crop::Top => format!("crop=iw:ih/2:0:0,{scale}"),
        }
    }

    fn eye_width(self, width: u64) -> u64 {
        if self == Crop::Left { width / 2 } else { width }
    }
}

/// A scene's free trailer.
#[derive(Debug, Clone, PartialEq)]
pub struct Trailer {
    pub url: String,
    /// The file name, for `source`.
    pub name: String,
    pub crop: Crop,
}

/// The file name of a URL (no query).
fn file_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// The free trailer in scene details `d`: a `trailerEncodings` source (h264 first, being the
/// quickest to decode), the largest whose eye fits MAX_WIDTH, else the smallest; else the
/// `videoPreview` clip. https URLs only; `None` when there's neither.
pub fn trailer(d: &Value) -> Option<Trailer> {
    let crop = Crop::of(d);
    let mut sources: Vec<(bool, u64, &str)> = Vec::new();
    for enc in d["trailerEncodings"].as_array().into_iter().flatten() {
        let h264 = enc["name"].as_str() == Some("h264");
        for v in enc["videoSources"].as_array().into_iter().flatten() {
            if let Some(url) = v["url"].as_str().filter(|u| u.starts_with("https://")) {
                sources.push((h264, crop.eye_width(v["width"].as_u64().unwrap_or(0)), url));
            }
        }
    }
    let h264 = sources.iter().any(|s| s.0);
    sources.retain(|s| s.0 == h264);
    let fits = sources
        .iter()
        .filter(|s| s.1 <= MAX_WIDTH)
        .max_by_key(|s| s.1);
    let url = fits
        .or_else(|| sources.iter().min_by_key(|s| s.1))
        .map(|s| s.2)
        .or_else(|| {
            d["videoPreview"]
                .as_str()
                .filter(|u| u.starts_with("https://"))
        })?;
    Some(Trailer {
        url: url.to_string(),
        name: file_name(url),
        crop,
    })
}

/// What makes the frames: ffmpeg and ffprobe, or a fake in tests.
pub trait Media {
    /// The video's length in seconds.
    fn duration(&self, url: &str) -> Result<f64, String>;
    /// One JPEG of the video at `at_s` seconds, cropped by `crop`, written to `out`.
    fn grab(&self, url: &str, at_s: f64, crop: Crop, out: &Path) -> Result<(), String>;
}

/// The real [`Media`]: ffprobe for the length, ffmpeg with input seeking for each frame.
pub struct Ffmpeg;

impl Media for Ffmpeg {
    fn duration(&self, url: &str) -> Result<f64, String> {
        let out = run(Command::new("ffprobe")
            .args(["-v", "error", "-rw_timeout", NET_TIMEOUT_US])
            .args(["-show_entries", "format=duration", "-of", "csv=p=0"])
            .arg(url))?;
        String::from_utf8_lossy(&out)
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|d| d.is_finite() && *d > 0.0)
            .ok_or_else(|| "unknown length".to_string())
    }

    fn grab(&self, url: &str, at_s: f64, crop: Crop, out: &Path) -> Result<(), String> {
        run(Command::new("ffmpeg")
            .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"])
            .args(["-rw_timeout", NET_TIMEOUT_US])
            .args(["-ss", &format!("{at_s:.3}")])
            .arg("-i")
            .arg(url)
            .args(["-frames:v", "1", "-vf", &crop.filter(), "-q:v", "3"])
            .arg(out))
        .map(drop)
    }
}

/// Run `cmd` (killed after RUN_DEADLINE); its stdout, or why it failed (the last line of
/// its stderr).
fn run(cmd: &mut Command) -> Result<Vec<u8>, String> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("can't run ffmpeg/ffprobe ({e}); is it installed?"))?;
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    };
    let stdout = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let stderr = drain(child.stderr.take().map(|p| Box::new(p) as _));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() >= RUN_DEADLINE => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!("timed out after {}s", RUN_DEADLINE.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => break Err(e.to_string()),
        }
    };
    let out = stdout.join().unwrap_or_default();
    let err = stderr.join().unwrap_or_default();
    let status = status?;
    if status.success() {
        return Ok(out);
    }
    let err = String::from_utf8_lossy(&err);
    let last = err
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("");
    Err(format!("{status} ({})", last.trim()))
}

/// `<userData>/scout-frames/slr/<id>/`.
pub fn scene_dir(user_data: &Path, id: u64) -> PathBuf {
    user_data.join(DIR).join("slr").join(id.to_string())
}

/// The frames of `trailer` into `dir` (each through a temp file); the names of the files
/// made, in POINTS order, or why none was.
pub fn take(media: &dyn Media, dir: &Path, trailer: &Trailer) -> Result<Vec<String>, String> {
    let length = media.duration(&trailer.url)?;
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut files = Vec::new();
    let mut why = String::new();
    for p in POINTS {
        let name = format!("{p}.jpg");
        let tmp = dir.join(format!(".{p}.part.jpg"));
        let result = media.grab(
            &trailer.url,
            length * f64::from(p) / 100.0,
            trailer.crop,
            &tmp,
        );
        match result {
            Ok(()) if fs::metadata(&tmp).is_ok_and(|m| m.len() > 0) => {
                match fs::rename(&tmp, dir.join(&name)) {
                    Ok(()) => files.push(name),
                    Err(e) => why = format!("{name}: {e}"),
                }
            }
            Ok(()) => why = "no frame".into(),
            Err(e) => why = e,
        }
        let _ = fs::remove_file(&tmp);
    }
    if files.is_empty() {
        Err(why)
    } else {
        Ok(files)
    }
}

/// The EroScripts scout's waiting list (its posts with a video on SLR stand in for that scene
/// on the deck, with their own frames).
pub const WAITING_FILE: &str = "eroscripts-scout.json";

/// The SLR scene ids of the waiting posts' videos (`scenes[].video` with `host: "slr"` and
/// its `slrSceneId`): the deck shows the post instead of the scene. A missing or unreadable
/// list covers none.
pub fn covered(user_data: &Path) -> HashSet<u64> {
    let Ok(raw) = fs::read(user_data.join(WAITING_FILE)) else {
        return HashSet::new();
    };
    let v: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    v["candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|c| c["scenes"].as_array().into_iter().flatten())
        .map(|s| &s["video"])
        .filter(|v| v["host"] == "slr")
        .filter_map(|v| match &v["slrSceneId"] {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.parse().ok(),
            _ => None,
        })
        .collect()
}

/// Carry the previous file's `frames` (while one of its files is still on disk) or else its
/// `framesFailed` over to the same scenes.
pub fn keep(previous: Option<&ScoutFile>, scenes: &mut [Scene], user_data: &Path) {
    let Some(p) = previous else { return };
    let old: HashMap<u64, &Scene> = p.scenes.iter().map(|s| (s.id, s)).collect();
    for s in scenes {
        let Some(o) = old.get(&s.id) else { continue };
        let dir = scene_dir(user_data, s.id);
        s.frames = o
            .frames
            .clone()
            .filter(|f| f.files.iter().any(|n| dir.join(n).is_file()));
        s.frames_failed = if s.frames.is_none() {
            o.frames_failed.clone()
        } else {
            None
        };
    }
}

/// How one frames pass went.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Scenes that got frames.
    pub made: usize,
    /// Scenes tried that got none (`framesFailed`).
    pub failed: usize,
    /// Scenes still due, past the run's limit.
    pub left: usize,
}

/// One frames pass over `scenes` (highest score first): each scene with no frames, not
/// downloaded, not in `skip` (swiped or covered) and no failure in the last RETRY_DAYS gets
/// frames from its trailer in `trailers`, at most `limit` scenes, one trailer per `pacer`
/// turn. A scene without a trailer fails at once (no request). Progress goes to `log`.
#[allow(clippy::too_many_arguments)]
pub fn pass(
    media: &dyn Media,
    user_data: &Path,
    scenes: &mut [Scene],
    trailers: &HashMap<u64, Trailer>,
    skip: &HashSet<u64>,
    limit: usize,
    pacer: &mut Pacer,
    now: SystemTime,
    log: &mut dyn Write,
) -> Outcome {
    let cutoff = timefmt::iso_utc(
        now.checked_sub(Duration::from_secs(RETRY_DAYS * 86_400))
            .unwrap_or(SystemTime::UNIX_EPOCH),
    );
    let at = timefmt::iso_utc(now);
    let mut out = Outcome::default();
    let mut tried = 0;
    for s in scenes.iter_mut() {
        let due = s.frames.is_none()
            && s.fetched_at.is_none()
            && !skip.contains(&s.id)
            && s.frames_failed
                .as_ref()
                .is_none_or(|f| f.at.as_str() < cutoff.as_str());
        if !due {
            continue;
        }
        let result = match trailers.get(&s.id) {
            None => Err("no free trailer".to_string()),
            Some(_) if tried == limit => {
                out.left += 1;
                continue;
            }
            Some(t) => {
                tried += 1;
                pacer.wait();
                take(media, &scene_dir(user_data, s.id), t).map(|files| (t, files))
            }
        };
        match result {
            Ok((t, files)) => {
                let _ = writeln!(log, "  frames {}: {} from {}", s.id, files.len(), t.name);
                s.frames = Some(Frames {
                    at: at.clone(),
                    source: format!("slr: {}", t.name),
                    files,
                });
                s.frames_failed = None;
                out.made += 1;
            }
            Err(reason) => {
                let _ = writeln!(log, "  frames {}: none ({reason})", s.id);
                s.frames_failed = Some(FramesFailed {
                    at: at.clone(),
                    reason,
                });
                out.failed += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_dir(name: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "slr-scout-frames-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn source(width: u64, url: &str) -> Value {
        json!({"width": width, "height": width / 2, "url": url})
    }

    #[test]
    fn crops_by_projection() {
        let c = |v: Value| Crop::of(&v);
        assert_eq!(
            c(json!({"screenType": 0, "stereomode": "sbs2l", "viewAngle": 180})),
            Crop::Left
        );
        assert_eq!(
            c(json!({"screenType": 11, "stereomode": "sbs2l"})),
            Crop::Left
        );
        assert_eq!(c(json!({"screenType": 6})), Crop::Left);
        assert_eq!(c(json!({"screenType": 5, "stereomode": "tb"})), Crop::Top);
        assert_eq!(c(json!({"stereomode": "ou"})), Crop::Top);
        assert_eq!(
            c(json!({"screenType": 4, "viewAngle": 360, "stereomode": "mono"})),
            Crop::Whole
        );
        assert_eq!(c(json!({"isFlatScreenType": true})), Crop::Whole);
        assert_eq!(c(json!({"screenType": 3})), Crop::Whole);
        assert_eq!(
            Crop::Left.filter(),
            "crop=iw/2:ih:0:0,scale='min(1280,iw)':-2"
        );
        assert_eq!(
            Crop::Top.filter(),
            "crop=iw:ih/2:0:0,scale='min(1280,iw)':-2"
        );
        assert_eq!(Crop::Whole.filter(), "scale='min(1280,iw)':-2");
    }

    #[test]
    fn picks_the_trailer() {
        let d = json!({
            "stereomode": "sbs2l",
            "videoPreview": "https://cdn/preview/1_300p.mp4",
            "trailerEncodings": [
                {"name": "h265", "videoSources": [source(2160, "https://cdn/h265/1_1080p.mp4")]},
                {"name": "h264", "videoSources": [
                    source(960, "https://cdn/h264/1_480p.mp4?Expires=1&Signature=x"),
                    source(2160, "https://cdn/h264/1_1080p.mp4?Expires=1&Signature=x"),
                    source(4320, "https://cdn/h264/1_2160p.mp4"),
                    source(1440, "http://cdn/h264/1_720p.mp4")]}
            ]
        });
        // The left eye of 2160 is 1080: the largest that fits.
        let t = trailer(&d).unwrap();
        assert_eq!(t.url, "https://cdn/h264/1_1080p.mp4?Expires=1&Signature=x");
        assert_eq!((t.name.as_str(), t.crop), ("1_1080p.mp4", Crop::Left));
        // Mono: 2160 is too wide, so 960.
        let mut mono = d.clone();
        mono["stereomode"] = json!("mono");
        assert_eq!(trailer(&mono).unwrap().name, "1_480p.mp4");
        // Only sources too wide: the smallest; no h264: another encoding.
        let wide = json!({"trailerEncodings": [{"name": "h265", "videoSources": [
            source(8640, "https://cdn/a.mp4"), source(4320, "https://cdn/b.mp4")]}]});
        assert_eq!(trailer(&wide).unwrap().name, "b.mp4");
        // No encodings: the preview clip; nothing at all (or not https): none.
        let preview =
            json!({"videoPreview": "https://cdn/preview/1_300p.mp4", "trailerEncodings": []});
        assert_eq!(trailer(&preview).unwrap().name, "1_300p.mp4");
        assert_eq!(trailer(&json!({"videoPreview": "http://cdn/p.mp4"})), None);
        assert_eq!(trailer(&json!({})), None);
    }

    /// Writes `fake <url> <at_s> <crop>` as the frame; fails for urls containing "403" and
    /// at the 50 % mark of urls containing "half".
    #[derive(Default)]
    struct FakeMedia {
        grabs: RefCell<Vec<(String, f64, Crop)>>,
    }

    impl Media for FakeMedia {
        fn duration(&self, url: &str) -> Result<f64, String> {
            if url.contains("403") {
                Err("exit status: 1 (HTTP error 403 Forbidden)".into())
            } else {
                Ok(120.0)
            }
        }

        fn grab(&self, url: &str, at_s: f64, crop: Crop, out: &Path) -> Result<(), String> {
            self.grabs.borrow_mut().push((url.into(), at_s, crop));
            if url.contains("half") && at_s == 60.0 {
                return Err("exit status: 1 (bad frame)".into());
            }
            fs::write(out, format!("fake {url} {at_s} {crop:?}")).map_err(|e| e.to_string())
        }
    }

    fn scene(id: u64) -> Scene {
        crate::slr::scene(&json!({"id": id, "label": format!("s-{id}"), "title": "T"})).unwrap()
    }

    fn trailer_for(url: &str) -> Trailer {
        Trailer {
            url: url.into(),
            name: file_name(url),
            crop: Crop::Left,
        }
    }

    #[test]
    fn takes_three_frames_through_temp_files() {
        let dir = temp_dir("take");
        let media = FakeMedia::default();
        let files = take(&media, &dir.join("7"), &trailer_for("https://cdn/7.mp4")).unwrap();
        assert_eq!(files, ["25.jpg", "50.jpg", "75.jpg"]);
        assert_eq!(
            media.grabs.borrow().iter().map(|g| g.1).collect::<Vec<_>>(),
            [30.0, 60.0, 90.0]
        );
        assert_eq!(
            fs::read_to_string(dir.join("7/50.jpg")).unwrap(),
            "fake https://cdn/7.mp4 60 Left"
        );
        let mut names: Vec<String> = fs::read_dir(dir.join("7"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["25.jpg", "50.jpg", "75.jpg"]);
        // One frame failing leaves the other two.
        let files = take(&media, &dir.join("8"), &trailer_for("https://cdn/half.mp4")).unwrap();
        assert_eq!(files, ["25.jpg", "75.jpg"]);
        assert!(!dir.join("8/50.jpg").exists() && !dir.join("8/.50.part.jpg").exists());
        // A refused trailer: no frame, no folder.
        let e = take(&media, &dir.join("9"), &trailer_for("https://cdn/403.mp4")).unwrap_err();
        assert!(e.contains("403 Forbidden"), "{e}");
        assert!(!dir.join("9").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pass_is_bounded_and_records_failures() {
        let ud = temp_dir("pass");
        let media = FakeMedia::default();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_794_700_000);
        let day = |d: u64| timefmt::iso_utc(now - Duration::from_secs(d * 86_400));
        let mut scenes: Vec<Scene> = (1..=8).map(scene).collect();
        scenes[1].fetched_at = Some(day(1)); // 2: downloaded
        scenes[2].frames_failed = Some(FramesFailed {
            at: day(3),
            reason: "x".into(),
        }); // 3: failed lately
        scenes[3].frames_failed = Some(FramesFailed {
            at: day(8),
            reason: "x".into(),
        }); // 4: failed long ago, tried again
        let kept = Frames {
            at: day(2),
            source: "slr: old.mp4".into(),
            files: vec!["25.jpg".into()],
        };
        scenes[4].frames = Some(kept.clone()); // 5: has frames
        let trailers: HashMap<u64, Trailer> = [1, 2, 3, 4, 5, 6, 8]
            .into_iter()
            .map(|id| (id, trailer_for(&format!("https://cdn/{id}.mp4"))))
            .chain([(4, trailer_for("https://cdn/403.mp4"))])
            .collect();
        let skip = HashSet::from([6]); // 6: swiped
        let mut log = Vec::new();
        let out = pass(
            &media,
            &ud,
            &mut scenes,
            &trailers,
            &skip,
            2,
            &mut Pacer::new(Duration::ZERO),
            now,
            &mut log,
        );
        // Tried: 1 (frames), 4 (403); 7 has no trailer (fails without a try); 8 is past the
        // limit.
        assert_eq!(
            out,
            Outcome {
                made: 1,
                failed: 2,
                left: 1
            }
        );
        let f = scenes[0].frames.as_ref().unwrap();
        assert_eq!(
            (f.at.as_str(), f.source.as_str(), f.files.len()),
            (timefmt::iso_utc(now).as_str(), "slr: 1.mp4", 3)
        );
        assert!(ud.join("scout-frames/slr/1/75.jpg").is_file());
        assert!(scenes[1].frames.is_none() && scenes[1].frames_failed.is_none());
        assert_eq!(scenes[2].frames_failed.as_ref().unwrap().at, day(3));
        let failed = scenes[3].frames_failed.as_ref().unwrap();
        assert!(failed.reason.contains("403") && failed.at == timefmt::iso_utc(now));
        assert_eq!(scenes[4].frames.as_ref(), Some(&kept));
        assert!(scenes[5].frames.is_none() && scenes[5].frames_failed.is_none());
        assert_eq!(
            scenes[6].frames_failed.as_ref().unwrap().reason,
            "no free trailer"
        );
        assert!(scenes[7].frames.is_none() && scenes[7].frames_failed.is_none());
        let urls: HashSet<String> = media.grabs.borrow().iter().map(|g| g.0.clone()).collect();
        assert_eq!(urls, HashSet::from(["https://cdn/1.mp4".to_string()]));
        assert!(
            String::from_utf8(log)
                .unwrap()
                .contains("frames 1: 3 from 1.mp4")
        );

        // A success clears an old failure.
        scenes[7].frames_failed = Some(FramesFailed {
            at: day(10),
            reason: "x".into(),
        });
        let out = pass(
            &media,
            &ud,
            &mut scenes[7..],
            &trailers,
            &skip,
            5,
            &mut Pacer::new(Duration::ZERO),
            now,
            &mut Vec::new(),
        );
        assert_eq!(out.made, 1);
        assert!(scenes[7].frames.is_some() && scenes[7].frames_failed.is_none());
        let _ = fs::remove_dir_all(&ud);
    }

    #[test]
    fn waiting_posts_cover_their_slr_scenes() {
        let ud = temp_dir("covered");
        assert!(covered(&ud).is_empty());
        fs::write(ud.join(WAITING_FILE), b"not json").unwrap();
        assert!(covered(&ud).is_empty());
        let waiting = json!({"candidates": [
            {"id": 1, "scenes": [
                {"video": {"host": "slr", "slrSceneId": 66249}},
                {"video": {"host": "MEGA", "slrSceneId": 5}},
                {"video": {"host": "slr", "slrSceneId": "77"}},
                {"video": {"host": "slr"}}]},
            {"id": 2},
            {"id": 3, "scenes": [{"base": "x"}]}
        ]});
        fs::write(ud.join(WAITING_FILE), waiting.to_string()).unwrap();
        assert_eq!(covered(&ud), HashSet::from([66249, 77]));
        let _ = fs::remove_dir_all(&ud);
    }

    #[test]
    fn keeps_frames_while_a_file_is_on_disk() {
        let ud = temp_dir("keep");
        let frames = |files: &[&str]| Frames {
            at: "2026-11-14T00:00:00.000Z".into(),
            source: "slr: x.mp4".into(),
            files: files.iter().map(|f| f.to_string()).collect(),
        };
        let mut old: Vec<Scene> = (1..=3).map(scene).collect();
        old[0].frames = Some(frames(&["25.jpg", "50.jpg"]));
        old[1].frames = Some(frames(&["25.jpg"]));
        old[2].frames_failed = Some(FramesFailed {
            at: "2026-11-14T00:00:00.000Z".into(),
            reason: "no free trailer".into(),
        });
        fs::create_dir_all(scene_dir(&ud, 1)).unwrap();
        fs::write(scene_dir(&ud, 1).join("50.jpg"), b"x").unwrap();
        let previous = ScoutFile {
            generated_at: String::new(),
            actors: vec![],
            quota: None,
            quota_error: None,
            scenes: old,
        };
        let mut scenes: Vec<Scene> = (1..=4).map(scene).collect();
        keep(Some(&previous), &mut scenes, &ud);
        assert_eq!(scenes[0].frames, Some(frames(&["25.jpg", "50.jpg"])));
        // 2's file is gone: due again.
        assert_eq!(scenes[1].frames, None);
        assert_eq!(
            scenes[2].frames_failed.as_ref().unwrap().reason,
            "no free trailer"
        );
        assert!(scenes[3].frames.is_none() && scenes[3].frames_failed.is_none());
        keep(None, &mut scenes, &ud);
        assert!(scenes[0].frames.is_some());
        let _ = fs::remove_dir_all(&ud);
    }

    fn has_ffmpeg() -> bool {
        ["ffmpeg", "ffprobe"].iter().all(|b| {
            Command::new(b)
                .arg("-version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
    }

    fn dimensions(image: &Path) -> String {
        let out = Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", "v:0"])
            .args(["-show_entries", "stream=width,height", "-of", "csv=p=0:s=x"])
            .arg(image)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[test]
    fn ffmpeg_takes_the_left_eye_of_a_test_video() {
        if !has_ffmpeg() {
            eprintln!("ffmpeg/ffprobe not found; skipping the real frames test");
            return;
        }
        let dir = temp_dir("ffmpeg");
        let video = dir.join("trailer_LR.mp4");
        let status = Command::new("ffmpeg")
            .args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi"])
            .args(["-i", "testsrc=duration=4:size=2720x320:rate=10"])
            .args(["-pix_fmt", "yuv420p"])
            .arg(&video)
            .status()
            .unwrap();
        assert!(status.success());
        let url = video.to_str().unwrap();
        assert!((Ffmpeg.duration(url).unwrap() - 4.0).abs() < 0.2);
        let t = Trailer {
            url: url.into(),
            name: file_name(url),
            crop: Crop::Left,
        };
        let out = dir.join("frames");
        assert_eq!(
            take(&Ffmpeg, &out, &t).unwrap(),
            ["25.jpg", "50.jpg", "75.jpg"]
        );
        // The left half (1360 px) scaled to 1280 px wide.
        assert_eq!(dimensions(&out.join("50.jpg")), "1280x302");
        let top = Trailer {
            crop: Crop::Top,
            ..t.clone()
        };
        let top_dir = dir.join("top");
        take(&Ffmpeg, &top_dir, &top).unwrap();
        assert_eq!(dimensions(&top_dir.join("25.jpg")), "1280x76");
        let e = Ffmpeg
            .duration(dir.join("missing.mp4").to_str().unwrap())
            .unwrap_err();
        assert!(!e.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
