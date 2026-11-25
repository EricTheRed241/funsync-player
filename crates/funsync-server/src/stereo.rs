//! The frame check of the VR decision (`backend/services/stereo_detect.py`):
//! for videos neither the user nor the file name decides, four grey
//! 128×128 frames (20 / 40 / 60 / 80 %) are compared half against half.
//! Per frame, the left half's horizontal detail (|Δx|) is correlated with
//! the right half's, allowing ±4 px of parallax; the second-lowest frame
//! score decides (≥ 0.3 side by side, ≤ 0.2 one picture, else undecided).
//! A picture between dark side bars that each fill ≥ 20 % of the width is
//! one picture. Results are kept per path with the file's size and mtime in
//! `<data dir>/vr-stereo-cache.json` (the old backend's file and shape), so
//! the decision survives restarts; `layout` never touches the disk.

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::media::Media;

pub const CACHE_FILE: &str = "vr-stereo-cache.json";
/// Bump when the analysis changes so old results are looked at again.
const VERSION: u64 = 1;
const SIZE: usize = 128;
const FRACTIONS: [f64; 4] = [0.2, 0.4, 0.6, 0.8];
const STEREO_MIN: f64 = 0.3;
const MONO_MAX: f64 = 0.2;
const SIDE_BAR_MIN: f64 = 0.2;
const FRAME_TIMEOUT: Duration = Duration::from_secs(60);

/// One frame's verdict: a picture between side bars, or a score.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Score {
    Mono,
    Value(f64),
}

fn corr(a: &[f64], b: &[f64]) -> Option<f64> {
    let n = a.len() as f64;
    if a.is_empty() {
        return None;
    }
    let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
    let va: f64 = a.iter().map(|x| (x - ma).powi(2)).sum();
    let vb: f64 = b.iter().map(|y| (y - mb).powi(2)).sum();
    if va <= 1e-9 || vb <= 1e-9 {
        return None;
    }
    let cov: f64 = a.iter().zip(b).map(|(x, y)| (x - ma) * (y - mb)).sum();
    Some(cov / (va * vb).sqrt())
}

/// Fraction of the width the dark, flat bars fill on the left / right.
fn side_bars(px: &[u8], w: usize, h: usize) -> (f64, f64) {
    let dead = |x: usize| {
        let col = (0..h).map(|y| px[y * w + x]);
        let (min, max) = col
            .clone()
            .fold((255u8, 0u8), |(a, b), v| (a.min(v), b.max(v)));
        max - min <= 12 && col.map(f64::from).sum::<f64>() / h as f64 <= 24.0
    };
    let left = (0..w).take_while(|&x| dead(x)).count();
    let right = (0..w).take_while(|&x| dead(w - 1 - x)).count();
    (left as f64 / w as f64, right as f64 / w as f64)
}

/// Side-by-side score of one grey `w`×`h` frame; none when it is too flat
/// to say (black, a fade).
pub fn frame_score(px: &[u8], w: usize, h: usize) -> Option<Score> {
    let (min, max) = px
        .iter()
        .fold((255u8, 0u8), |(a, b), &v| (a.min(v), b.max(v)));
    if max.saturating_sub(min) < 40 {
        return None;
    }
    let (left, right) = side_bars(px, w, h);
    if left.min(right) >= SIDE_BAR_MIN {
        return Some(Score::Mono);
    }
    let grad: Vec<f64> = (0..h)
        .flat_map(|y| {
            let row = &px[y * w..(y + 1) * w];
            row.windows(2)
                .map(|p| (f64::from(p[1]) - f64::from(p[0])).abs())
                .chain(std::iter::once(0.0))
        })
        .collect();
    let half = w / 2;
    (-4i64..=4)
        .filter_map(|shift| {
            let (mut a, mut b) = (Vec::new(), Vec::new());
            for y in 0..h {
                for x in 6..half - 6 {
                    a.push(grad[y * w + x]);
                    b.push(grad[((y * w + half + x) as i64 + shift) as usize]);
                }
            }
            corr(&a, &b)
        })
        .reduce(f64::max)
        .map(Score::Value)
}

