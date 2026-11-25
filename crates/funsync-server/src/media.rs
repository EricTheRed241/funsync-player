//! Video metadata (ffprobe) and generated thumbnails (ffmpeg).
//!
//! The thumbnail rule is the old backend's (`backend/services/ffmpeg.py`,
//! `generate_single_thumbnail`): a frame 10 % in (at least 10 s in for
//! videos over a minute), retried at 25 / 40 / 60 % when it is dark, blown
//! out, flat or a title card, the best-scoring try kept, all inside one
//! 25 s budget. Cache names match the old `thumb-cache` so the two caches
//! could be shared later.

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::Semaphore;

pub const THUMB_MARKER: &str = "v5";
const THUMB_STATS_SIZE: usize = 32;
const THUMB_DARK_MEAN: f64 = 16.0;
const THUMB_BRIGHT_MEAN: f64 = 240.0;
const THUMB_FLAT_SD: f64 = 8.0;
const THUMB_CARD_SHARE: f64 = 0.85;
const THUMB_CARD_NEAR: usize = 10;
const THUMB_RETRY_PCTS: [f64; 3] = [0.25, 0.40, 0.60];
const THUMB_TIMEOUT: Duration = Duration::from_secs(25);
pub const THUMB_WIDTH: u32 = 320;
const THUMB_SEEK_PCT: f64 = 0.1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Metadata {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    /// The video stream's codec as the headsets name it (`video_info.py`):
    /// `h265`, `h264`, else ffprobe's name.
    #[serde(default = "h264")]
    pub codec: String,
}

fn h264() -> String {
    "h264".into()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameStats {
    pub mean: f64,
    pub sd: f64,
    pub dominant: f64,
}

pub fn frame_stats(gray: &[u8]) -> Option<FrameStats> {
    let n = gray.len();
    if n == 0 {
        return None;
    }
    let mut hist = [0usize; 256];
    for &b in gray {
        hist[b as usize] += 1;
    }
    let mean = gray.iter().map(|&b| b as f64).sum::<f64>() / n as f64;
    let var = hist
        .iter()
        .enumerate()
        .map(|(v, &c)| c as f64 * (v as f64 - mean).powi(2))
        .sum::<f64>()
        / n as f64;
    let dominant = (0..256usize)
        .map(|v| {
            hist[v.saturating_sub(THUMB_CARD_NEAR)..(v + THUMB_CARD_NEAR + 1).min(256)]
                .iter()
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0) as f64
        / n as f64;
    Some(FrameStats {
        mean,
        sd: var.sqrt(),
        dominant,
    })
}

/// Why a frame makes a bad thumbnail, or None when it's fine.
pub fn frame_problem(s: Option<FrameStats>) -> Option<&'static str> {
    let s = s?;
    if s.mean < THUMB_DARK_MEAN {
        Some("dark")
    } else if s.mean > THUMB_BRIGHT_MEAN {
        Some("bright")
    } else if s.sd < THUMB_FLAT_SD {
        Some("flat")
    } else if s.dominant >= THUMB_CARD_SHARE {
        Some("title-card")
    } else {
        None
    }
}

fn frame_score(s: Option<FrameStats>) -> f64 {
    s.map_or(0.0, |s| s.sd * (1.0 - s.dominant))
}

/// Seek positions in seconds for an automatic (not pinned) frame.
pub fn seek_positions(duration: f64) -> Vec<f64> {
    let first = if duration > 60.0 {
        (duration * THUMB_SEEK_PCT).max(10.0)
    } else {
        duration * THUMB_SEEK_PCT
    };
    std::iter::once(first)
        .chain(THUMB_RETRY_PCTS.iter().map(|p| duration * p))
        .collect()
}

/// md5(size:mtime_ns)[:8]: a replaced file gets a new thumbnail.
pub fn stat_key(path: &Path) -> std::io::Result<String> {
    let m = std::fs::metadata(path)?;
    let ns = m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128;
    Ok(format!("{:x}", md5::compute(format!("{}:{}", m.len(), ns)))[..8].to_string())
}

/// `crop` is a `thumb_crop` key (`vr::thumb_crop`), part of the name as in
/// the old cache; `pin` a frame the user pinned (0–0.999 of the length),
/// marked `e` (exact).
pub fn thumb_file_name(
    video_id: &str,
    stat_key: &str,
    crop: Option<&str>,
    pin: Option<f64>,
) -> String {
    format!(
        "{video_id}_{stat_key}_single_{THUMB_MARKER}{}_{:04}_{THUMB_WIDTH}{}.jpg",
        if pin.is_some() { "e" } else { "" },
        (pin.unwrap_or(THUMB_SEEK_PCT) * 1000.0) as u32,
        crop.map(|c| format!("_{c}")).unwrap_or_default()
    )
}

/// The ffmpeg filters before scaling (`THUMB_VR_CROPS`): one eye, then for
/// a `…c` crop the middle of it.
fn crop_filters(crop: Option<&str>) -> &'static str {
    match crop {
        Some("sbs") => "crop=iw/2:ih:0:0,",
        Some("sbsc") => "crop=iw/2:ih:0:0,crop=trunc(iw*0.35)*2:trunc(ih*0.25)*2,",
        Some("tb") => "crop=iw:ih/2:0:0,",
        Some("tbc") => "crop=iw:ih/2:0:0,crop=trunc(iw*0.35)*2:trunc(ih*0.25)*2,",
        _ => "",
    }
}

