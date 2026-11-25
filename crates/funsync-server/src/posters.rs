//! Posters imported after each scan (ported from the old Electron app; see
//! `shared/faptap-thumbnail.js`, `rule34video-thumbnail.js`, `preview-match.js`):
//!
//! - **FapTap**: a FapTap scene's thumbnail (`scriptTags[path].previewUrl`
//!   of a `faptap-scout` record), only when it shows the video: its pHash is
//!   within 12 bits of a sampled frame's (every keyframe of the middle 90 %,
//!   else 12 seeks; the left eye too for side-by-side frames). Each settled
//!   attempt is kept in `library.faptapThumbnails` (`imported`, `no-match`,
//!   `gone`), so a link isn't fetched again; failures are retried.
//! - **rule34video**: the cover link the scan found in `eroscripts.json`
//!   (`posterUrl`), as is. Attempts in `library.rule34videoThumbnails`; a
//!   failure that may pass is retried after a day.
//!
//! Either only fills an empty slot or replaces an automatic poster (a
//! rule34video cover also replaces an EroScripts preview or FapTap
//! thumbnail); uploads and picks by hand stay. Images are saved as JPEG
//! (≤ 640 wide) in this server's `custom-thumbs/` and recorded in
//! `customThumbnails`. Blocking; network through a `Fetch` client that
//! never follows a redirect by itself, so every hop is checked against the
//! allowed hosts (`fetch_image`).

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::RwLock;

use crate::data::Library;
use crate::media::Media;
use crate::scan::{Video, is_rule34video_poster_url};

pub const FAPTAP_ATTEMPTS: &str = "faptapThumbnails";
pub const RULE34_ATTEMPTS: &str = "rule34videoThumbnails";
const RULE34_RETRY_MS: u64 = 24 * 60 * 60 * 1000;
const MAX_BYTES: u64 = 5 * 1024 * 1024;
const MAX_REDIRECTS: usize = 3;
const MAX_WIDTH: u32 = 640;
/// Bits of 64 two pHashes may differ by and still be the same picture.
pub const MATCH_THRESHOLD: u32 = 12;
const MIN_CONTRAST: f64 = 8.0;
const VR_MIN_ASPECT: f64 = 1.8;
const HASH: usize = 32;
const SAMPLE_COUNT: usize = 12;
const SAMPLE_WIDTH: u32 = 128;
const SAMPLE_EDGE: f64 = 0.05;
const SAMPLE_MAX_FRAMES: usize = 400;
const SAMPLE_PASS_MAX_BYTES: u64 = 2_500_000_000;
const SAMPLE_BIG_FILE_SEEKS: usize = 24;
const PASS_TIMEOUT: Duration = Duration::from_secs(90);
const FRAME_TIMEOUT: Duration = Duration::from_secs(25);

/// What one GET answered (no redirect followed).
#[derive(Debug, Clone, Default)]
pub struct Reply {
    pub status: u16,
    pub location: Option<String>,
    pub content_type: String,
}