/// `sbs` / `mono` / none (undecided) from the frames' scores.
pub fn layout_from_scores(scores: &[Option<Score>]) -> Option<&'static str> {
    let usable: Vec<Score> = scores.iter().flatten().copied().collect();
    let bars = usable.iter().filter(|s| **s == Score::Mono).count();
    if bars > 0 && bars * 2 >= usable.len() {
        return Some("mono");
    }
    let mut values: Vec<f64> = usable
        .iter()
        .filter_map(|s| match s {
            Score::Value(v) => Some(*v),
            Score::Mono => None,
        })
        .collect();
    if values.len() < 2 {
        return None;
    }
    values.sort_by(f64::total_cmp);
    match values[1] {
        d if d >= STEREO_MIN => Some("sbs"),
        d if d <= MONO_MAX => Some("mono"),
        _ => None,
    }
}

/// The layout of one video file. Blocking (ffmpeg).
fn analyze(media: &Media, path: &str, duration: f64) -> Option<&'static str> {
    if duration <= 0.0 {
        return None;
    }
    let scores: Vec<Option<Score>> = FRACTIONS
        .iter()
        .filter_map(|f| {
            let at = format!("{:.2}", duration * f);
            let scale = format!("scale={SIZE}:{SIZE}");
            let args = [
                "-v",
                "error",
                "-ss",
                &at,
                "-i",
                path,
                "-frames:v",
                "1",
                "-vf",
                &scale,
            ];
            let px = media.ffmpeg_blocking(
                args.iter()
                    .chain(&["-f", "rawvideo", "-pix_fmt", "gray", "-"]),
                FRAME_TIMEOUT,
            )?;
            (px.len() == SIZE * SIZE).then(|| frame_score(&px, SIZE, SIZE))
        })
        .collect();
    layout_from_scores(&scores)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    size: u64,
    mtime_ns: u64,
    layout: Option<String>,
    version: u64,
}

/// The cache file and its entries by path.
type Store = (Option<PathBuf>, HashMap<String, Entry>);
static STORE: LazyLock<Mutex<Store>> = LazyLock::new(Default::default);

fn sig(path: &str) -> Option<(u64, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((
        m.len(),
        m.mtime() as u64 * 1_000_000_000 + m.mtime_nsec() as u64,
    ))
}

/// The entries of a cache file of this analysis version.
fn read_file(file: &Path) -> HashMap<String, Entry> {
    let entries: HashMap<String, Entry> = std::fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|d| serde_json::from_value(d["entries"].clone()).ok())
        .unwrap_or_default();
    entries
        .into_iter()
        .filter(|(_, e)| e.version == VERSION)
        .collect()
}

fn write_file(file: &Path, entries: &HashMap<String, Entry>) {
    let body = json!({"version": 1, "entries": entries}).to_string();
    let _ = crate::data::write_atomic(file, body.as_bytes());
}

/// Read `<data_dir>/vr-stereo-cache.json` and keep results there from now on.
pub fn load(data_dir: &Path) {
    let file = data_dir.join(CACHE_FILE);
    *STORE.lock().unwrap() = (Some(file.clone()), read_file(&file));
}

fn save() {
    let store = STORE.lock().unwrap();
    if let Some(file) = &store.0 {
        write_file(file, &store.1);
    }
}

/// `sbs` (two eyes side by side), `mono` (one picture) or none (undecided,
/// or not looked at). Never touches the disk.
pub fn layout(path: &str) -> Option<&'static str> {
    match STORE.lock().unwrap().1.get(path)?.layout.as_deref() {
        Some("sbs") => Some("sbs"),
        Some("mono") => Some("mono"),
        _ => None,
    }
}

/// Set a layout without looking (tests).
pub fn set_layout(path: &str, layout: Option<&str>) {
    STORE.lock().unwrap().1.insert(
        path.into(),
        Entry {
            size: 0,
            mtime_ns: 0,
            layout: layout.map(Into::into),
            version: VERSION,
        },
    );
}

