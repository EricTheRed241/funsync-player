//! The Downloads page (`/downloads`, `GET /api/downloads`): everything swiped right and where
//! it stands, from the same files the deck reads, rebuilt on every request.
//!
//! - **Waiting**: swiped right and not in the library yet; the next download run takes it
//!   (an EroScripts post still on the waiting list, a scene not fetched, a creator's post).
//! - **SLR**: the same for whatever downloads from SexLikeReal (SLR scenes, posts and creator
//!   posts whose video is on SLR): those wait on the account's quota, which comes along
//!   (`slr-scout.json`'s `quota` / `quotaError`), so they're kept apart from Waiting.
//! - **Downloaded**: swiped right and in the library, within the last RECENT_DAYS, newest
//!   first (at most MAX_RECENT). A post's time is its newest library video's modification
//!   time (crate::library::topic_downloaded), a scene's its scout's `fetchedAt`.
//! - **Won't download**: swiped right, but the scout turned it down when it fetched it
//!   (`eroscripts-scout-rejected.json`'s reasons), or it left the waiting list or the scout's
//!   file.
//!
//! Plus the download job (`eroscripts-scout` in deploy/jobs.json: every scout's `--fetch`)
//! as the job runner last saw it: running now (and its log's last line) or its last run.

use crate::creators::Creators;
use crate::frames;
use crate::library;
use crate::report::{self, id_of, str_of, strings_of};
use crate::scenes::{self, Source};
use crate::swipes::{self, Dir, Swipes};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, SystemTime};

/// "Downloaded" lists what came in this many days back.
pub const RECENT_DAYS: u64 = 30;
pub const MAX_RECENT: usize = 100;
/// The job that downloads the right swipes (deploy/jobs.json).
pub const DOWNLOAD_JOB: &str = "eroscripts-scout";
pub const REJECTED_FILE: &str = "eroscripts-scout-rejected.json";
/// Longest log line passed on.
const MAX_LOG_LINE: usize = 300;

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Downloads {
    pub waiting: Vec<Item>,
    /// Waiting items that download from SexLikeReal (`Item::slr`).
    pub slr: Vec<Item>,
    pub recent: Vec<Item>,
    pub wont: Vec<Item>,
    /// slr:scout's last quota read (`{checkedAt, videos, scripts, aiScripts}`), and why the
    /// last read failed.
    pub quota: Value,
    pub quota_error: Option<String>,
    pub job: Job,
    /// Files that exist but couldn't be read, library sources that can't be.
    pub problems: Vec<String>,
}

/// One right swipe.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    /// `eroscripts`, a scene source (`slr`, `vrb`, `pmvhaven`, `faptap`) or `creators`.
    pub source: &'static str,
    /// The badge: the source, `vrbangers` / `vrconk` for vrb, the post's kind for creators.
    pub site: String,
    pub id: String,
    pub title: Option<String>,
    pub author: Option<String>,
    /// An https image or a local frame / picture path (relative, as the deck's).
    pub image: Option<String>,
    /// The post or scene page.
    pub url: Option<String>,
    /// The video: its host's page, or the file itself (PMVHaven).
    pub video_url: Option<String>,
    pub vr: bool,
    /// Its video downloads from SexLikeReal (costs the account's quota).
    pub slr: bool,
    /// Size of the video(s) when known, in GB (10^9 bytes).
    pub gb: Option<f64>,
    pub swiped_at: Option<String>,
    pub downloaded_at: Option<String>,
    /// Why it waits or won't come.
    pub note: Option<String>,
}

/// The download job as the job runner last saw it.
#[derive(Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub name: &'static str,
    pub running: bool,
    /// The last (or current) run: unix seconds, and `done`, `work left`, `failed`, …
    pub started: Option<i64>,
    pub ended: Option<i64>,
    pub result: Option<String>,
    pub exit: Option<i32>,
    /// The run's log's last line while it runs.
    pub last_line: Option<String>,
}

