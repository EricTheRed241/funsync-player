//! EroScripts post previews as thumbnails (README "Post previews as
//! thumbnails"; renderer/js/eroscripts-preview.js, electron/preview-match.js
//! `collectPreviewCandidates`, eroscripts-api.js `downloadPreviewImage`):
//!
//! - every image of the post's first post (and the topic thumbnail) that
//!   can be a picture of the video is a candidate: not avatars, emoji,
//!   quotes, mentions, link previews, images linking off-site, site images,
//!   under 320 px or outside 1.2–3.2 aspect; at most 8;
//! - each is downloaded from the forum or its CDN only (the session cookie
//!   to the forum alone, every redirect hop checked) and compared with
//!   frames sampled across the video by pHash (`posters`); the closest
//!   within 12 of 64 bits becomes the poster (`source: eroscripts`,
//!   `topicId`, `imageUrl`, `matchScore`), as a JPEG ≤ 640 wide in this
//!   server's `custom-thumbs/`;
//! - only a video without any custom thumbnail gets one; the library-wide
//!   batch first re-checks the posters FunSync set by itself (EroScripts
//!   previews, FapTap thumbnails; not uploads, pins or picks by hand) and
//!   removes those that don't show their video.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use regex::Regex;
use serde_json::{Value, json};
use slr_scout::forum::decode_entities;
use tokio::sync::RwLock;

use crate::data::Library;
use crate::media::Media;
use crate::posters::{
    Fetch, MATCH_THRESHOLD, closest, fetch_image, frame_hashes, image_hashes, misshapen,
    sample_video, to_jpeg,
};
use crate::scan::Video;

const CDN_HOST: &str = "eroscripts-discourse.eroscripts.com";
const MAX_CANDIDATES: usize = 8;
const FOREIGN_CLASSES: [&str; 5] = ["quote", "onebox", "mention", "user-card", "badge"];
const CHROME_CLASSES: [&str; 5] = ["emoji", "avatar", "site-icon", "onebox-avatar", "badge"];

/// The lower-case host of an https URL without user or port, else none.
pub fn host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let h = rest.split(['/', '?', '#']).next()?.to_ascii_lowercase();
    (!h.is_empty() && !h.contains(['@', ':'])).then_some(h)
}

/// The forum's host and its upload CDN: where post images may come from.
fn image_hosts(forum: &str) -> [String; 2] {
    [host(forum).unwrap_or_default(), CDN_HOST.into()]
}

fn absolute(forum: &str, url: &str) -> String {
    if let Some(rest) = url.strip_prefix("//") {
        format!("https://{rest}")
    } else if url.starts_with("http") {
        url.into()
    } else {
        format!(
            "{}{}{url}",
            forum.trim_end_matches('/'),
            if url.starts_with('/') { "" } else { "/" }
        )
    }
}

/// Why a URL can't be a picture of the video (`urlExclusion`).
fn url_exclusion(url: &str, forum: &str) -> Option<&'static str> {
    let rest = url.split_once("://").map_or(url, |x| x.1);
    let path = rest
        .find('/')
        .map_or("", |i| &rest[i..])
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .to_lowercase();
    if path.contains("/images/emoji/") {
        return Some("emoji");
    }
    if path.contains("avatar") {
        return Some("avatar");
    }
    if !host(url).is_some_and(|h| image_hosts(forum).contains(&h)) {
        return Some("not a forum upload");
    }
    if !["/uploads/", "/original/", "/optimized/"]
        .iter()
        .any(|p| path.starts_with(p))
    {
        return Some("site image");
    }
    let ext = path.rsplit_once('.').map_or("", |x| x.1);
    (!["jpg", "jpeg", "png", "gif", "webp", "avif"].contains(&ext)).then_some("not an image")
}

/// The same upload under another size shares its SHA-1 (`uploadKey`).
fn upload_key(url: &str) -> String {
    static SHA: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)/([0-9a-f]{40})(?:_[^/]*)?\.[a-z0-9]+(?:\?|$)").unwrap());
    SHA.captures(url)
        .map_or(url.into(), |c| c[1].to_lowercase())
}

fn attr(attrs: &str, name: &str) -> Option<String> {
    let re = Regex::new(&format!(
        r#"(?i)(?:^|\s){name}\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#
    ))
    .ok()?;
    let c = re.captures(attrs)?;
    Some(decode_entities(
        c.get(1).or(c.get(2)).or(c.get(3))?.as_str(),
    ))
}

