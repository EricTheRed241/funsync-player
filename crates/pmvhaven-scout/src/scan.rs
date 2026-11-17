//! A scan: every list page, each kept video's details (cached), the
//! screening, the script choice and the score.

use crate::api::{self, Detail, Video};
use crate::classify;
use crate::library::Known;
use crate::prefs::PmvhavenScout;
use crate::script::{self, Script};
use crate::store::{self, Cache, CacheEntry, Scene};
use slr_scout::http::{self, Http, NetError, Pacer, Request};
use slr_scout::prefs::Prefs;
use slr_scout::score::{self, Candidate, Scorer};
use slr_scout::screen::{self, Item};
use slr_scout::text;
use std::collections::{BTreeMap, HashSet};
use std::io::Write;

/// Stills after the thumbnail in a scene card.
pub const IMAGES: usize = 4;
/// Details read between saves of the detail cache.
pub const CHECKPOINT: usize = 100;

pub struct Scan<'a> {
    pub http: &'a dyn Http,
    /// pmvhaven.com: list pages and details.
    pub api: Pacer,
    pub prefs: &'a Prefs,
    pub own: &'a PmvhavenScout,
    pub scorer: &'a Scorer,
    pub known: &'a Known,
    pub now: String,
    pub max_pages: Option<u32>,
    /// Save the detail cache here every [`CHECKPOINT`] details, so a first
    /// scan (~20 min) that is stopped keeps what it read.
    pub user_data: Option<&'a std::path::Path>,
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub scenes: Vec<Scene>,
    /// Videos dropped, by reason.
    pub dropped: BTreeMap<String, usize>,
    /// Videos listed (each once).
    pub listed: usize,
    /// PMVHaven's count of scripted videos.
    pub total: u64,
    pub pages: u32,
    pub details_read: usize,
    /// Videos left out this time: no details yet and PMVHaven rate limited us.
    pub left: usize,
    /// The list stopped early (rate limited); scenes are what was read.
    pub partial: bool,
    pub rate_limited: bool,
}

impl Outcome {
    /// Count a video dropped for `reason`.
    fn drop(&mut self, reason: &str) {
        *self.dropped.entry(reason.to_string()).or_default() += 1;
    }
}

/// The video's tags for scoring and screening: slugged in both spellings
/// (`score::scene_tags`), plus `pmv` / `compilation` when the words say so.
pub fn tags(v: &Video, pmv: bool, compilation: bool) -> Vec<String> {
    let mut tags = score::scene_tags(&v.tags, false);
    for (applies, tag) in [(pmv, "pmv"), (compilation, "compilation")] {
        if applies && !tags.iter().any(|t| t == tag) {
            tags.push(tag.to_string());
        }
    }
    tags
}

/// Uploader, original creator and performers: who the screening and the
/// favourites look at.
fn creators(v: &Video) -> Vec<String> {
    let mut out = vec![v.uploader.clone()];
    out.extend(v.original_creator.clone());
    out.extend(v.stars.iter().cloned());
    out.retain(|c| !c.trim().is_empty());
    out
}

/// The screening's verdict; `description` empty before the details are read.
pub fn screened(prefs: &Prefs, v: &Video, tags: &[String], description: &str) -> Option<String> {
    let creators = creators(v);
    screen::screen(
        &prefs.screening,
        &Item {
            title: &v.title,
            tags,
            text: description,
            creators: &creators,
            duration_ms: v.duration_s.map(|s| s * 1000),
        },
    )
    .map(|r| r.key().to_string())
}

/// The scene for a kept video.
pub fn scene(
    v: &Video,
    d: &Detail,
    script: Script,
    own: &PmvhavenScout,
    scorer: &Scorer,
    known: &Known,
) -> Scene {
    let pmv = classify::is_pmv(v, own);
    let compilation = classify::is_compilation(v, own);
    let tags = tags(v, pmv, compilation);
    let vr_suffix = classify::vr_suffix(v, own).unwrap_or_default();
    let studio = v
        .original_creator
        .clone()
        .or_else(|| Some(v.uploader.clone()).filter(|u| !u.is_empty()));
    let favourite = scorer.favourite_in(creators(v).iter().map(String::as_str));
    let artist_in_library = studio
        .iter()
        .chain([&v.uploader])
        .any(|n| scorer.in_library(n));
    let c = Candidate {
        tags: &tags,
        title: &v.title,
        vr: !vr_suffix.is_empty(),
        artist_in_library,
        favourite: favourite.is_some(),
        multi_axis: !script.axes.is_empty(),
        likes: v.likes as f64,
    };
    let parts = scorer.shared_parts(&c, own.popularity_per);
    let creampie = text::contains_words(&classify::words_text(v), "creampie");
    Scene {
        id: v.id.clone(),
        site: "pmvhaven".into(),
        url: api::page_url(&v.id, &v.title),
        title: v.title.clone(),
        studio,
        actors: v.stars.clone(),
        date: v.date.clone(),
        duration_s: v.duration_s,
        thumbnail: v.thumbnail.clone(),
        images: api::spread(&d.stills, IMAGES),
        vr: !vr_suffix.is_empty(),
        vr_suffix: vr_suffix.to_string(),
        fps: None,
        passthrough: false,
        creampie,
        pmv,
        compilation,
        likes: v.likes,
        views: v.views,
        tag_scores: scorer.tag_scores(&tags),
        tags,
        favourite,
        script: Some(script),
        handmade: Vec::new(),
        in_library: known.has(&v.id, &v.title, v.duration_s),
        fetched_at: None,
        video_url: v.video_url.clone().unwrap_or_default(),
        file_size: d.file_size,
        uploader: v.uploader.clone(),
        score: score::round2(parts.iter().map(|(_, x)| x).sum()),
        score_parts: parts
            .into_iter()
            .map(|(k, x)| (k, score::round2(x)))
            .collect(),
    }
}