/// Read every file and sort the right swipes.
pub fn load(user_data: &Path) -> Downloads {
    let mut problems = Vec::new();
    let config = report::read_json(user_data, "config.json", &mut problems);
    let links = report::read_json(user_data, library::LINKS_FILE, &mut problems);
    let in_library = library::topic_downloaded(&config, &links, &mut problems);
    let cutoff =
        video_tags::timefmt::iso_utc(SystemTime::now() - Duration::from_secs(RECENT_DAYS * 86400));
    let mut d = Downloads::default();
    let mut posts_seen = HashSet::new();

    // EroScripts posts.
    let state = report::read_json(user_data, report::STATE_FILE, &mut problems);
    let waiting = report::read_json(user_data, report::WAITING_FILE, &mut problems);
    let rejected = report::read_json(user_data, REJECTED_FILE, &mut problems);
    let made = report::read_json(user_data, frames::EROSCRIPTS_FILE, &mut problems);
    let candidates: HashMap<u64, &Value> = waiting
        .get("candidates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| Some((id_of(c.get("id")?)?, c)))
        .collect();
    let reasons = rejected_reasons(&rejected);
    let post_swipes = report::read_swipes(user_data, swipes::EROSCRIPTS, &mut problems);
    for (id, at) in rights(&post_swipes) {
        let Ok(topic) = id.parse::<u64>() else {
            continue;
        };
        posts_seen.insert(topic);
        let rec = state
            .pointer(&format!("/topics/{topic}"))
            .unwrap_or(&Value::Null);
        let cand = candidates.get(&topic).copied();
        let mut item = post_item(user_data, topic, rec, cand, &made);
        item.swiped_at = Some(at);
        let downloaded = in_library
            .get(&topic)
            .map(|t| video_tags::timefmt::iso_utc(*t));
        place(&mut d, item, downloaded, cand.is_some(), || {
            reasons
                .get(&topic)
                .cloned()
                .unwrap_or_else(|| "not on the waiting list any more".into())
        });
    }

    // Each scene scout's scenes.
    for src in Source::ALL {
        let file = report::read_json(user_data, src.file(), &mut problems);
        if src == Source::Slr {
            d.quota = file.get("quota").cloned().unwrap_or(Value::Null);
            d.quota_error = str_of(&file, "quotaError");
        }
        let by_id: HashMap<String, &Value> = scenes::scenes(src, &file).collect();
        let swipes = report::read_swipes(user_data, src.swipes(), &mut problems);
        for (id, at) in rights(&swipes) {
            let Some(scene) = by_id.get(&id).copied() else {
                let item = Item {
                    source: src.name(),
                    site: src.name().into(),
                    id,
                    swiped_at: Some(at),
                    note: Some(format!("not in {} any more", src.file())),
                    ..Item::default()
                };
                d.wont.push(item);
                continue;
            };
            if str_of(scene, "fetchedAt").is_none() && scenes::is_done(scene) {
                continue; // In the library already, not downloaded for this swipe.
            }
            let mut item = scene_item(src, id, scene);
            item.swiped_at = Some(at);
            place(&mut d, item, str_of(scene, "fetchedAt"), true, String::new);
        }
    }

    // Creators' posts swiped want on the deck's creator cards.
    let creators = Creators::read(user_data, &mut problems);
    for w in creators.wanted() {
        let (kind, id) = w.key.split_once(':').unwrap_or(("", ""));
        let topic = (kind == "eroscripts")
            .then(|| id.parse::<u64>().ok())
            .flatten();
        if topic.is_some_and(|t| posts_seen.contains(&t)) {
            continue; // Swiped on its own post card too.
        }
        let post = w.post.unwrap_or(&Value::Null);
        let item = Item {
            source: crate::creators::SOURCE,
            site: kind.to_string(),
            id: w.key.clone(),
            title: str_of(post, "title"),
            author: w.who.as_ref().map(|who| who.name.clone()),
            image: w.who.as_ref().and_then(|who| {
                str_of(post, "image").and_then(|f| crate::creator_pictures::url(&who.slug, &f))
            }),
            url: https(str_of(post, "url")),
            video_url: crate::creators::first_video_page(post),
            vr: strings_of(post, "tags")
                .unwrap_or_default()
                .iter()
                .any(|t| t.eq_ignore_ascii_case("vr")),
            slr: crate::creators::on_slr(post),
            swiped_at: w.at.clone(),
            ..Item::default()
        };
        let downloaded = topic
            .and_then(|t| in_library.get(&t))
            .map(|t| video_tags::timefmt::iso_utc(*t));
        if downloaded.is_none() && flag(post, "inLibrary") {
            continue; // In the library under another name: nothing to wait for.
        }
        let downloadable = w.post.is_some() && flag(post, "downloadable");
        place(&mut d, item, downloaded, downloadable, || {
            if w.post.is_none() {
                "not in creators.json any more".into()
            } else {
                "no downloadable video".into()
            }
        });
    }

    d.waiting.sort_by(|a, b| b.swiped_at.cmp(&a.swiped_at));
    d.slr.sort_by(|a, b| b.swiped_at.cmp(&a.swiped_at));
    d.wont.sort_by(|a, b| b.swiped_at.cmp(&a.swiped_at));
    d.recent.retain(|i| {
        i.downloaded_at
            .as_deref()
            .is_some_and(|t| t >= cutoff.as_str())
    });
    d.recent
        .sort_by(|a, b| b.downloaded_at.cmp(&a.downloaded_at));
    d.recent.truncate(MAX_RECENT);
    d.job = job(user_data);
    d.problems = problems;
    d
}

/// File `item`: downloaded (at that time) → recent; else still wanted → slr (on SLR) or
/// waiting; else → won't, with `why`.
fn place(
    d: &mut Downloads,
    mut item: Item,
    downloaded: Option<String>,
    wanted: bool,
    why: impl FnOnce() -> String,
) {
    if let Some(at) = downloaded {
        item.downloaded_at = Some(at);
        d.recent.push(item);
    } else if wanted && item.slr {
        d.slr.push(item);
    } else if wanted {
        d.waiting.push(item);
    } else {
        item.note = Some(why());
        d.wont.push(item);
    }
}

/// The right swipes of a file: (id, when).
fn rights(s: &Swipes) -> impl Iterator<Item = (String, String)> + '_ {
    s.swipes
        .iter()
        .filter(|(_, sw)| sw.dir == Dir::Right)
        .map(|(id, sw)| (id.clone(), sw.at.clone()))
}