/// One GET that doesn't follow redirects; a 2xx body is written to `dest`.
pub trait Fetch: Sync {
    fn get(
        &self,
        url: &str,
        headers: &[(&'static str, String)],
        dest: &Path,
    ) -> Result<Reply, String>;
}

/// curl, without `-L`, capped at MAX_BYTES and 20 s.
pub struct Curl;

impl Fetch for Curl {
    fn get(
        &self,
        url: &str,
        headers: &[(&'static str, String)],
        dest: &Path,
    ) -> Result<Reply, String> {
        let mut cmd = std::process::Command::new("curl");
        cmd.args([
            "-s",
            "-S",
            "--max-time",
            "20",
            "--max-filesize",
            &MAX_BYTES.to_string(),
        ])
        .args(["-A", slr_scout::http::USER_AGENT, "-D", "-", "-o"])
        .arg(dest);
        for (k, v) in headers {
            cmd.arg("-H").arg(format!("{k}: {v}"));
        }
        let out = cmd
            .arg(url)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("can't run curl: {e}"))?;
        if out.status.code() == Some(63) {
            return Err("Image too large".into());
        }
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        let mut reply = Reply::default();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some(rest) = line.strip_prefix("HTTP/") {
                reply.status = rest
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            } else if let Some((k, v)) = line.split_once(':') {
                match k.trim().to_ascii_lowercase().as_str() {
                    "location" => reply.location = Some(v.trim().into()),
                    "content-type" => reply.content_type = v.trim().to_ascii_lowercase(),
                    _ => {}
                }
            }
        }
        Ok(reply)
    }
}

/// `to` (a `Location`) against the URL that answered it.
fn join_url(base: &str, to: &str) -> String {
    if to.starts_with("https://") || to.starts_with("http://") {
        return to.into();
    }
    if let Some(rest) = to.strip_prefix("//") {
        return format!("https://{rest}");
    }
    let host_start = base.find("://").map_or(0, |i| i + 3);
    let host_end = base[host_start..]
        .find('/')
        .map_or(base.len(), |i| host_start + i);
    if to.starts_with('/') {
        return format!("{}{to}", &base[..host_end]);
    }
    let dir = base
        .rfind('/')
        .filter(|&i| i >= host_end)
        .unwrap_or(host_end);
    format!("{}/{to}", &base[..dir])
}

/// Download an image into `dest` (`downloadImage`): every redirect hop
/// `allowed` (each allow-list takes https only), at most 3 redirects, an image content type, at
/// most 5 MB. `headers` per URL (a cookie for the forum only).
pub fn fetch_image(
    fetch: &dyn Fetch,
    url: &str,
    allowed: &dyn Fn(&str) -> bool,
    headers: &dyn Fn(&str) -> Vec<(&'static str, String)>,
    dest: &Path,
) -> Result<(), String> {
    let mut current = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        if !allowed(&current) {
            return Err("Unsupported image link".into());
        }
        let mut h = headers(&current);
        h.push(("Accept", "image/*".into()));
        let r = fetch.get(&current, &h, dest)?;
        match r.status {
            300..=399 => {
                let to = r
                    .location
                    .ok_or_else(|| format!("Image download failed ({})", r.status))?;
                current = join_url(&current, &to);
            }
            200..=299 if !r.content_type.starts_with("image/") => return Err("Not an image".into()),
            200..=299 if std::fs::metadata(dest).map_or(true, |m| m.len() > MAX_BYTES) => {
                return Err("Image too large".into());
            }
            200..=299 => return Ok(()),
            s => return Err(format!("Image download failed ({s})")),
        }
    }
    Err("Too many redirects".into())
}

/// `src` as a JPEG at most 640 wide (`importThumbnail`).
pub(crate) fn to_jpeg(media: &Media, src: &Path, dest: &Path) -> bool {
    let scale = format!("scale='min({MAX_WIDTH},iw)':-2");
    let args: [&std::ffi::OsStr; 12] = [
        "-v".as_ref(),
        "error".as_ref(),
        "-i".as_ref(),
        src.as_os_str(),
        "-frames:v".as_ref(),
        "1".as_ref(),
        "-vf".as_ref(),
        scale.as_ref(),
        "-q:v".as_ref(),
        "3".as_ref(),
        "-y".as_ref(),
        dest.as_os_str(),
    ];
    media.ffmpeg_blocking(args, FRAME_TIMEOUT).is_some() && dest.is_file()
}
/// One poster to try.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub path: String,
    pub url: String,
    /// FapTap's scene id, or the post (`topicId`) of a rule34video cover.
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Faptap(Value),
    Rule34(Value),
}

/// FapTap's own hosts: faptap.net and its Bunny CDN zones, https only.
pub fn is_faptap_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    host == "faptap.net"
        || host
            .strip_prefix("vz-")
            .and_then(|h| h.strip_suffix(".b-cdn.net"))
            .is_some_and(|z| {
                !z.is_empty()
                    && z.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            })
}

fn auto(entry: &Value, source: &str) -> bool {
    entry["type"] == "image" && entry["source"] == source && entry["forced"] != true
}

/// The FapTap import may fill an empty slot or replace its own poster from
/// another link.
pub fn can_faptap(entry: &Value, url: &str) -> bool {
    entry.is_null() || (auto(entry, "faptap") && entry["imageUrl"] != url)
}

/// The rule34video import may also replace an automatic EroScripts preview
/// or FapTap thumbnail.
pub fn can_rule34(entry: &Value, url: &str) -> bool {
    entry.is_null()
        || (auto(entry, "rule34video") && entry["imageUrl"] != url)
        || auto(entry, "eroscripts")
        || auto(entry, "faptap")
}

/// What the import would try now, FapTap first.
pub fn plan(lib: &Library, videos: &[Video], now: u64) -> Vec<Job> {
    let l = lib.lib();
    let custom = |p: &str| &l["customThumbnails"][p];
    let mut jobs = Vec::new();
    for v in videos {
        let record = &l["scriptTags"][&v.path];
        let Some(url) = record["previewUrl"].as_str().filter(|u| !u.is_empty()) else {
            continue;
        };
        if record["source"] != "faptap-scout"
            || l[FAPTAP_ATTEMPTS][&v.path]["url"] == url
            || !can_faptap(custom(&v.path), url)
        {
            continue;
        }
        let kind = Kind::Faptap(record["faptapId"].clone());
        jobs.push(Job {
            path: v.path.clone(),
            url: url.into(),
            kind,
        });
    }
    for v in videos {
        let Some(e) = &v.eroscripts else { continue };
        let Some(url) = e.poster_url.as_deref().filter(|u| !u.is_empty()) else {
            continue;
        };
        let tried = &l[RULE34_ATTEMPTS][&v.path];
        let recent = tried["url"] == url
            && (tried["status"] != "failed"
                || tried["at"]
                    .as_u64()
                    .is_some_and(|at| now.saturating_sub(at) < RULE34_RETRY_MS));
        if !recent && can_rule34(custom(&v.path), url) {
            let kind = Kind::Rule34(e.topic_id.clone());
            jobs.push(Job {
                path: v.path.clone(),
                url: url.into(),
                kind,
            });
        }
    }
    jobs
}