/// Score desc, then likes desc, then id.
pub fn sort(scenes: &mut [Scene]) {
    scenes.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(b.likes.cmp(&a.likes))
            .then(a.id.cmp(&b.id))
    });
}

impl Scan<'_> {
    /// The list, page by page (each video once).
    fn list(&mut self, out: &mut Outcome, err: &mut dyn Write) -> Vec<Video> {
        let mut videos: Vec<Video> = Vec::new();
        let mut seen = HashSet::new();
        let mut page = 1;
        loop {
            let url = api::list_url(page);
            match http::paced_json(self.http, &mut self.api, &Request::get(&url))
                .map_err(|e| (e.clone(), e.to_string()))
                .and_then(|v| api::page(&v).map_err(|m| (NetError::Failed(m.clone()), m)))
            {
                Ok(p) => {
                    out.pages = page;
                    out.total = p.total;
                    for v in p.videos {
                        if seen.insert(v.id.clone()) {
                            videos.push(v);
                        }
                    }
                    let _ = writeln!(
                        err,
                        "  page {page}/{}: {} video(s) so far",
                        p.total_pages,
                        videos.len()
                    );
                    if page >= p.total_pages || self.max_pages.is_some_and(|m| page >= m) {
                        break;
                    }
                    page += 1;
                }
                Err((e, msg)) => {
                    let _ = writeln!(err, "Warning: list page {page}: {msg}");
                    out.partial = true;
                    out.rate_limited |= matches!(e, NetError::RateLimited(_));
                    break;
                }
            }
        }
        videos
    }

    /// Walk the list and build the scenes; `cache` is updated with what
    /// was read.
    pub fn run(&mut self, cache: &mut Cache, err: &mut dyn Write) -> Outcome {
        let mut out = Outcome::default();
        let _ = writeln!(err, "Reading PMVHaven's scripted videos...");
        let videos = self.list(&mut out, err);
        out.listed = videos.len();
        let mut stopped = out.rate_limited;
        let todo = videos
            .iter()
            .filter(|v| {
                cache
                    .videos
                    .get(&v.id)
                    .is_none_or(|e| !store::fresh(e, &self.now, self.own.detail_max_age_days))
            })
            .count();
        if todo > 0 {
            let _ = writeln!(
                err,
                "Details: up to {todo} to read (1 s apart), the rest from {}...",
                store::CACHE_NAME
            );
        }
        for v in &videos {
            if let Some(d) = classify::drop_reason(v, self.own) {
                out.drop(d.key());
                continue;
            }
            let pmv = classify::is_pmv(v, self.own);
            let compilation = classify::is_compilation(v, self.own);
            let tags = tags(v, pmv, compilation);
            if let Some(key) = screened(self.prefs, v, &tags, "") {
                out.drop(&key);
                continue;
            }
            let cached = cache.videos.get(&v.id);
            let fresh =
                cached.is_some_and(|e| store::fresh(e, &self.now, self.own.detail_max_age_days));
            let detail = if fresh || stopped {
                cached.map(|e| e.detail.clone())
            } else {
                let url = api::detail_url(&v.id);
                match http::paced_json(self.http, &mut self.api, &Request::get(&url))
                    .and_then(|j| api::detail(&j).map_err(NetError::Failed))
                {
                    Ok(d) => {
                        out.details_read += 1;
                        cache.videos.insert(
                            v.id.clone(),
                            CacheEntry {
                                checked_at: self.now.clone(),
                                detail: d.clone(),
                            },
                        );
                        if out.details_read % CHECKPOINT == 0 {
                            let _ = writeln!(err, "  {} details read", out.details_read);
                            if let Some(dir) = self.user_data
                                && let Err(e) = store::write_cache(dir, cache)
                            {
                                let _ = writeln!(err, "Warning: {e}");
                            }
                        }
                        Some(d)
                    }
                    Err(NetError::RateLimited(m)) => {
                        let _ = writeln!(
                            err,
                            "Warning: PMVHaven rate limited us ({m}); no more details this run."
                        );
                        stopped = true;
                        out.rate_limited = true;
                        cached.map(|e| e.detail.clone())
                    }
                    Err(NetError::Failed(m)) => {
                        let _ = writeln!(err, "Warning: {} ({}): {m}", v.title, v.id);
                        match cached {
                            Some(e) => Some(e.detail.clone()),
                            None => {
                                out.drop("detail-failed");
                                continue;
                            }
                        }
                    }
                }
            };
            let Some(detail) = detail else {
                out.left += 1;
                continue;
            };
            if let Some(key) = screened(self.prefs, v, &tags, &detail.description) {
                out.drop(&key);
                continue;
            }
            let Some(script) = script::choose(&detail, &v.uploader, self.own) else {
                out.drop("no-script");
                continue;
            };
            out.scenes
                .push(scene(v, &detail, script, self.own, self.scorer, self.known));
        }
        // Forget videos no longer listed (only after a whole list).
        if !out.partial && self.max_pages.is_none() {
            let listed: HashSet<&str> = videos.iter().map(|v| v.id.as_str()).collect();
            cache.videos.retain(|id, _| listed.contains(id.as_str()));
        }
        sort(&mut out.scenes);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use slr_scout::library;

    fn scorer(config: serde_json::Value, creators: &[&str]) -> Scorer {
        Scorer::new(
            &slr_scout::prefs::load().unwrap(),
            &library::from_json(&config),
            creators.iter().map(|c| c.to_string()).collect(),
        )
    }

    fn video() -> Video {
        api::video(&json!({
            "_id": "v1", "title": "Shrooms - Creampie PMV", "uploader": "Scruffy6",
            "originalCreator": "Vassago", "starsTags": ["Riley reid"],
            "videoUrl": "https://s3/v.mp4", "durationSeconds": 293, "aspectRatio": 2,
            "likes": 750, "views": 9000, "tags": ["Vr", "Blowjob", "Pmv，music"],
            "thumbnailSizes": {"lg": {"url": "https://s3/lg.webp"}}
        }))
        .unwrap()
    }

    fn uploader_script() -> Script {
        script::choose(
            &Detail {
                uploader_script: Some("https://s3/s.funscript".into()),
                ..Detail::default()
            },
            "Scruffy6",
            &crate::prefs::load().unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn builds_and_scores_scenes() {
        let own = crate::prefs::load().unwrap();
        let s = scorer(
            json!({"library": {"favouriteArtists": {"Riley Reid": true}, "favouriteArtistScore": 1}}),
            &["vassago"],
        );
        let d = Detail {
            stills: (0..8).map(|i| format!("https://s3/t{i}.webp")).collect(),
            file_size: Some(123),
            ..Detail::default()
        };
        let known = Known::default();
        let sc = scene(&video(), &d, uploader_script(), &own, &s, &known);
        assert_eq!(sc.studio.as_deref(), Some("Vassago"));
        assert_eq!(sc.url, "https://pmvhaven.com/video/shrooms-creampie-pmv_v1");
        assert!(sc.vr && sc.vr_suffix == "_180_LR");
        assert!(sc.pmv && sc.creampie && !sc.compilation);
        assert_eq!(sc.favourite.as_deref(), Some("Riley Reid"));
        assert_eq!(sc.images.len(), IMAGES);
        assert_eq!(sc.tags, ["vr", "blowjob", "pmv-music", "pmvmusic", "pmv"]);
        let parts: Vec<(&str, f64)> = sc
            .score_parts
            .iter()
            .map(|(k, v)| (k.as_str(), *v))
            .collect();
        // likes 750 / popularityPer 375 = 2; PMV counts once in the Music group.
        assert_eq!(
            parts,
            [
                ("artistInLibrary", 2.0),
                ("favourite", 1.0),
                ("likes", 2.0),
                ("tag:pmv", 2.0),
                ("vr", 3.0)
            ]
        );
        assert_eq!(sc.score, 10.0);
        assert!(!sc.in_library);
        let known = Known {
            ids: HashSet::from(["v1".to_string()]),
            ..Known::default()
        };
        assert!(scene(&video(), &d, uploader_script(), &own, &s, &known).in_library);
    }

    #[test]
    fn screens_with_creators_tags_and_description() {
        let prefs = slr_scout::prefs::load().unwrap();
        let v = video();
        let t = tags(&v, true, false);
        assert_eq!(screened(&prefs, &v, &t, ""), None);
        assert_eq!(
            screened(&prefs, &v, &t, "a loli edit").as_deref(),
            Some("excluded")
        );
        let mut b = v.clone();
        b.stars.push("Aiden Het".into());
        assert_eq!(screened(&prefs, &b, &t, "").as_deref(), Some("blocked"));
        let mut s = v.clone();
        s.tags.push("Non-nude".into());
        let st = tags(&s, false, false);
        assert_eq!(screened(&prefs, &s, &st, "").as_deref(), Some("sfw"));
        let mut short = v;
        short.duration_s = Some(60);
        assert_eq!(
            screened(&prefs, &short, &t, "").as_deref(),
            Some("too-short")
        );
    }
}