/// Per topic, why its videos were turned down or removed (eroscripts-scout-rejected.json,
/// electron/rejected-videos.js's reasons), each reason once, joined.
fn rejected_reasons(rejected: &Value) -> HashMap<u64, String> {
    let mut by_topic: HashMap<u64, Vec<String>> = HashMap::new();
    for e in rejected
        .get("entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(topic), Some(reason)) = (e.get("topicId").and_then(id_of), str_of(e, "reason"))
        else {
            continue;
        };
        let text = reason_text(&reason);
        let list = by_topic.entry(topic).or_default();
        if !list.contains(&text) {
            list.push(text);
        }
    }
    by_topic
        .into_iter()
        .map(|(t, r)| (t, r.join("; ")))
        .collect()
}

fn reason_text(reason: &str) -> String {
    match reason {
        "short" => "too short".into(),
        "duplicate" => "a copy of a video already in the library".into(),
        "low-rating" => "downloaded, then deleted by its low rating".into(),
        "no-thank-you" => "downloaded, then deleted (No thank you)".into(),
        other => format!("turned down: {other}"),
    }
}

fn post_item(
    user_data: &Path,
    topic: u64,
    rec: &Value,
    cand: Option<&Value>,
    made: &Value,
) -> Item {
    let cand_str = |k: &str| cand.and_then(|c| str_of(c, k));
    let scenes: &[Value] = cand
        .and_then(|c| c.get("scenes"))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let image = https(str_of(rec, "image"))
        .or_else(|| {
            strings_of(rec, "images")
                .unwrap_or_default()
                .into_iter()
                .find(|u| u.starts_with("https://"))
        })
        .or_else(|| {
            frames::urls(
                user_data,
                frames::Source::Eroscripts,
                topic,
                frames::post_record(made, topic),
            )
            .into_iter()
            .next()
        });
    Item {
        source: crate::deck::EROSCRIPTS,
        site: crate::deck::EROSCRIPTS.into(),
        id: topic.to_string(),
        title: str_of(rec, "title").or_else(|| cand_str("title")),
        author: str_of(rec, "author").or_else(|| cand_str("author")),
        image,
        url: Some(format!("{}{topic}", report::FORUM_TOPIC)),
        video_url: scenes.iter().find_map(|s| {
            s.pointer("/video/url")
                .and_then(Value::as_str)
                .and_then(crate::deck::watch_url)
        }),
        vr: cand
            .and_then(|c| c.get("vr"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        slr: scenes
            .iter()
            .any(|s| s.pointer("/video/host").and_then(Value::as_str) == Some("slr")),
        gb: report::total_gb(scenes),
        ..Item::default()
    }
}

fn scene_item(src: Source, id: String, s: &Value) -> Item {
    let site = str_of(s, "site").unwrap_or_else(|| src.name().to_string());
    let url = https(str_of(s, "url"));
    Item {
        source: src.name(),
        note: (site == "vrconk").then(|| "needs a VRConk membership".to_string()),
        site,
        id,
        title: str_of(s, "title"),
        author: str_of(s, "studio").or_else(|| str_of(s, "author")),
        image: https(str_of(s, "thumbnail").or_else(|| str_of(s, "thumbnailUrl"))),
        video_url: https(str_of(s, "videoUrl")).or_else(|| url.clone()),
        url,
        vr: flag(s, "vr"),
        slr: src == Source::Slr,
        gb: s
            .get("fileSize")
            .and_then(Value::as_f64)
            .map(|b| (b / 1e7).round() / 100.0),
        ..Item::default()
    }
}

/// The download job's state (job_runner::state) and whether it runs now (its lock's pid).
fn job(user_data: &Path) -> Job {
    let last = job_runner::state::read(user_data, DOWNLOAD_JOB).last;
    let running = job_runner::exec::holder(user_data, DOWNLOAD_JOB).is_some();
    let last_line = running
        .then(|| last.as_ref().and_then(|r| last_log_line(Path::new(&r.log))))
        .flatten();
    Job {
        name: DOWNLOAD_JOB,
        running,
        started: last.as_ref().map(|r| r.started),
        ended: last.as_ref().and_then(|r| r.ended),
        result: last.as_ref().map(|r| r.result.clone()),
        exit: last.as_ref().and_then(|r| r.exit),
        last_line,
    }
}

/// The last non-empty line of a log (its last 8 KB), at most MAX_LOG_LINE characters.
fn last_log_line(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(8192))).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    // Progress bars rewrite their line with `\r`: the last state is what's on screen.
    let line = text
        .lines()
        .rev()
        .map(|l| l.rsplit('\r').next().unwrap_or(l).trim())
        .find(|l| !l.is_empty())?;
    Some(line.chars().take(MAX_LOG_LINE).collect())
}