// ---- perceptual hash (shared/preview-match.js) ----

/// Box-filter `rect` (x, y, w, h) of `gray` (w×h) to tw×th, with fractional
/// pixel coverage.
fn area_resize(
    gray: &[u8],
    w: usize,
    h: usize,
    tw: usize,
    th: usize,
    rect: (f64, f64, f64, f64),
) -> Vec<f64> {
    let (rx, ry, rw, rh) = rect;
    let (sx, sy) = (rw / tw as f64, rh / th as f64);
    let mut out = vec![0.0; tw * th];
    for ty in 0..th {
        let (y0, y1) = (ry + ty as f64 * sy, ry + (ty + 1) as f64 * sy);
        for tx in 0..tw {
            let (x0, x1) = (rx + tx as f64 * sx, rx + (tx + 1) as f64 * sx);
            let (mut sum, mut area) = (0.0, 0.0);
            for y in (y0.floor() as usize)..(y1.ceil() as usize).min(h) {
                let wy = (y as f64 + 1.0).min(y1) - (y as f64).max(y0);
                if wy <= 0.0 {
                    continue;
                }
                for x in (x0.floor() as usize)..(x1.ceil() as usize).min(w) {
                    let wx = (x as f64 + 1.0).min(x1) - (x as f64).max(x0);
                    if wx > 0.0 {
                        sum += f64::from(gray[y * w + x]) * wx * wy;
                        area += wx * wy;
                    }
                }
            }
            out[ty * tw + tx] = if area > 0.0 { sum / area } else { 0.0 };
        }
    }
    out
}

/// 64-bit pHash of a rectangle of a grey image (the 8×8 lowest DCT
/// frequencies of a 32×32 shrink against their median), and the shrink's
/// contrast (standard deviation).
pub fn phash(gray: &[u8], w: usize, h: usize, rect: (f64, f64, f64, f64)) -> (u64, f64) {
    let s = area_resize(gray, w, h, HASH, HASH, rect);
    let cos = |u: usize, x: usize| {
        (((2 * x + 1) * u) as f64 * std::f64::consts::PI / (2 * HASH) as f64).cos()
    };
    let mut rows = vec![0.0; HASH * 8];
    for y in 0..HASH {
        for u in 0..8 {
            rows[y * 8 + u] = (0..HASH).map(|x| s[y * HASH + x] * cos(u, x)).sum();
        }
    }
    let mut dct = [0.0; 64];
    for v in 0..8 {
        for u in 0..8 {
            dct[v * 8 + u] = (0..HASH).map(|y| rows[y * 8 + u] * cos(v, y)).sum();
        }
    }
    let mut ac = dct[1..].to_vec();
    ac.sort_by(f64::total_cmp);
    let median = ac[31];
    let hash = dct
        .iter()
        .fold(0u64, |h, &c| (h << 1) | u64::from(c > median));
    let mean = s.iter().sum::<f64>() / s.len() as f64;
    let contrast = (s.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / s.len() as f64).sqrt();
    (hash, contrast)
}

/// A video frame's hashes: the whole frame, plus its left eye when it looks
/// side by side; flat frames none.
pub fn frame_hashes(gray: &[u8], w: usize, h: usize) -> Vec<u64> {
    let mut rects = vec![(0.0, 0.0, w as f64, h as f64)];
    if w as f64 / h as f64 >= VR_MIN_ASPECT {
        rects.push((0.0, 0.0, w as f64 / 2.0, h as f64));
    }
    rects
        .into_iter()
        .map(|r| phash(gray, w, h, r))
        .filter(|(_, c)| *c >= MIN_CONTRAST)
        .map(|(h, _)| h)
        .collect()
}

/// The smallest distance between an image's hash and a frame's.
pub fn best_distance(image: u64, frames: &[u64]) -> Option<u32> {
    frames.iter().map(|f| (image ^ f).count_ones()).min()
}

/// An image too small or of the wrong shape to be a frame (`dimensionExclusion`).
pub(crate) fn misshapen(w: usize, h: usize) -> bool {
    let aspect = w as f64 / h as f64;
    w.max(h) < 320 || !(1.2..=3.2).contains(&aspect)
}