fn classes(attrs: &str) -> Vec<String> {
    attr(attrs, "class")
        .unwrap_or_default()
        .to_lowercase()
        .split_whitespace()
        .map(String::from)
        .collect()
}

/// Every image of a topic (`/t/<id>.json`) that could be a frame of the
/// video, best first: `known` URLs, the topic thumbnail, the first post's
/// images in order (`collectPreviewCandidates`).
pub fn candidates(data: &Value, known: &[String], forum: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut consider = |raw: &str, w: u64, h: u64, context: Option<&str>| {
        let url = absolute(forum, raw);
        if !seen.insert(upload_key(&url)) {
            return;
        }
        let misfit = w > 0 && h > 0 && misshapen(w as usize, h as usize);
        if context.is_none() && url_exclusion(&url, forum).is_none() && !misfit {
            out.push(url);
        }
    };
    for u in known.iter().filter(|u| !u.is_empty()) {
        consider(u, 0, 0, None);
    }
    let width = |t: &Value| t["width"].as_u64().unwrap_or(0);
    let thumbs: Vec<&Value> = data["thumbnails"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["url"].is_string())
        .collect();
    if let Some(original) = thumbs
        .iter()
        .copied()
        .reduce(|a, b| if width(b) > width(a) { b } else { a })
    {
        let sized = thumbs
            .iter()
            .copied()
            .filter(|t| width(t) >= 640)
            .min_by_key(|t| width(t))
            .unwrap_or(original);
        consider(
            sized["url"].as_str().unwrap(),
            width(original),
            original["height"].as_u64().unwrap_or(0),
            None,
        );
    } else if let Some(u) = data["image_url"].as_str() {
        consider(u, 0, 0, None);
    }
    static TAG: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)<(/?)([a-z][a-z0-9]*)\b([^>]*)>").unwrap());
    let html = data["post_stream"]["posts"][0]["cooked"]
        .as_str()
        .unwrap_or("");
    // Open <aside>/<a>/<div>/<span>: (tag, classes, href).
    let mut stack: Vec<(String, Vec<String>, Option<String>)> = Vec::new();
    let hosts = image_hosts(forum);
    for m in TAG.captures_iter(html) {
        let (closing, tag, attrs) = (&m[1] == "/", m[2].to_lowercase(), &m[3]);
        if ["aside", "a", "div", "span"].contains(&tag.as_str()) {
            if closing {
                if let Some(at) = stack.iter().rposition(|e| e.0 == tag) {
                    stack.truncate(at);
                }
            } else if !attrs.trim().ends_with('/') {
                let href = (tag == "a").then(|| attr(attrs, "href")).flatten();
                stack.push((tag, classes(attrs), href));
            }
            continue;
        }
        if tag != "img" || closing {
            continue;
        }
        let Some(src) = attr(attrs, "src") else {
            continue;
        };
        let own = classes(attrs);
        let chrome = CHROME_CLASSES.iter().any(|c| own.iter().any(|o| o == c));
        let foreign = stack.iter().any(|(tag, cls, href)| {
            cls.iter().any(|c| FOREIGN_CLASSES.contains(&c.as_str()))
                || (tag == "a"
                    && href
                        .as_deref()
                        .and_then(|h| host(&absolute(forum, h)))
                        .is_some_and(|h| !hosts.contains(&h)))
        });
        let dim = |n| attr(attrs, n).and_then(|v| v.parse().ok()).unwrap_or(0);
        consider(
            &src,
            dim("width"),
            dim("height"),
            (chrome || foreign).then_some("excluded"),
        );
    }
    out.truncate(MAX_CANDIDATES);
    out
}

/// The video's frame hashes (sampled once per video).
fn video_hashes(media: &Media, path: &str) -> Vec<u64> {
    let (w, h, frames) = sample_video(media, path);
    frames.iter().flat_map(|f| frame_hashes(f, w, h)).collect()
}

/// The distance between a poster file and its video's frames (none when
/// either can't be read).
pub fn distance(media: &Media, video_path: &str, image: &Path) -> Option<u32> {
    closest(
        &image_hashes(media, image),
        &video_hashes(media, video_path),
    )
}

/// Where the forum's images may be fetched from, and with what headers.
pub struct Forum<'a> {
    pub base: String,
    pub cookie: Option<String>,
    pub fetch: &'a dyn Fetch,
    /// `/t/<id>.json` of a topic.
    pub topic: &'a dyn Fn(u64) -> Option<Value>,
}