/// A generated thumbnail's name (`_THUMB_FILE_RE`): video id first.
static THUMB_NAME: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^([0-9a-f]{12})_([0-9a-f]{8})_single_(v\d+)e?_\d{4}_\d+(?:_[a-z]+)?\.jpg$")
        .unwrap()
});

pub struct Media {
    thumb_dir: PathBuf,
    /// path → (stat key, metadata): a replaced file is probed again.
    meta: Mutex<HashMap<String, (String, Metadata)>>,
    /// Where `meta` is kept between runs (`with_cache`).
    cache_file: Option<PathBuf>,
    slots: Semaphore,
    ffmpeg: String,
    ffprobe: String,
}

impl Media {
    pub fn new(thumb_dir: PathBuf) -> Self {
        Self {
            thumb_dir,
            meta: Mutex::new(HashMap::new()),
            cache_file: None,
            slots: Semaphore::new(4),
            ffmpeg: find_binary("ffmpeg"),
            ffprobe: find_binary("ffprobe"),
        }
    }

    /// Keep the probed metadata in `file` (read now, written by
    /// `save_cache`), so frame sizes survive restarts.
    pub fn with_cache(mut self, file: PathBuf) -> Self {
        if let Some(m) = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
        {
            self.meta = Mutex::new(m);
        }
        self.cache_file = Some(file);
        self
    }

    pub fn save_cache(&self) {
        if let Some(file) = &self.cache_file {
            let body = serde_json::to_vec(&*self.meta.lock().unwrap()).unwrap_or_default();
            let _ = crate::data::write_atomic(file, &body);
        }
    }

