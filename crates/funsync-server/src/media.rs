//! Video metadata (ffprobe) and generated thumbnails (ffmpeg).
//!
//! The thumbnail rule is the old backend's (`backend/services/ffmpeg.py`,
//! `generate_single_thumbnail`): a frame 10 % in (at least 10 s in for
//! videos over a minute), retried at 25 / 40 / 60 % when it is dark, blown
//! out, flat or a title card, the best-scoring try kept, all inside one
//! 25 s budget. Cache names match the old `thumb-cache` so the two caches
//! could be shared later.

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
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

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Metadata {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
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
/// the old cache.
pub fn thumb_file_name(video_id: &str, stat_key: &str, crop: Option<&str>) -> String {
    format!(
        "{video_id}_{stat_key}_single_{THUMB_MARKER}_{:04}_{THUMB_WIDTH}{}.jpg",
        (THUMB_SEEK_PCT * 1000.0) as u32,
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

pub struct Media {
    thumb_dir: PathBuf,
    /// path → (stat key, metadata): a replaced file is probed again.
    meta: Mutex<HashMap<String, (String, Metadata)>>,
    slots: Semaphore,
    ffmpeg: String,
    ffprobe: String,
}

impl Media {
    pub fn new(thumb_dir: PathBuf) -> Self {
        Self {
            thumb_dir,
            meta: Mutex::new(HashMap::new()),
            slots: Semaphore::new(4),
            ffmpeg: find_binary("ffmpeg"),
            ffprobe: find_binary("ffprobe"),
        }
    }

    pub async fn metadata(&self, path: &Path) -> Result<Metadata, String> {
        let p = path.to_string_lossy().into_owned();
        let key = stat_key(path).map_err(|e| e.to_string())?;
        if let Some((k, m)) = self.meta.lock().unwrap().get(&p)
            && *k == key
        {
            return Ok(*m);
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
        self.meta.lock().unwrap().insert(p, (key, m));
        Ok(m)
    }

    /// Metadata probed earlier, without probing or statting the file.
    pub fn cached(&self, path: &str) -> Option<Metadata> {
        self.meta.lock().unwrap().get(path).map(|(_, m)| *m)
    }

    /// The cached thumbnail of a video, generated when missing; `crop` is a
    /// VR crop (`vr::thumb_crop`).
    pub async fn thumbnail(
        &self,
        video_id: &str,
        path: &Path,
        crop: Option<&str>,
    ) -> Result<PathBuf, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("video not found: {e}"))?;
        if meta.len() == 0 {
            return Err("video is empty (0 bytes)".into());
        }
        let out = self.thumb_dir.join(thumb_file_name(
            video_id,
            &stat_key(path).map_err(|e| e.to_string())?,
            crop,
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
        for (i, seek) in seek_positions(md.duration).into_iter().enumerate() {
            let budget = deadline.saturating_duration_since(Instant::now());
            if budget < Duration::from_secs(1) {
                break;
            }
            let tmp = tmp_dir.join(format!("funsync-rust-thumb-{video_id}-{i}.jpg"));
            match self.grab_frame(path, seek, crop, &tmp, budget).await {
                Ok(stats) => {
                    attempts.push((frame_score(stats), tmp));
                    if frame_problem(stats).is_none() {
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
    ) -> Result<Option<FrameStats>, String> {
        let s = THUMB_STATS_SIZE;
        let graph = format!(
            "[0:v]{}scale={THUMB_WIDTH}:-2,split=2[o][s];[s]scale={s}:{s}:flags=area,format=gray[g]",
            crop_filters(crop)
        );
        let child = Command::new(&self.ffmpeg)
            .args([
                "-v",
                "error",
                "-skip_frame",
                "nokey",
                "-ss",
                &format!("{seek}"),
                "-i",
            ])
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
    Ok(Metadata {
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
            br#"{"streams":[{"codec_type":"audio"},{"codec_type":"video","width":3840,"height":1920}],
                "format":{"duration":"125.5"}}"#,
        )
        .unwrap();
        assert_eq!(
            m,
            Metadata {
                duration: 125.5,
                width: 3840,
                height: 1920
            }
        );
    }
}