impl Forum<'_> {
    /// An image from the forum (with the session cookie) or its CDN only,
    /// every redirect hop included.
    pub fn download(&self, url: &str, dest: &Path) -> Result<(), String> {
        let forum = format!("{}/", self.base.trim_end_matches('/'));
        let allowed = |u: &str| u.starts_with(&forum) || host(u).is_some_and(|h| h == CDN_HOST);
        let headers = |u: &str| match &self.cookie {
            Some(c) if u.starts_with(&forum) => vec![("Cookie", format!("_t={c}"))],
            _ => Vec::new(),
        };
        fetch_image(self.fetch, url, &allowed, &headers, dest)
    }
}

/// How one video's preview went.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Imported(u32),
    Skipped,
    NoMatch(Option<u32>),
    NoImage,
    Failed,
}

/// Import the post image that shows the video as its thumbnail, when the
/// video has no custom thumbnail (`importPostPreview`).
pub fn import(
    lib: &RwLock<Library>,
    media: &Media,
    dir: &Path,
    forum: &Forum,
    video_path: &str,
    topic_id: u64,
    known: &[String],
) -> Outcome {
    let free = |l: &Library| l.lib()["customThumbnails"][video_path].is_null();
    if !free(&lib.blocking_read()) {
        return Outcome::Skipped;
    }
    let Some(data) = (forum.topic)(topic_id) else {
        return Outcome::Failed;
    };
    let urls = candidates(&data, known, &forum.base);
    if urls.is_empty() {
        return Outcome::NoImage;
    }
    let video = video_hashes(media, video_path);
    if video.is_empty() {
        return Outcome::Failed;
    }
    let _ = std::fs::create_dir_all(dir);
    let (mut best, mut nearest): (Option<(u32, String, std::path::PathBuf)>, Option<u32>) =
        (None, None);
    for (i, url) in urls.iter().enumerate() {
        let part = dir.join(format!("{:x}-{i}.part", md5::compute(video_path)));
        let distance = forum
            .download(url, &part)
            .ok()
            .map(|_| image_hashes(media, &part))
            .and_then(|h| closest(&h, &video));
        nearest = match (nearest, distance) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        match distance {
            Some(d) if d <= MATCH_THRESHOLD && best.as_ref().is_none_or(|b| d < b.0) => {
                if let Some((_, _, old)) = best.replace((d, url.clone(), part)) {
                    let _ = std::fs::remove_file(old);
                }
            }
            _ => {
                let _ = std::fs::remove_file(&part);
            }
        }
    }
    let Some((distance, url, part)) = best else {
        return Outcome::NoMatch(nearest);
    };
    let dest = dir.join(format!("{:x}-eroscripts.jpg", md5::compute(video_path)));
    let converted = to_jpeg(media, &part, &dest);
    let _ = std::fs::remove_file(&part);
    if !converted {
        return Outcome::Failed;
    }
    let mut l = lib.blocking_write();
    if !free(&l) {
        return Outcome::Skipped;
    }
    l.lib_mut("customThumbnails")[video_path] = json!({"type": "image", "imagePath": dest,
        "source": "eroscripts", "topicId": topic_id, "imageUrl": url, "matchScore": distance});
    Outcome::Imported(distance)
}

/// A poster FunSync set by itself (`isRevalidatable`).
fn revalidatable(entry: &Value) -> bool {
    entry["type"] == "image"
        && (entry["source"] == "eroscripts" || entry["source"] == "faptap")
        && entry["forced"] != true
}

/// Batch progress for the EroScripts page.
pub static STATUS: LazyLock<Mutex<Value>> = LazyLock::new(|| Mutex::new(json!({"running": false})));

fn progress(phase: &str, done: usize, total: usize) {
    let mut s = STATUS.lock().unwrap();
    s["phase"] = json!(phase);
    s["done"] = json!(done);
    s["total"] = json!(total);
}