    /// Run ffmpeg with `args`, blocking: its output when it exits 0 within
    /// `timeout` (killed after that).
    pub fn ffmpeg_blocking<I, S>(&self, args: I, timeout: Duration) -> Option<Vec<u8>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut child = std::process::Command::new(&self.ffmpeg)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut out = child.stdout.take()?;
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut out, &mut buf).map(|_| buf)
        });
        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(s) = child.try_wait().ok()? {
                break s;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let bytes = reader.join().ok()?.ok()?;
        status.success().then_some(bytes)
    }

    /// Delete generated thumbnails no video uses (`prune_thumb_cache`):
    /// `keep` maps every video id to the file names it uses now, or to none
    /// when its file can't be read right now (it keeps whatever it has).
    /// Any other JPEG in the cache goes. Returns how many were deleted.
    pub fn prune(&self, keep: &HashMap<String, Option<HashSet<String>>>) -> usize {
        let Ok(dir) = std::fs::read_dir(&self.thumb_dir) else {
            return 0;
        };
        let mut removed = 0;
        for name in dir
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
        {
            if !name.ends_with(".jpg") {
                continue;
            }
            let kept = THUMB_NAME
                .captures(&name)
                .and_then(|c| keep.get(&c[1]))
                .is_some_and(|used| used.as_ref().is_none_or(|u| u.contains(&name)));
            if !kept && std::fs::remove_file(self.thumb_dir.join(&name)).is_ok() {
                removed += 1;
            }
        }
        removed
    }

    pub async fn metadata(&self, path: &Path) -> Result<Metadata, String> {
        let p = path.to_string_lossy().into_owned();
        let key = stat_key(path).map_err(|e| e.to_string())?;
        if let Some((k, m)) = self.meta.lock().unwrap().get(&p)
            && *k == key
        {
            return Ok(m.clone());
        }
        let out = tokio::time::timeout(
            THUMB_TIMEOUT,
            Command::new(&self.ffprobe)
                .args([
                    "-v",
                    "error",
                    "-print_format",
                    "json",
                    "-show_format",
                    "-show_streams",
                ])
                .arg(path)
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| "ffprobe timed out".to_string())?
        .map_err(|e| format!("ffprobe: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "ffprobe failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let m = parse_probe(&out.stdout)?;
        self.meta.lock().unwrap().insert(p, (key, m.clone()));
        Ok(m)
    }

    /// Metadata probed earlier, without probing or statting the file.
    pub fn cached(&self, path: &str) -> Option<Metadata> {
        self.meta.lock().unwrap().get(path).map(|(_, m)| m.clone())
    }

    /// The cached thumbnail of a video, generated when missing; `crop` is a
    /// VR crop (`vr::thumb_crop`). A pinned frame (`pin`) is grabbed exactly
    /// there: no keyframe skipping, no 10 s minimum, no retries.
    pub async fn thumbnail(
        &self,
        video_id: &str,
        path: &Path,
        crop: Option<&str>,
        pin: Option<f64>,
    ) -> Result<PathBuf, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("video not found: {e}"))?;
        if meta.len() == 0 {
            return Err("video is empty (0 bytes)".into());
        }
        let out = self.thumb_dir.join(thumb_file_name(
            video_id,
            &stat_key(path).map_err(|e| e.to_string())?,
            crop,
            pin,
        ));
        if out.exists() {
            return Ok(out);
        }
        let _slot = self.slots.acquire().await.map_err(|e| e.to_string())?;
        if out.exists() {
            return Ok(out);
        }
        let md = self.metadata(path).await?;
        if md.duration <= 0.0 {
            return Err("video has no duration".into());
        }
        let deadline = Instant::now() + THUMB_TIMEOUT;
        let tmp_dir = std::env::temp_dir();
        let mut attempts: Vec<(f64, PathBuf)> = Vec::new();
        let mut result = Ok(());
        let positions = match pin {
            Some(p) => vec![md.duration * p],
            None => seek_positions(md.duration),
        };
        for (i, seek) in positions.into_iter().enumerate() {
            let budget = deadline.saturating_duration_since(Instant::now());
            if budget < Duration::from_secs(1) {
                break;
            }
            let tmp = tmp_dir.join(format!("funsync-rust-thumb-{video_id}-{i}.jpg"));
            match self
                .grab_frame(path, seek, crop, &tmp, budget, pin.is_some())
                .await
            {
                Ok(stats) => {
                    attempts.push((frame_score(stats), tmp));
                    if pin.is_some() || frame_problem(stats).is_none() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    if i == 0 || e.contains("timed out") {
                        result = Err(e);
                        break;
                    }
                }
            }
        }
        let best = attempts
            .iter()
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|a| a.1.clone());
        let saved = match (best, result) {
            (Some(best), _) => std::fs::create_dir_all(&self.thumb_dir)
                .and_then(|_| {
                    let part = out.with_extension("jpg.part");
                    std::fs::copy(&best, &part)?;
                    std::fs::rename(&part, &out)
                })
                .map_err(|e| e.to_string()),
            (None, Err(e)) => Err(e),
            (None, Ok(())) => Err("no frame".into()),
        };
        for (_, tmp) in &attempts {
            let _ = std::fs::remove_file(tmp);
        }
        saved.map(|_| out)
    }

    async fn grab_frame(
        &self,
        path: &Path,
        seek: f64,
        crop: Option<&str>,
        out: &Path,
        budget: Duration,
        exact: bool,
    ) -> Result<Option<FrameStats>, String> {
        let s = THUMB_STATS_SIZE;
        let graph = format!(
            "[0:v]{}scale={THUMB_WIDTH}:-2,split=2[o][s];[s]scale={s}:{s}:flags=area,format=gray[g]",
            crop_filters(crop)
        );
        // Exact: `-ss` before `-i` without keyframe skipping is frame-accurate.
        let skip: &[&str] = if exact {
            &[]
        } else {
            &["-skip_frame", "nokey"]
        };
        let child = Command::new(&self.ffmpeg)
            .args(["-v", "error"])
            .args(skip)
            .args(["-ss", &format!("{seek}"), "-i"])
            .arg(path)
            .args([
                "-filter_complex",
                &graph,
                "-map",
                "[o]",
                "-frames:v",
                "1",
                "-q:v",
                "5",
                "-y",
            ])
            .arg(out)
            .args(["-map", "[g]", "-frames:v", "1", "-f", "rawvideo", "pipe:1"])
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output();
        let res = tokio::time::timeout(budget, child)
            .await
            .map_err(|_| format!("ffmpeg timed out after {}s", budget.as_secs()))?
            .map_err(|e| format!("ffmpeg: {e}"))?;
        if !res.status.success() || !out.exists() {
            return Err(format!(
                "ffmpeg failed: {}",
                String::from_utf8_lossy(&res.stderr)
            ));
        }
        Ok(if res.stdout.len() == s * s {
            frame_stats(&res.stdout)
        } else {
            None
        })
    }
}