fn https(u: Option<String>) -> Option<String> {
    u.filter(|u| u.starts_with("https://"))
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user_data(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "scout-report-downloads-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, v: Value) {
        std::fs::write(dir.join(name), v.to_string()).unwrap();
    }

    fn ids(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .map(|i| format!("{}:{}", i.source, i.id))
            .collect()
    }

    #[test]
    fn sorts_right_swipes_into_waiting_downloaded_and_wont() {
        let dir = user_data("sort");
        let lib = dir.join("Library");
        let scene = lib.join("Ann/Scene");
        std::fs::create_dir_all(&scene).unwrap();
        std::fs::write(scene.join("v.mp4"), b"x").unwrap();
        std::fs::write(
            scene.join("eroscripts.json"),
            json!({"topicId": 10, "file": "v.mp4"}).to_string(),
        )
        .unwrap();
        write(
            &dir,
            "config.json",
            json!({"settings": {"library": {"sources": [{"path": lib.to_str().unwrap()}]}}}),
        );
        write(
            &dir,
            report::STATE_FILE,
            json!({"topics": {
            "10": {"title": "In the library"}, "11": {"title": "Waiting", "image": "https://x/11.jpg"},
            "12": {"title": "Short"}, "13": {"title": "Gone"}, "14": {"title": "Hidden"}}}),
        );
        write(
            &dir,
            report::WAITING_FILE,
            json!({"candidates": [
            {"id": 11, "vr": true, "scenes": [{"video": {"url": "https://rule34video.com/video/1/", "size": 2e9}}]},
            {"id": 15, "scenes": [{"video": {"host": "slr", "url": "https://www.sexlikereal.com/scenes/x-15"}}]}]}),
        );
        write(
            &dir,
            REJECTED_FILE,
            json!({"entries": [
            {"topicId": 12, "reason": "short"}, {"topicId": 12, "reason": "short"}, {"topicId": 12, "reason": "duplicate"}]}),
        );
        write(
            &dir,
            swipes::EROSCRIPTS.file,
            json!({"version": 1, "swipes": {
            "10": {"dir": "right", "at": "2026-11-20T00:00:00.000Z"},
            "11": {"dir": "right", "at": "2026-11-21T00:00:00.000Z"},
            "12": {"dir": "right", "at": "2026-11-19T00:00:00.000Z"},
            "13": {"dir": "right", "at": "2026-11-18T00:00:00.000Z"},
            "14": {"dir": "left", "at": "2026-11-22T00:00:00.000Z"},
            "15": {"dir": "right", "at": "2026-11-17T00:00:00.000Z"}}}),
        );
        let recent = video_tags::timefmt::now_iso();
        write(
            &dir,
            Source::Pmvhaven.file(),
            json!({"scenes": [
            {"id": "p1", "title": "Fetched", "fetchedAt": recent},
            {"id": "p2", "title": "Old", "fetchedAt": "2020-01-01T00:00:00.000Z"},
            {"id": "p3", "title": "Next", "videoUrl": "https://cdn/p3.mp4", "url": "https://pmvhaven.com/video/p3", "fileSize": 1.5e9},
            {"id": "p4", "title": "Had it", "inLibrary": true}]}),
        );
        write(
            &dir,
            swipes::PMVHAVEN.file,
            json!({"version": 1, "swipes": {
            "p1": {"dir": "right", "at": "2026-11-20T00:00:00.000Z"},
            "p2": {"dir": "right", "at": "2026-11-20T00:00:00.000Z"},
            "p3": {"dir": "right", "at": "2026-11-22T00:00:00.000Z"},
            "p4": {"dir": "right", "at": "2026-11-22T00:00:00.000Z"},
            "p9": {"dir": "right", "at": "2026-11-17T00:00:00.000Z"}}}),
        );
        write(
            &dir,
            Source::Vrb.file(),
            json!({"scenes": [{"id": "c1", "site": "vrconk", "url": "https://vrconk.com/c1"}]}),
        );
        write(
            &dir,
            swipes::VRB.file,
            json!({"version": 1, "swipes": {"c1": {"dir": "right", "at": "2026-11-15T00:00:00.000Z"}}}),
        );

        // What downloads from SexLikeReal waits apart, with the quota: an SLR scene and a post
        // whose video is on SLR.
        write(
            &dir,
            Source::Slr.file(),
            json!({"quota": {"videos": {"remaining": 26, "total": 35}}, "quotaError": "x",
                   "scenes": [{"id": 900, "title": "SLR scene"}]}),
        );
        write(
            &dir,
            swipes::SLR.file,
            json!({"version": 1, "swipes": {"900": {"dir": "right", "at": "2026-11-16T00:00:00.000Z"}}}),
        );
        let d = load(&dir);
        assert_eq!(ids(&d.waiting), ["pmvhaven:p3", "eroscripts:11", "vrb:c1"]);
        assert_eq!(ids(&d.slr), ["eroscripts:15", "slr:900"]);
        assert_eq!(
            (&d.quota["videos"]["remaining"], d.quota_error.as_deref()),
            (&json!(26), Some("x"))
        );
        assert_eq!(ids(&d.recent), ["pmvhaven:p1", "eroscripts:10"]);
        assert_eq!(
            ids(&d.wont),
            ["eroscripts:12", "eroscripts:13", "pmvhaven:p9"]
        );
        let post = &d.waiting[1];
        assert_eq!(
            (
                post.title.as_deref(),
                post.video_url.as_deref(),
                post.gb,
                post.vr
            ),
            (
                Some("Waiting"),
                Some("https://rule34video.com/video/1/"),
                Some(2.0),
                true
            )
        );
        assert_eq!(
            post.url.as_deref(),
            Some("https://discuss.eroscripts.com/t/11")
        );
        // A PMVHaven scene's video is the file itself.
        assert_eq!(
            d.waiting[0].video_url.as_deref(),
            Some("https://cdn/p3.mp4")
        );
        assert_eq!(d.waiting[0].gb, Some(1.5));
        assert_eq!(
            d.waiting[2].note.as_deref(),
            Some("needs a VRConk membership")
        );
        assert_eq!(
            d.wont[0].note.as_deref(),
            Some("too short; a copy of a video already in the library")
        );
        assert_eq!(
            d.wont[1].note.as_deref(),
            Some("not on the waiting list any more")
        );
        assert!(d.recent[1].downloaded_at.as_deref().unwrap().ends_with('Z'));
        assert_eq!((d.job.running, d.job.result.as_deref()), (false, None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn creator_posts_wait_until_their_post_is_in_the_library() {
        let dir = user_data("creators");
        write(
            &dir,
            crate::creators::FILE,
            json!({"creators": {"ann": {"name": "Ann", "posts": [
            {"kind": "eroscripts", "id": 20, "title": "Hers", "downloadable": true, "url": "https://discuss.eroscripts.com/t/20",
             "videoPages": ["https://pixeldrain.com/u/abc"]},
            {"kind": "faptap", "id": "f1", "title": "Not downloadable", "downloadable": false},
            {"kind": "eroscripts", "id": 21, "title": "Also a post card", "downloadable": true}]}}}),
        );
        write(
            &dir,
            crate::creators::SWIPES_FILE,
            json!({"version": 1, "creators": {}, "posts": {
            "eroscripts:20": {"choice": "want", "at": "2026-11-20T00:00:00.000Z", "creator": "ann"},
            "faptap:f1": {"choice": "want", "at": "2026-11-19T00:00:00.000Z", "creator": "ann"},
            "eroscripts:21": {"choice": "want", "at": "2026-11-18T00:00:00.000Z", "creator": "ann"},
            "eroscripts:22": {"choice": "hide", "at": "2026-11-18T00:00:00.000Z", "creator": "ann"}}}),
        );
        write(
            &dir,
            swipes::EROSCRIPTS.file,
            json!({"version": 1, "swipes": {
            "21": {"dir": "right", "at": "2026-11-17T00:00:00.000Z"}}}),
        );
        let d = load(&dir);
        assert_eq!(ids(&d.waiting), ["creators:eroscripts:20"]);
        assert_eq!(d.waiting[0].author.as_deref(), Some("Ann"));
        assert_eq!(
            d.waiting[0].video_url.as_deref(),
            Some("https://pixeldrain.com/u/abc")
        );
        // 21 counts once, as the post; f1 can't be downloaded.
        assert_eq!(ids(&d.wont), ["creators:faptap:f1", "eroscripts:21"]);
        assert_eq!(d.wont[0].note.as_deref(), Some("no downloadable video"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_logs_last_line_is_what_a_progress_bar_shows_last() {
        let dir = user_data("log");
        let log = dir.join("run.log");
        std::fs::write(
            &log,
            "step 1\nfetching a.mp4  10%\r fetching a.mp4  55%\r\n\n",
        )
        .unwrap();
        assert_eq!(last_log_line(&log).as_deref(), Some("fetching a.mp4  55%"));
        assert_eq!(last_log_line(&dir.join("none.log")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