/// The library-wide "Fetch EroScripts previews" (`fetchPreviewsBatch`):
/// re-check every poster FunSync set by itself against its video (removing
/// those that don't show it; their file only when it is ours), then give
/// every video with a known post and no custom thumbnail a matching post
/// image, `delay` between posts. Returns the summary.
pub fn batch(
    lib: &RwLock<Library>,
    videos: &[Video],
    media: &Media,
    dir: &Path,
    forum: &Forum,
    delay: Duration,
) -> Value {
    let todo: Vec<(String, Value)> = {
        let l = lib.blocking_read();
        let custom = &l.lib()["customThumbnails"];
        videos
            .iter()
            .filter(|v| revalidatable(&custom[&v.path]))
            .map(|v| (v.path.clone(), custom[&v.path].clone()))
            .collect()
    };
    let (mut kept, mut removed, mut unchecked) = (0, 0, 0);
    for (i, (path, entry)) in todo.iter().enumerate() {
        progress("revalidate", i, todo.len());
        let image = entry["imagePath"].as_str().map(Path::new);
        let Some(distance) = image.and_then(|i| distance(media, path, i)) else {
            unchecked += 1;
            continue;
        };
        let mut l = lib.blocking_write();
        let custom = l.lib_mut("customThumbnails");
        if custom[path] != *entry {
            unchecked += 1;
            continue;
        }
        if distance <= MATCH_THRESHOLD {
            kept += 1;
            custom[path]["matchScore"] = json!(distance);
            continue;
        }
        removed += 1;
        custom.as_object_mut().map(|m| m.remove(path));
        if let Some(file) = entry["imagePath"]
            .as_str()
            .map(Path::new)
            .filter(|f| f.starts_with(dir))
        {
            let _ = std::fs::remove_file(file);
        }
    }
    let pending: Vec<(String, u64, Vec<String>)> = {
        let l = lib.blocking_read();
        let (tags, custom) = (&l.lib()["scriptTags"], &l.lib()["customThumbnails"]);
        videos
            .iter()
            .filter_map(|v| {
                let id = tags[&v.path]["topicId"].as_u64()?;
                let known = tags[&v.path]["previewUrl"]
                    .as_str()
                    .map(String::from)
                    .into_iter()
                    .collect();
                custom[&v.path]
                    .is_null()
                    .then(|| (v.path.clone(), id, known))
            })
            .collect()
    };
    let known_posts = videos
        .iter()
        .filter(|v| lib.blocking_read().lib()["scriptTags"][&v.path]["topicId"].is_u64())
        .count();
    // Topics are fetched once per batch: bundle posts cover many videos.
    let cache: Mutex<HashMap<u64, Option<Value>>> = Mutex::default();
    let cached = |id: u64| {
        if let Some(t) = cache.lock().unwrap().get(&id) {
            return t.clone();
        }
        if !cache.lock().unwrap().is_empty() {
            std::thread::sleep(delay);
        }
        let t = (forum.topic)(id);
        cache.lock().unwrap().insert(id, t.clone());
        t
    };
    let once = Forum {
        base: forum.base.clone(),
        cookie: forum.cookie.clone(),
        fetch: forum.fetch,
        topic: &cached,
    };
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for (i, (path, id, known)) in pending.iter().enumerate() {
        progress("fetch", i, pending.len());
        let key = match import(lib, media, dir, &once, path, *id, known) {
            Outcome::Imported(_) => "imported",
            Outcome::Skipped => "skipped",
            Outcome::NoMatch(_) => "noMatch",
            Outcome::NoImage => "noImage",
            Outcome::Failed => "failed",
        };
        *counts.entry(key).or_default() += 1;
    }
    let n = |k| counts.get(k).copied().unwrap_or(0);
    json!({"known": known_posts, "total": pending.len(), "imported": n("imported"), "skipped": n("skipped"),
           "noMatch": n("noMatch"), "noImage": n("noImage"), "failed": n("failed"),
           "revalidated": {"checked": todo.len(), "kept": kept, "removed": removed, "unchecked": unchecked}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posters::tests::Fake;

    const F: &str = "https://forum.test";

    #[test]
    fn only_pictures_of_the_video_are_candidates() {
        let up = |n: &str| format!("{F}/uploads/default/original/1X/{n}.jpg");
        let sha = "a".repeat(40);
        let data = json!({
            "thumbnails": [
                {"url": format!("https://{CDN_HOST}/optimized/1X/{sha}_2_1024x576.jpeg"), "width": 1024, "height": 576},
                {"url": format!("https://{CDN_HOST}/original/1X/{sha}.jpeg"), "width": 1920, "height": 1080}
            ],
            "post_stream": {"posts": [{"cooked": format!(r#"
                <p><img src="{a}" width="1280" height="720"></p>
                <img src="{F}/images/emoji/twitter/fire.png" class="emoji">
                <a href="https://www.patreon.com/me"><img src="{b}" width="1200" height="600"></a>
                <aside class="quote"><img src="{c}" width="1200" height="600"></aside>
                <img src="{d}" width="1200" height="200">
                <img src="{e}" width="300" height="200">
                <a href="{F}/uploads/x.jpg"><img src="{f}" width="1200" height="675"></a>
                <img src="https://imgur.com/x.jpg" width="1200" height="675">
                <img src="{F}/user_avatar/me.png" width="1200" height="675">
                <img src="//{CDN_HOST}/original/1X/{sha}.jpeg">"#,
                a = up("a"), b = up("b"), c = up("c"), d = up("d"), e = up("e"), f = up("f"))}]}
        });
        let got = candidates(&data, &[format!("{F}/uploads/known.webp")], F);
        assert_eq!(
            got,
            [
                format!("{F}/uploads/known.webp"),
                format!("https://{CDN_HOST}/optimized/1X/{sha}_2_1024x576.jpeg"),
                up("a"),
                up("f"),
            ],
            "known first, the topic thumbnail at poster size once, banners, quotes, odd sizes and other sites left out"
        );
    }

    #[test]
    fn previews_must_show_the_video_and_revalidation_removes_others() {
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
        let (banner, shot) = (
            format!("{F}/uploads/banner.jpg"),
            format!("https://{CDN_HOST}/original/1X/shot.jpg"),
        );
        let fetch = Fake(
            [
                (banner.clone(), std::fs::read(&other).unwrap()),
                (
                    format!("{F}/uploads/short.jpg"),
                    format!("redirect:{shot}").into_bytes(),
                ),
                (shot.clone(), std::fs::read(&frame).unwrap()),
            ]
            .into(),
        );
        let post = |imgs: &[&str]| {
            let cooked: String = imgs
                .iter()
                .map(|u| format!(r#"<img src="{u}" width="1280" height="720">"#))
                .collect();
            json!({"post_stream": {"posts": [{"cooked": cooked}]}})
        };
        let topics: HashMap<u64, Value> = [
            (1, post(&[&banner])),
            (2, post(&[&banner, "/uploads/short.jpg"])),
        ]
        .into();
        let topic = |id| topics.get(&id).cloned();
        let forum = Forum {
            base: F.into(),
            cookie: Some("c".into()),
            fetch: &fetch,
            topic: &topic,
        };
        let dir = d.path().join("custom-thumbs");
        let lib = RwLock::new(Library::default());
        assert!(matches!(
            import(&lib, &media, &dir, &forum, &path, 1, &[]),
            Outcome::NoMatch(Some(_))
        ));
        let Outcome::Imported(dist) = import(&lib, &media, &dir, &forum, &path, 2, &[]) else {
            panic!()
        };
        assert!(dist <= MATCH_THRESHOLD);
        let entry = lib.blocking_read().lib()["customThumbnails"][&path].clone();
        assert_eq!(
            (
                entry["source"].clone(),
                entry["topicId"].clone(),
                entry["imageUrl"].clone()
            ),
            (
                json!("eroscripts"),
                json!(2),
                json!(format!("{F}/uploads/short.jpg"))
            )
        );
        assert_eq!(
            import(&lib, &media, &dir, &forum, &path, 2, &[]),
            Outcome::Skipped,
            "a set thumbnail stays"
        );

        // Revalidation: a poster that doesn't show the video goes (its file
        // too, when ours); one picked by hand stays.
        let wrong = dir.join("wrong.jpg");
        std::fs::copy(&other, &wrong).unwrap();
        lib.blocking_write().lib_mut("customThumbnails")[&path] =
            json!({"type": "image", "imagePath": wrong, "source": "eroscripts"});
        lib.blocking_write().lib_mut("scriptTags")[&path] = json!({"topicId": 2});
        let video: Video = serde_json::from_value(json!({"id": "x", "name": "v", "path": path, "ext": ".mp4",
            "funscriptPath": null, "fuzzyMatched": false, "subtitlePath": null, "variants": [], "axes": [],
            "sourceName": "s", "dateAdded": 0.0, "size": 0}))
        .unwrap();
        let summary = batch(
            &lib,
            std::slice::from_ref(&video),
            &media,
            &dir,
            &forum,
            Duration::ZERO,
        );
        assert_eq!(summary["revalidated"]["removed"], 1);
        assert_eq!(summary["imported"], 1, "then the matching image comes back");
        assert!(!wrong.exists());
        lib.blocking_write().lib_mut("customThumbnails")[&path]["forced"] = json!(true);
        let summary = batch(&lib, &[video], &media, &dir, &forum, Duration::ZERO);
        assert_eq!(summary["revalidated"]["checked"], 0);
    }
}
