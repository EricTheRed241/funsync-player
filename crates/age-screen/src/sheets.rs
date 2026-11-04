//! Contact sheets with ffmpeg (12 frames evenly spread over the video, 400 px
//! wide, the left eye of VR, tiled 4×3) and the judge's batch manifests.

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

pub const FRAMES: usize = 12;
pub const FRAME_WIDTH: u32 = 400;
pub const TILE: &str = "tile=4x3:padding=4";
pub const BATCH_SIZE: usize = 15;
const WORKERS: usize = 4;

/// One video to make a sheet of.
#[derive(Debug, Clone)]
pub struct Job {
    /// File-safe id (sheet `<work>/<file_id>.jpg`).
    pub file_id: String,
    pub path: String,
    /// Seconds; 0 = ask ffprobe.
    pub duration: f64,
    pub vr: bool,
}

/// A backend id as a file name: anything outside `[A-Za-z0-9_-]` becomes
/// `_`.
pub fn file_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The frame times: the middle of each of 12 equal slices.
pub fn frame_times(duration: f64) -> Vec<f64> {
    (0..FRAMES)
        .map(|i| duration * (i as f64 + 0.5) / FRAMES as f64)
        .collect()
}

/// The ffmpeg filter for one frame: VR keeps the left eye.
pub fn frame_filter(vr: bool) -> String {
    let scale = format!("scale={FRAME_WIDTH}:-2");
    if vr {
        format!("crop=iw/2:ih:0:0,{scale}")
    } else {
        scale
    }
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("can't run ffmpeg/ffprobe ({e}); is it installed?"))?;
    if out.status.success() {
        Ok(())
    } else {
        let msg = String::from_utf8_lossy(&out.stderr);
        let last = msg
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        Err(format!("{} ({last})", out.status))
    }
}

fn probe_duration(path: &str) -> Result<f64, String> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("can't run ffprobe ({e})"))?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|d| d.is_finite() && *d > 0.0)
        .ok_or_else(|| "unknown duration".to_string())
}

/// Make `<work>/<file_id>.jpg`. Frames ffmpeg can't decode are left out; a
/// sheet needs at least one.
pub fn make_sheet(work: &Path, job: &Job) -> Result<PathBuf, String> {
    let duration = if job.duration > 0.0 {
        job.duration
    } else {
        probe_duration(&job.path)?
    };
    let frames_dir = work.join("frames").join(&job.file_id);
    let _ = fs::remove_dir_all(&frames_dir);
    fs::create_dir_all(&frames_dir).map_err(|e| format!("{}: {e}", frames_dir.display()))?;
    let filter = frame_filter(job.vr);
    let mut made = 0;
    let mut last_err = String::new();
    for t in frame_times(duration) {
        let out = frames_dir.join(format!("{:02}.jpg", made + 1));
        let result = run(Command::new("ffmpeg")
            .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"])
            .args(["-ss", &format!("{t:.3}")])
            .arg("-i")
            .arg(&job.path)
            .args(["-frames:v", "1", "-vf", &filter, "-q:v", "4"])
            .arg(&out));
        match result {
            Ok(()) if out.is_file() => made += 1,
            Ok(()) => last_err = "no frame".into(),
            Err(e) => last_err = e,
        }
    }
    if made == 0 {
        let _ = fs::remove_dir_all(&frames_dir);
        return Err(format!("no frames ({last_err})"));
    }
    let sheet = work.join(format!("{}.jpg", job.file_id));
    let result = run(Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"])
        .args(["-framerate", "1", "-start_number", "1", "-i"])
        .arg(frames_dir.join("%02d.jpg"))
        .args(["-vf", TILE, "-frames:v", "1", "-q:v", "3"])
        .arg(&sheet));
    let _ = fs::remove_dir_all(&frames_dir);
    result.map_err(|e| format!("tiling failed: {e}"))?;
    if !sheet.is_file() {
        return Err("tiling made no sheet".into());
    }
    Ok(sheet)
}

/// Make every job's sheet, a few ffmpeg runs at a time. `done` is called on
/// this thread as each finishes (job index, result).
pub fn make_sheets(
    work: &Path,
    jobs: &[Job],
    mut done: impl FnMut(usize, Result<PathBuf, String>),
) {
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, WORKERS)
        .min(jobs.len().max(1));
    let (tx, rx) = mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let tx = tx.clone();
            let next = &next;
            scope.spawn(move || {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(job) = jobs.get(i) else { break };
                    if tx.send((i, make_sheet(work, job))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        for (i, result) in rx {
            done(i, result);
        }
    });
    let _ = fs::remove_dir_all(work.join("frames"));
}

/// One manifest entry.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ManifestEntry {
    pub id: String,
    pub title: String,
    pub sheet: String,
}

/// A written batch: `<work>/batches/NN.json`, results expected at
/// `<work>/results/NN.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    pub name: String,
    pub manifest: PathBuf,
    pub results: PathBuf,
    pub entries: Vec<ManifestEntry>,
}

/// Split the entries into batches of 15, write the manifests, create the
/// results folder.
pub fn write_batches(work: &Path, entries: Vec<ManifestEntry>) -> Result<Vec<Batch>, String> {
    let batches_dir = work.join("batches");
    let results_dir = work.join("results");
    for dir in [&batches_dir, &results_dir] {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    entries
        .chunks(BATCH_SIZE)
        .enumerate()
        .map(|(i, chunk)| {
            let name = format!("{:02}", i + 1);
            let manifest = batches_dir.join(format!("{name}.json"));
            let json = serde_json::to_string_pretty(chunk).map_err(|e| e.to_string())?;
            fs::write(&manifest, json + "\n")
                .map_err(|e| format!("{}: {e}", manifest.display()))?;
            Ok(Batch {
                results: results_dir.join(format!("{name}.json")),
                name,
                manifest,
                entries: chunk.to_vec(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_spread_over_the_whole_video() {
        let t = frame_times(120.0);
        assert_eq!(t.len(), 12);
        assert_eq!(t[0], 5.0);
        assert_eq!(t[11], 115.0);
        assert_eq!(frame_filter(true), "crop=iw/2:ih:0:0,scale=400:-2");
        assert_eq!(frame_filter(false), "scale=400:-2");
        assert_eq!(file_id("ab/../c d"), "ab____c_d");
    }

    #[test]
    fn batches_of_fifteen() {
        let work = std::env::temp_dir().join(format!("age-screen-batches-{}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        let entries: Vec<ManifestEntry> = (0..31)
            .map(|i| ManifestEntry {
                id: i.to_string(),
                title: format!("t{i}"),
                sheet: format!("/w/{i}.jpg"),
            })
            .collect();
        let batches = write_batches(&work, entries).unwrap();
        let sizes: Vec<usize> = batches.iter().map(|b| b.entries.len()).collect();
        assert_eq!(sizes, vec![15, 15, 1]);
        assert_eq!(batches[2].name, "03");
        assert!(batches[2].results.ends_with("results/03.json"));
        let written: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&batches[2].manifest).unwrap()).unwrap();
        assert_eq!(
            written,
            serde_json::json!([{ "id": "30", "title": "t30", "sheet": "/w/30.jpg" }])
        );
        let _ = fs::remove_dir_all(&work);
    }
}