/// Look at every `(path, duration)` without a current result, one at a
/// time, saving every 20 and at the end. Blocking. Returns how many were
/// looked at.
pub fn check(media: &Media, targets: &[(String, f64)]) -> usize {
    let mut done = 0;
    for (path, duration) in targets {
        let Some((size, mtime_ns)) = sig(path) else {
            continue;
        };
        let current = STORE
            .lock()
            .unwrap()
            .1
            .get(path)
            .map(|e| (e.size, e.mtime_ns));
        if current == Some((size, mtime_ns)) {
            continue;
        }
        let layout = analyze(media, path, *duration).map(String::from);
        let entry = Entry {
            size,
            mtime_ns,
            layout,
            version: VERSION,
        };
        STORE.lock().unwrap().1.insert(path.clone(), entry);
        done += 1;
        if done % 20 == 0 {
            save();
        }
    }
    if done > 0 {
        save();
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 128×128 frame from a picture function of (x, y).
    fn frame(f: impl Fn(usize, usize) -> u8) -> Vec<u8> {
        (0..SIZE * SIZE).map(|i| f(i % SIZE, i / SIZE)).collect()
    }

    /// Noise: no two places alike.
    fn texture(x: usize, y: usize) -> u8 {
        (((x * 73_856_093) ^ (y * 19_349_663)) % 251) as u8
    }

    #[test]
    fn side_by_side_scores_high_and_one_picture_low() {
        // The same picture in both halves, the right eye 2 px over.
        let sbs = frame(|x, y| texture(x % 64 + if x >= 64 { 2 } else { 0 }, y));
        let Some(Score::Value(s)) = frame_score(&sbs, SIZE, SIZE) else {
            panic!()
        };
        assert!(s > 0.9, "{s}");
        let mono = frame(texture);
        let Some(Score::Value(m)) = frame_score(&mono, SIZE, SIZE) else {
            panic!()
        };
        assert!(m < 0.2, "{m}");
        assert_eq!(
            frame_score(&frame(|_, _| 100), SIZE, SIZE),
            None,
            "flat frame"
        );
        let bars = frame(|x, y| {
            if (30..98).contains(&x) {
                texture(x, y)
            } else {
                0
            }
        });
        assert_eq!(frame_score(&bars, SIZE, SIZE), Some(Score::Mono));
    }

    #[test]
    fn the_second_lowest_score_decides() {
        let v = |x| Some(Score::Value(x));
        assert_eq!(
            layout_from_scores(&[v(0.9), v(0.05), v(0.8), v(0.7)]),
            Some("sbs"),
            "one title card"
        );
        assert_eq!(
            layout_from_scores(&[v(0.9), v(0.1), v(0.15), v(0.7)]),
            Some("mono")
        );
        assert_eq!(
            layout_from_scores(&[v(0.9), v(0.25), v(0.25), None]),
            None,
            "in between"
        );
        assert_eq!(
            layout_from_scores(&[v(0.9), None, None, None]),
            None,
            "too few frames"
        );
        let bar = Some(Score::Mono);
        assert_eq!(
            layout_from_scores(&[bar, bar, v(0.9), v(0.9)]),
            Some("mono"),
            "side bars"
        );
        assert_eq!(
            layout_from_scores(&[bar, v(0.9), v(0.8), v(0.9)]),
            Some("sbs")
        );
    }

    #[test]
    fn results_are_cached_per_file_version() {
        let d = tempfile::tempdir().unwrap();
        let video = d.path().join("v.mp4");
        std::fs::write(&video, b"x").unwrap();
        let p = video.to_string_lossy().to_string();
        let media = Media::new(d.path().join("thumbs"));
        // No frames from a 1-byte file: looked at once, undecided, kept.
        assert_eq!(check(&media, &[(p.clone(), 10.0)]), 1);
        assert_eq!(check(&media, &[(p.clone(), 10.0)]), 0, "current entry");
        assert_eq!(layout(&p), None);
        std::fs::write(&video, b"xy").unwrap();
        assert_eq!(
            check(&media, &[(p.clone(), 10.0)]),
            1,
            "a replaced file is looked at again"
        );

        let file = d.path().join(CACHE_FILE);
        let entry = |layout: &str, version| Entry {
            size: 1,
            mtime_ns: 2,
            layout: Some(layout.into()),
            version,
        };
        let entries = HashMap::from([
            ("/a".to_string(), entry("sbs", VERSION)),
            ("/b".to_string(), entry("sbs", 0)),
        ]);
        write_file(&file, &entries);
        let back = read_file(&file);
        assert_eq!(back["/a"].layout.as_deref(), Some("sbs"));
        assert!(
            !back.contains_key("/b"),
            "an older analysis is looked at again"
        );
    }
}