/// Up to 6 frames, evenly spread, of a picture (an animated GIF / WebP has
/// several; its first 60 are looked at) as grey pixels at the picture's own
/// size (ffmpeg → PGM).
fn decode_frames(media: &Media, file: &Path) -> Vec<(usize, usize, Vec<u8>)> {
    let args: [&std::ffi::OsStr; 13] = [
        "-v".as_ref(),
        "error".as_ref(),
        "-i".as_ref(),
        file.as_os_str(),
        "-frames:v".as_ref(),
        "60".as_ref(),
        "-f".as_ref(),
        "image2pipe".as_ref(),
        "-c:v".as_ref(),
        "pgm".as_ref(),
        "-pix_fmt".as_ref(),
        "gray".as_ref(),
        "-".as_ref(),
    ];
    let out = media
        .ffmpeg_blocking(args, FRAME_TIMEOUT)
        .unwrap_or_default();
    // Each frame: "P5 <w> <h> <max>", one whitespace byte, the pixels.
    let mut frames = Vec::new();
    let mut i = 0;
    loop {
        let mut fields = Vec::new();
        while fields.len() < 4 && i < out.len() {
            while i < out.len() && out[i].is_ascii_whitespace() {
                i += 1;
            }
            let start = i;
            while i < out.len() && !out[i].is_ascii_whitespace() {
                i += 1;
            }
            fields.push(String::from_utf8_lossy(&out[start..i]).into_owned());
        }
        let dims = (
            fields.get(1).and_then(|f| f.parse().ok()),
            fields.get(2).and_then(|f| f.parse().ok()),
        );
        let (Some("P5"), (Some(w), Some(h))) = (fields.first().map(String::as_str), dims) else {
            break;
        };
        let Some(px) = out.get(i + 1..i + 1 + w * h) else {
            break;
        };
        frames.push((w, h, px.to_vec()));
        i += 1 + w * h;
    }
    let n = frames.len().min(6);
    (0..n)
        .map(|k| frames[k * frames.len() / n].clone())
        .collect()
}

/// A picture's hashes (each looked-at frame of an animation), none when it
/// can't be read, is too small or of the wrong shape, or is flat.
pub fn image_hashes(media: &Media, file: &Path) -> Vec<u64> {
    decode_frames(media, file)
        .into_iter()
        .filter(|(w, h, _)| !misshapen(*w, *h))
        .map(|(w, h, px)| phash(&px, w, h, (0.0, 0.0, w as f64, h as f64)))
        .filter(|(_, c)| *c >= MIN_CONTRAST)
        .map(|(h, _)| h)
        .collect()
}

/// The closest pair between a picture's hashes and the video's frames.
pub fn closest(image: &[u64], frames: &[u64]) -> Option<u32> {
    image.iter().filter_map(|h| best_distance(*h, frames)).min()
}

/// Grey frames across the video at 128 wide: every keyframe of the middle
/// 90 % (thinned to 400), else evenly spaced seeks (`sample_gray_frames`).
pub(crate) fn sample_video(media: &Media, path: &str) -> (usize, usize, Vec<Vec<u8>>) {
    let Some(m) = media
        .cached(path)
        .filter(|m| m.duration > 0.0 && m.width > 0 && m.height > 0)
    else {
        return (0, 0, Vec::new());
    };
    let w = SAMPLE_WIDTH as usize;
    let h =
        ((w as f64 * f64::from(m.height) / f64::from(m.width) / 2.0).round() as usize * 2).max(2);
    let scale = format!("scale={w}:{h}:flags=area,format=gray");
    let raw = |args: Vec<String>, timeout| media.ffmpeg_blocking(args, timeout).unwrap_or_default();
    let seeks = |count: usize| -> Vec<Vec<u8>> {
        (0..count)
            .map(|i| SAMPLE_EDGE + (1.0 - 2.0 * SAMPLE_EDGE) * (i as f64 + 0.5) / count as f64)
            .map(|pct| {
                let at = format!("{:.3}", m.duration * pct);
                raw(
                    [
                        "-v",
                        "error",
                        "-skip_frame",
                        "nokey",
                        "-ss",
                        &at,
                        "-i",
                        path,
                        "-frames:v",
                        "1",
                        "-an",
                        "-sn",
                        "-vf",
                        &scale,
                        "-f",
                        "rawvideo",
                        "pipe:1",
                    ]
                    .map(String::from)
                    .into(),
                    FRAME_TIMEOUT,
                )
            })
            .filter(|f| f.len() == w * h)
            .collect()
    };
    let big = std::fs::metadata(path).is_ok_and(|f| f.len() > SAMPLE_PASS_MAX_BYTES);
    let mut frames = if big {
        Vec::new()
    } else {
        let (start, span) = (
            m.duration * SAMPLE_EDGE,
            m.duration * (1.0 - 2.0 * SAMPLE_EDGE),
        );
        let (start, span) = (format!("{start:.3}"), format!("{span:.3}"));
        let all = raw(
            [
                "-nostats",
                "-v",
                "error",
                "-skip_frame",
                "nokey",
                "-ss",
                &start,
                "-t",
                &span,
                "-i",
                path,
                "-an",
                "-sn",
                "-dn",
                "-fps_mode",
                "passthrough",
                "-vf",
                &scale,
                "-f",
                "rawvideo",
                "pipe:1",
            ]
            .map(String::from)
            .into(),
            PASS_TIMEOUT,
        );
        all.chunks_exact(w * h).map(<[u8]>::to_vec).collect()
    };
    if big {
        frames = seeks(SAMPLE_COUNT.max(SAMPLE_BIG_FILE_SEEKS));
    } else if frames.len() < SAMPLE_COUNT {
        frames = seeks(SAMPLE_COUNT);
    }
    if frames.len() > SAMPLE_MAX_FRAMES {
        let n = frames.len();
        frames = (0..SAMPLE_MAX_FRAMES)
            .map(|i| frames[i * n / SAMPLE_MAX_FRAMES].clone())
            .collect();
    }
    (w, h, frames)
}