fn parse_probe(json: &[u8]) -> Result<Metadata, String> {
    let v: serde_json::Value = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    let num = |x: Option<&serde_json::Value>| -> f64 {
        x.and_then(|x| {
            x.as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| x.as_f64())
        })
        .unwrap_or(0.0)
    };
    let video = v["streams"]
        .as_array()
        .and_then(|a| a.iter().find(|s| s["codec_type"] == "video"));
    let mut duration = num(v["format"].get("duration"));
    if duration <= 0.0 {
        duration = num(video.and_then(|s| s.get("duration")));
    }
    let codec = match video
        .and_then(|s| s["codec_name"].as_str())
        .unwrap_or("h264")
    {
        "hevc" | "h265" => "h265",
        "h264" | "avc" | "avc1" => "h264",
        c => c,
    };
    Ok(Metadata {
        codec: codec.into(),
        duration,
        width: video.and_then(|s| s["width"].as_u64()).unwrap_or(0) as u32,
        height: video.and_then(|s| s["height"].as_u64()).unwrap_or(0) as u32,
    })
}

/// Homebrew's ffmpeg isn't on a launchd PATH.
fn find_binary(name: &str) -> String {
    for dir in ["/opt/homebrew/bin", "/usr/local/bin"] {
        let p = Path::new(dir).join(name);
        if p.exists() {
            return p.to_string_lossy().into_owned();
        }
    }
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frame_checks() {
        assert_eq!(frame_problem(frame_stats(&[5; 1024])), Some("dark"));
        assert_eq!(frame_problem(frame_stats(&[250; 1024])), Some("bright"));
        assert_eq!(frame_problem(frame_stats(&[128; 1024])), Some("flat"));
        // 90 % one grey level with some text: a title card.
        let mut card = vec![30u8; 1024];
        for (i, px) in card.iter_mut().enumerate().take(100) {
            *px = 200 + (i % 50) as u8;
        }
        assert_eq!(frame_problem(frame_stats(&card)), Some("title-card"));
        let busy: Vec<u8> = (0..1024).map(|i| (i * 37 % 256) as u8).collect();
        assert_eq!(frame_problem(frame_stats(&busy)), None);
    }

    #[test]
    fn seek_rule() {
        assert_eq!(seek_positions(50.0)[0], 5.0);
        assert_eq!(seek_positions(61.0)[0], 10.0);
        assert_eq!(seek_positions(600.0), vec![60.0, 150.0, 240.0, 360.0]);
    }

    #[test]
    fn probe_parse() {
        let m = parse_probe(
            br#"{"streams":[{"codec_type":"audio"},{"codec_type":"video","codec_name":"hevc","width":3840,"height":1920}],
                "format":{"duration":"125.5"}}"#,
        )
        .unwrap();
        let want = Metadata {
            duration: 125.5,
            width: 3840,
            height: 1920,
            codec: "h265".into(),
        };
        assert_eq!(m, want);
        let codec = |c: &str| {
            let json = format!(r#"{{"streams":[{{"codec_type":"video","codec_name":"{c}"}}]}}"#);
            parse_probe(json.as_bytes()).unwrap().codec
        };
        assert_eq!((codec("avc1"), codec("vp9")), ("h264".into(), "vp9".into()));
    }

    #[test]
    fn pinned_frames_have_their_own_exact_names() {
        let auto = thumb_file_name("aaaaaaaaaaaa", "11111111", None, None);
        let pinned = thumb_file_name("aaaaaaaaaaaa", "11111111", Some("sbs"), Some(0.4567));
        assert_eq!(
            auto,
            format!("aaaaaaaaaaaa_11111111_single_{THUMB_MARKER}_0100_320.jpg")
        );
        assert_eq!(
            pinned,
            format!("aaaaaaaaaaaa_11111111_single_{THUMB_MARKER}e_0456_320_sbs.jpg")
        );
        assert!(THUMB_NAME.is_match(&pinned), "pruning knows them");
    }

    #[test]
    fn prune_keeps_what_videos_use() {
        let d = tempfile::tempdir().unwrap();
        let media = Media::new(d.path().to_path_buf());
        let name = |id: &str, key: &str, crop| thumb_file_name(id, key, crop, None);
        let files = [
            name("aaaaaaaaaaaa", "11111111", None),
            name("aaaaaaaaaaaa", "22222222", None),
            name("aaaaaaaaaaaa", "11111111", Some("sbs")),
            name("bbbbbbbbbbbb", "33333333", None),
            name("cccccccccccc", "44444444", None),
            "aaaaaaaaaaaa.jpg".into(),
            "notes.txt".into(),
        ];
        for f in &files {
            std::fs::write(d.path().join(f), b"x").unwrap();
        }
        let keep = HashMap::from([
            (
                "aaaaaaaaaaaa".to_string(),
                Some(HashSet::from([files[0].clone()])),
            ),
            ("bbbbbbbbbbbb".to_string(), None),
        ]);
        assert_eq!(media.prune(&keep), 4);
        let left: HashSet<String> = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        let want: HashSet<String> = [&files[0], &files[3], &files[6]]
            .into_iter()
            .cloned()
            .collect();
        assert_eq!(
            left, want,
            "an older file, crop or name, and unknown videos go"
        );
    }

    #[test]
    fn probed_metadata_survives_a_restart() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("media-cache.json");
        // A cache written before codecs were kept: h264.
        let m = Metadata {
            duration: 12.5,
            width: 3840,
            height: 1920,
            codec: "h264".into(),
        };
        let old =
            json!({"/v.mp4": ["abcd1234", {"duration": 12.5, "width": 3840, "height": 1920}]});
        std::fs::write(&file, serde_json::to_vec(&old).unwrap()).unwrap();
        let media = Media::new(d.path().join("t")).with_cache(file.clone());
        assert_eq!(media.cached("/v.mp4"), Some(m.clone()));
        std::fs::remove_file(&file).unwrap();
        media.save_cache();
        assert_eq!(
            Media::new(d.path().join("t"))
                .with_cache(file)
                .cached("/v.mp4"),
            Some(m)
        );
    }
}