/// How one try ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Imported,
    /// Settled without a poster (no match, gone, unusable): kept in the attempts.
    Settled(&'static str),
    /// Might work next time.
    Failed,
    /// The slot was taken meanwhile.
    Skipped,
}

/// Try every planned poster, one at a time, `delay` before each.
pub fn run(
    lib: &RwLock<Library>,
    videos: &[Video],
    media: &Media,
    dir: &Path,
    http: &dyn Fetch,
    delay: Duration,
    now: u64,
) -> Vec<(Job, Outcome)> {
    let jobs = plan(&lib.blocking_read(), videos, now);
    let mut out = Vec::new();
    for job in jobs {
        std::thread::sleep(delay);
        let outcome = import(lib, media, dir, http, &job, now);
        out.push((job, outcome));
    }
    out
}

fn import(
    lib: &RwLock<Library>,
    media: &Media,
    dir: &Path,
    http: &dyn Fetch,
    job: &Job,
    now: u64,
) -> Outcome {
    let faptap = matches!(job.kind, Kind::Faptap(_));
    let allowed = |l: &Library| {
        let entry = &l.lib()["customThumbnails"][&job.path];
        if faptap {
            can_faptap(entry, &job.url)
        } else {
            can_rule34(entry, &job.url)
        }
    };
    if !allowed(&lib.blocking_read()) {
        return Outcome::Skipped;
    }
    let (attempts, source) = if faptap {
        (FAPTAP_ATTEMPTS, "faptap")
    } else {
        (RULE34_ATTEMPTS, "rule34video")
    };
    let remember = |attempt: Value| {
        lib.blocking_write().lib_mut(attempts)[&job.path] = attempt;
    };
    let settle = |status: &'static str, extra: Value| {
        let mut a = json!({"url": job.url, "status": status});
        if let (Some(a), Some(x)) = (a.as_object_mut(), extra.as_object()) {
            a.extend(x.clone());
        }
        remember(a);
        Outcome::Settled(status)
    };
    let fail = || {
        if !faptap {
            remember(json!({"url": job.url, "status": "failed", "at": now}));
        }
        Outcome::Failed
    };
    let unusable = || {
        if faptap {
            fail()
        } else {
            settle("unusable", json!({}))
        }
    };
    let stem = format!("{:x}-{source}", md5::compute(&job.path));
    let part = dir.join(format!("{stem}.part"));
    let _ = std::fs::create_dir_all(dir);
    let allowed_url: &dyn Fn(&str) -> bool = if faptap {
        &is_faptap_url
    } else {
        &is_rule34video_poster_url
    };
    let headers = |_: &str| {
        let referer = (!faptap).then(|| ("Referer", "https://rule34video.com/".to_string()));
        referer.into_iter().collect()
    };
    let got = fetch_image(http, &job.url, allowed_url, &headers, &part);
    let finish = |o: Outcome| {
        let _ = std::fs::remove_file(&part);
        o
    };
    if let Err(e) = got {
        // Answers that won't change on the next try (`SETTLED_ERROR`).
        let gone = e.contains("(404)") || e.contains("(410)");
        let settled = gone
            || [
                "Not an image",
                "Image too large",
                "Unsupported image link",
                "Too many redirects",
            ]
            .contains(&e.as_str());
        return finish(match (faptap, gone, settled) {
            (true, true, _) => settle("gone", json!({})),
            (false, _, true) => settle("unusable", json!({})),
            _ => fail(),
        });
    }
    let mut distance = None;
    if faptap {
        let (w, h, frames) = sample_video(media, &job.path);
        let video: Vec<u64> = frames.iter().flat_map(|f| frame_hashes(f, w, h)).collect();
        distance = closest(&image_hashes(media, &part), &video);
        if distance.is_none_or(|d| d > MATCH_THRESHOLD) {
            return finish(settle("no-match", json!({"distance": distance})));
        }
    }
    let dest = dir.join(format!("{stem}.jpg"));
    if !to_jpeg(media, &part, &dest) {
        return finish(unusable());
    }
    let mut l = lib.blocking_write();
    if !allowed(&l) {
        return finish(Outcome::Skipped);
    }
    let before = l.lib()["customThumbnails"][&job.path].clone();
    let entry = match &job.kind {
        Kind::Faptap(id) => {
            json!({"type": "image", "imagePath": dest, "source": "faptap", "faptapId": id,
                                   "imageUrl": job.url, "matchScore": distance})
        }
        Kind::Rule34(topic) => json!({"type": "image", "imagePath": dest, "source": "rule34video",
                                      "topicId": topic, "imageUrl": job.url}),
    };
    l.lib_mut("customThumbnails")[&job.path] = entry;
    l.lib_mut(attempts)[&job.path] = json!({"url": job.url, "status": "imported"});
    drop(l);
    // The automatic poster replaced has its own file: drop it when it's ours.
    if let Some(old) = before["imagePath"].as_str().map(Path::new)
        && old.starts_with(dir)
        && old != dest
    {
        let _ = std::fs::remove_file(old);
    }
    finish(Outcome::Imported)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn hosts_and_slots() {
        assert!(is_faptap_url(
            "https://faptap.net/api/assets/thumbnails/1.webp"
        ));
        assert!(is_faptap_url("https://vz-ab12-3.b-cdn.net/x/thumbnail.jpg"));
        assert!(!is_faptap_url("http://faptap.net/a.jpg"));
        assert!(!is_faptap_url("https://vz-.b-cdn.net/a.jpg"));
        assert!(!is_faptap_url("https://evil.net/faptap.net"));
        let user = json!({"type": "image", "source": "user"});
        let faptap = json!({"type": "image", "source": "faptap", "imageUrl": "u1"});
        let preview = json!({"type": "image", "source": "eroscripts"});
        assert!(can_faptap(&Value::Null, "u") && can_faptap(&faptap, "u2"));
        assert!(
            !can_faptap(&faptap, "u1") && !can_faptap(&user, "u") && !can_faptap(&preview, "u")
        );
        assert!(can_rule34(&preview, "u") && can_rule34(&faptap, "u") && !can_rule34(&user, "u"));
        let forced = json!({"type": "image", "source": "faptap", "forced": true});
        assert!(!can_rule34(&forced, "u"));
    }

    #[test]
    fn phash_matches_the_same_picture_not_another() {
        let pic = |w: usize, h: usize, f: &dyn Fn(f64, f64) -> f64| -> Vec<u8> {
            (0..w * h)
                .map(|i| {
                    f((i % w) as f64 / w as f64, (i / w) as f64 / h as f64).clamp(0.0, 255.0) as u8
                })
                .collect()
        };
        let a = |x: f64, y: f64| 128.0 + 100.0 * (x * 9.0).sin() * (y * 5.0).cos();
        let b = |x: f64, y: f64| 128.0 + 100.0 * (x * 3.0 + y * 11.0).cos();
        let full = |px: &[u8], w, h| phash(px, w, h, (0.0, 0.0, w as f64, h as f64)).0;
        // The same picture at another size is (nearly) the same hash.
        let same =
            (full(&pic(640, 360, &a), 640, 360) ^ full(&pic(128, 72, &a), 128, 72)).count_ones();
        assert!(same <= MATCH_THRESHOLD, "{same}");
        let other =
            (full(&pic(640, 360, &a), 640, 360) ^ full(&pic(640, 360, &b), 640, 360)).count_ones();
        assert!(other > MATCH_THRESHOLD, "{other}");
        assert!(
            frame_hashes(&[90; 128 * 72], 128, 72).is_empty(),
            "flat frame"
        );
        // A side-by-side frame is also compared by its left eye.
        let sbs = pic(256, 64, &|x, y| a((x * 2.0) % 1.0, y));
        let eye = full(&pic(128, 64, &a), 128, 64);
        assert!(best_distance(eye, &frame_hashes(&sbs, 256, 64)).unwrap() <= MATCH_THRESHOLD);
        assert!(misshapen(300, 200) && misshapen(1000, 200) && !misshapen(640, 360));
    }

    fn video(path: &str, poster: Option<&str>) -> Video {
        let mut v: Video = serde_json::from_value(json!({"id": "x", "name": "n", "path": path, "ext": ".mp4",
            "funscriptPath": null, "fuzzyMatched": false, "subtitlePath": null, "variants": [], "axes": [],
            "sourceName": "s", "dateAdded": 0.0, "size": 0}))
        .unwrap();
        v.eroscripts = poster.map(|p| crate::scan::EroscriptsRef {
            topic_id: json!(7),
            url: None,
            tags: vec![],
            poster_url: Some(p.into()),
        });
        v
    }

    const COVER: &str = "https://rule34video.com/contents/videos_screenshots/3000/3456/preview.jpg";

    #[test]
    fn plan_respects_attempts_and_slots() {
        let lib = Library {
            config: json!({"settings": {"library": {
                "scriptTags": {
                    "/f1": {"source": "faptap-scout", "previewUrl": "https://faptap.net/1.webp", "faptapId": "9"},
                    "/f2": {"source": "faptap-scout", "previewUrl": "https://faptap.net/2.webp"},
                    "/f3": {"source": "eroscripts-scout", "previewUrl": "https://faptap.net/3.webp"}
                },
                "faptapThumbnails": {"/f2": {"url": "https://faptap.net/2.webp", "status": "no-match"}},
                "customThumbnails": {"/r2": {"type": "image", "source": "user"}},
                "rule34videoThumbnails": {
                    "/r3": {"url": COVER, "status": "failed", "at": 1000},
                    "/r4": {"url": COVER, "status": "failed", "at": 1000 + RULE34_RETRY_MS}
                }
            }}}),
            ..Default::default()
        };
        let vs: Vec<Video> = ["/f1", "/f2", "/f3"]
            .iter()
            .map(|p| video(p, None))
            .chain(
                ["/r1", "/r2", "/r3", "/r4"]
                    .iter()
                    .map(|p| video(p, Some(COVER))),
            )
            .collect();
        let paths: Vec<String> = plan(&lib, &vs, 2000 + RULE34_RETRY_MS)
            .into_iter()
            .map(|j| j.path)
            .collect();
        assert_eq!(
            paths,
            ["/f1", "/r1", "/r3"],
            "tried links, other slots and recent failures wait"
        );
    }

    /// Serves the images of a map; `redirect:<url>` answers 302 there; 404 otherwise.
    pub(crate) struct Fake(pub(crate) std::collections::HashMap<String, Vec<u8>>);
    impl Fetch for Fake {
        fn get(
            &self,
            url: &str,
            _: &[(&'static str, String)],
            dest: &Path,
        ) -> Result<Reply, String> {
            let Some(body) = self.0.get(url) else {
                return Ok(Reply {
                    status: 404,
                    ..Default::default()
                });
            };
            if let Some(to) = std::str::from_utf8(body)
                .ok()
                .and_then(|b| b.strip_prefix("redirect:"))
            {
                return Ok(Reply {
                    status: 302,
                    location: Some(to.into()),
                    ..Default::default()
                });
            }
            std::fs::write(dest, body).map_err(|e| e.to_string())?;
            Ok(Reply {
                status: 200,
                content_type: "image/jpeg".into(),
                ..Default::default()
            })
        }
    }

    #[test]
    fn every_redirect_hop_is_checked() {
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("x");
        let http = Fake(
            [
                (
                    "https://faptap.net/a.webp".to_string(),
                    b"redirect:/cdn/a.webp".to_vec(),
                ),
                ("https://faptap.net/cdn/a.webp".to_string(), b"img".to_vec()),
                (
                    "https://faptap.net/b.webp".to_string(),
                    b"redirect:https://evil.test/b.webp".to_vec(),
                ),
                (
                    "https://faptap.net/c.webp".to_string(),
                    b"redirect:/c.webp".to_vec(),
                ),
            ]
            .into(),
        );
        let get = |u: &str| fetch_image(&http, u, &is_faptap_url, &|_| Vec::new(), &dest);
        assert_eq!(get("https://faptap.net/a.webp"), Ok(()));
        assert_eq!(std::fs::read(&dest).unwrap(), b"img");
        assert_eq!(
            get("https://faptap.net/b.webp"),
            Err("Unsupported image link".into())
        );
        assert_eq!(
            get("https://faptap.net/c.webp"),
            Err("Too many redirects".into())
        );
        assert_eq!(
            get("https://faptap.net/gone.webp"),
            Err("Image download failed (404)".into())
        );
        assert_eq!(
            get("http://faptap.net/a.webp"),
            Err("Unsupported image link".into())
        );
        assert_eq!(
            join_url("https://h.test/a/b.jpg", "c.jpg"),
            "https://h.test/a/c.jpg"
        );
        assert_eq!(
            join_url("https://h.test/a/b.jpg", "//cdn.test/x"),
            "https://cdn.test/x"
        );
    }

    #[test]
    fn rule34video_covers_are_imported_or_settled() {
        let d = tempfile::tempdir().unwrap();
        let media = Media::new(d.path().join("thumbs"));
        let dir = d.path().join("custom-thumbs");
        let gone = COVER.replace("3456", "3457");
        let lib = RwLock::new(Library {
            config: json!({"settings": {"library": {"customThumbnails": {
                "/b": {"type": "image", "source": "faptap", "imagePath": dir.join("old-faptap.jpg")}}}}}),
            ..Default::default()
        });
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("old-faptap.jpg"), b"old").unwrap();
        // A real JPEG from ffmpeg, when ffmpeg is installed.
        let src = d.path().join("src.jpg");
        let made = media.ffmpeg_blocking(
            [
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=800x450",
                "-frames:v",
                "1",
                "-y",
            ]
            .iter()
            .map(std::ffi::OsStr::new)
            .chain([src.as_os_str()]),
            Duration::from_secs(20),
        );
        if made.is_none() {
            eprintln!("ffmpeg missing: skipped");
            return;
        }
        let http = Fake(
            [
                (COVER.to_string(), std::fs::read(&src).unwrap()),
                (gone.replace("3457", "3458"), b"not an image".to_vec()),
            ]
            .into(),
        );
        let vs = vec![
            video("/a", Some(COVER)),
            video("/b", Some(COVER)),
            video("/c", Some(&gone)),
            video("/d", Some(&gone.replace("3457", "3458"))),
        ];
        let out: Vec<Outcome> = run(&lib, &vs, &media, &dir, &http, Duration::ZERO, 5)
            .into_iter()
            .map(|x| x.1)
            .collect();
        assert_eq!(
            out,
            [
                Outcome::Imported,
                Outcome::Imported,
                Outcome::Settled("unusable"),
                Outcome::Settled("unusable")
            ]
        );
        let l = lib.blocking_read();
        let entry = &l.lib()["customThumbnails"]["/a"];
        assert_eq!(
            (entry["source"].clone(), entry["topicId"].clone()),
            (json!("rule34video"), json!(7))
        );
        assert!(Path::new(entry["imagePath"].as_str().unwrap()).is_file());
        assert!(
            !dir.join("old-faptap.jpg").exists(),
            "the FapTap poster it replaced is gone"
        );
        assert_eq!(l.lib()[RULE34_ATTEMPTS]["/c"]["status"], "unusable");
        drop(l);
        assert!(
            plan(&lib.blocking_read(), &vs, 5).is_empty(),
            "nothing is tried twice"
        );
    }

    #[test]
    fn faptap_thumbnails_must_show_the_video() {
        let d = tempfile::tempdir().unwrap();
        let media = Media::new(d.path().join("thumbs"));
        let ff = |args: &[&str], out: &Path| {
            let args = args
                .iter()
                .map(std::ffi::OsStr::new)
                .chain(["-y".as_ref(), out.as_os_str()]);
            media
                .ffmpeg_blocking(args, Duration::from_secs(30))
                .is_some()
        };
        let (clip, frame, other) = (
            d.path().join("v.mp4"),
            d.path().join("f.jpg"),
            d.path().join("o.jpg"),
        );
        let lavfi = ["-v", "error", "-f", "lavfi", "-i"];
        if !ff(
            &[
                &lavfi[..],
                &[
                    "testsrc2=size=640x360:rate=25",
                    "-t",
                    "3",
                    "-g",
                    "5",
                    "-pix_fmt",
                    "yuv420p",
                ],
            ]
            .concat(),
            &clip,
        ) {
            eprintln!("ffmpeg missing: skipped");
            return;
        }
        assert!(ff(
            &[
                "-v",
                "error",
                "-ss",
                "1.5",
                "-i",
                clip.to_str().unwrap(),
                "-frames:v",
                "1"
            ],
            &frame
        ));
        assert!(ff(
            &[&lavfi[..], &["mandelbrot=size=640x360", "-frames:v", "1"]].concat(),
            &other
        ));
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(media.metadata(&clip))
            .unwrap();
        let path = clip.to_string_lossy().to_string();
        let (u1, u2) = ("https://faptap.net/1.webp", "https://faptap.net/2.webp");
        let record =
            |url: &str| json!({"source": "faptap-scout", "previewUrl": url, "faptapId": "9"});
        let lib = RwLock::new(Library {
            config: json!({"settings": {"library": {"scriptTags": {&path: record(u1)}}}}),
            ..Default::default()
        });
        let http = Fake(
            [
                (u1.to_string(), std::fs::read(&other).unwrap()),
                (u2.to_string(), std::fs::read(&frame).unwrap()),
            ]
            .into(),
        );
        let dir = d.path().join("custom-thumbs");
        let vs = vec![video(&path, None)];
        let out = run(&lib, &vs, &media, &dir, &http, Duration::ZERO, 1);
        assert_eq!(out[0].1, Outcome::Settled("no-match"));
        assert!(lib.blocking_read().lib()["customThumbnails"][&path].is_null());
        lib.blocking_write().lib_mut("scriptTags")[&path] = record(u2);
        let out = run(&lib, &vs, &media, &dir, &http, Duration::ZERO, 2);
        let l = lib.blocking_read();
        assert_eq!(out[0].1, Outcome::Imported, "{}", l.lib()[FAPTAP_ATTEMPTS]);
        let entry = &l.lib()["customThumbnails"][&path];
        assert_eq!(
            (entry["source"].clone(), entry["faptapId"].clone()),
            (json!("faptap"), json!("9"))
        );
        assert!(entry["matchScore"].as_u64().unwrap() <= u64::from(MATCH_THRESHOLD));
        assert_eq!(l.lib()[FAPTAP_ATTEMPTS][&path]["status"], "imported");
    }
}
