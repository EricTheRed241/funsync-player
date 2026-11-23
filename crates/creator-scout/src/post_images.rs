//! `--post-images N`: posts filed without their picture get it, so their
//! cards on the swipe deck show the scene: a FapTap video's thumbnail (its
//! open API) and an EroScripts post's first image (the topic, with the forum
//! session; paced). A post without one, or whose picture won't download, is
//! tried again after [`MISS_DAYS`] days.

use crate::discover::{self, combine};
use crate::profiles::Pacers;
use crate::state::{State, Swipes};
use crate::store::{self, CreatorsFile, Post};
use crate::{eroscripts, faptap, instagram};
use serde_json::Value;
use slr_scout::http::{self, Http, NetError, Request};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

/// A post without a picture isn't tried again for this long.
pub const MISS_DAYS: i64 = 7;

/// A post waiting for its picture (or, for `--runtimes`, its length).
#[derive(Debug, Clone, PartialEq)]
pub struct Waiting {
    pub slug: String,
    pub kind: String,
    pub id: String,
}

/// Whether `key`'s miss (post key → when) is under [`MISS_DAYS`] old.
pub fn missed_recently(misses: &BTreeMap<String, String>, key: &str, now: i64) -> bool {
    misses
        .get(key)
        .and_then(|t| instagram::parse_iso(t))
        .is_some_and(|t| now - t < MISS_DAYS * 86_400)
}

/// A post still to be had: downloadable, not in the library, not swiped.
pub fn open_post(p: &Post, swipes: &Swipes) -> bool {
    p.downloadable && !p.in_library && !swipes.posts.contains_key(&p.key())
}

fn wants_image(p: &Post, swipes: &Swipes) -> bool {
    p.image.is_none() && (p.kind == "faptap" || p.kind == "eroscripts") && open_post(p, swipes)
}

/// The posts `wants` takes, of creators not swiped no, not missed in the
/// last [`MISS_DAYS`] days (`misses`). Creators with a source first
/// (swiped, favourite, pornstar list), then by slug; her posts in their
/// order.
pub fn waiting(
    file: &CreatorsFile,
    swipes: &Swipes,
    misses: &BTreeMap<String, String>,
    now: i64,
    wants: impl Fn(&Post) -> bool,
) -> Vec<Waiting> {
    let mut creators: Vec<_> = file
        .creators
        .iter()
        .filter(|(slug, _)| !swipes.creator_is_no(slug))
        .collect();
    creators.sort_by_key(|(slug, c)| (c.source_rank(), *slug));
    creators
        .into_iter()
        .flat_map(|(slug, c)| {
            c.posts
                .iter()
                .filter(|p| wants(p) && !missed_recently(misses, &p.key(), now))
                .map(move |p| Waiting {
                    slug: slug.clone(),
                    kind: p.kind.clone(),
                    id: p.id.clone(),
                })
        })
        .collect()
}

/// The posts whose picture is due: no image, an open post ([`open_post`]),
/// as [`waiting`] picks them.
pub fn due(file: &CreatorsFile, state: &State, swipes: &Swipes, now: i64) -> Vec<Waiting> {
    waiting(file, swipes, &state.post_image_misses, now, |p| {
        wants_image(p, swipes)
    })
}

/// Misses (post key → when) in the last [`MISS_DAYS`] days.
pub fn missed(misses: &BTreeMap<String, String>, now: i64) -> usize {
    misses
        .keys()
        .filter(|k| missed_recently(misses, k, now))
        .count()
}

#[derive(Debug, Default, PartialEq)]
pub struct Run {
    pub code: i32,
    pub filled: usize,
    pub missed: usize,
    pub failed: usize,
    /// Due posts this run didn't get to.
    pub left: usize,
    /// EroScripts posts skipped for want of the forum session.
    pub forum_skipped: usize,
}

enum Outcome {
    /// The picture's file name.
    Filled(String),
    /// No picture, or it won't download: recorded.
    Missing(String),
    /// The site didn't answer as it should: tried again next run.
    Failed(NetError),
}

/// A JSON document; `None` when it is gone (404 / 410).
pub(crate) fn json_unless_gone(http: &dyn Http, req: &Request) -> Result<Option<Value>, NetError> {
    let res = http
        .send(req)
        .map_err(|e| NetError::Failed(format!("{}: {e}", req.url)))?;
    if matches!(res.status, 404 | 410) {
        return Ok(None);
    }
    http::parse_json(&res, &req.url).map(Some)
}

/// A FapTap video's thumbnail (`/api/videos/<id>`), saved as `ft-<id>.jpg`.
fn faptap_image(
    http: &dyn Http,
    pacers: &mut Pacers,
    user_data: &Path,
    slug: &str,
    id: &str,
) -> Outcome {
    let Some(name) = store::faptap_picture(id) else {
        return Outcome::Missing("not a FapTap video id".into());
    };
    pacers.faptap.wait();
    let req = Request::get(format!("{}/api/videos/{id}", faptap::SITE))
        .header("Accept", "application/json");
    let v = match json_unless_gone(http, &req) {
        Ok(Some(v)) => v,
        Ok(None) => return Outcome::Missing("gone from FapTap".into()),
        Err(e) => return Outcome::Failed(e),
    };
    let Some(url) = faptap::thumbnail_url(&v["data"], faptap::SITE) else {
        return Outcome::Missing("no thumbnail".into());
    };
    pacers.faptap.wait();
    match instagram::save(http, &url, &store::pictures_dir(user_data, slug), &name) {
        Ok(_) => Outcome::Filled(name),
        Err(e) => Outcome::Missing(format!("thumbnail: {e}")),
    }
}

/// An EroScripts post's first image (the topic's first post), saved as
/// `es-<topic>-0.jpg`.
fn eroscripts_image(
    http: &dyn Http,
    pacers: &mut Pacers,
    cookie: &str,
    user_data: &Path,
    slug: &str,
    id: &str,
) -> Outcome {
    let Ok(topic) = id.parse::<u64>() else {
        return Outcome::Missing("not a topic id".into());
    };
    pacers.forum.wait();
    let req = eroscripts::topic_request(eroscripts::FORUM, cookie, topic);
    let v = match json_unless_gone(http, &req) {
        Ok(Some(v)) => v,
        Ok(None) => return Outcome::Missing("gone from the forum".into()),
        Err(e) => return Outcome::Failed(e),
    };
    let cooked = v["post_stream"]["posts"][0]["cooked"]
        .as_str()
        .unwrap_or_default();
    let Some(url) = eroscripts::first_image(cooked, eroscripts::FORUM) else {
        return Outcome::Missing("no image in the post".into());
    };
    let name = store::eroscripts_picture(topic, 0);
    match discover::save_image(
        http,
        user_data,
        Some(cookie),
        &mut pacers.forum,
        &url,
        slug,
        &name,
    ) {
        Ok(_) => Outcome::Filled(name),
        Err(e) => Outcome::Missing(format!("image: {e}")),
    }
}

pub struct PostImages<'a> {
    pub http: &'a dyn Http,
    pub user_data: &'a Path,
    /// The forum session cookie (`_t`).
    pub cookie: Option<&'a str>,
    /// Canned network (tests): no pacing.
    pub fake: bool,
}

pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// `--post-images N`. Writes creators.json after each creator that got a
/// picture, and the state after each creator.
pub fn run(
    p: &PostImages,
    file: &mut CreatorsFile,
    state: &mut State,
    swipes: &Swipes,
    limit: usize,
    out: &mut dyn Write,
) -> Result<Run, String> {
    let (work, forum): (Vec<Waiting>, Vec<Waiting>) = due(file, state, swipes, now_secs())
        .into_iter()
        .partition(|w| w.kind == "faptap" || p.cookie.is_some());
    let mut run = Run {
        forum_skipped: forum.len(),
        ..Run::default()
    };
    let mut pacers = Pacers::new(p.fake);
    let take = work.len().min(limit);
    let mut done = 0;
    'creators: for chunk in work[..take].chunk_by(|a, b| a.slug == b.slug) {
        let slug = &chunk[0].slug;
        let mut changed = false;
        for w in chunk {
            let outcome = match (w.kind.as_str(), p.cookie) {
                ("eroscripts", Some(cookie)) => {
                    eroscripts_image(p.http, &mut pacers, cookie, p.user_data, slug, &w.id)
                }
                _ => faptap_image(p.http, &mut pacers, p.user_data, slug, &w.id),
            };
            let Some(c) = file.creators.get_mut(slug) else {
                continue 'creators;
            };
            let key = store::post_key(&w.kind, &w.id);
            let what = format!("  {}: {} {}", c.name, w.kind, w.id);
            match outcome {
                Outcome::Filled(name) => {
                    let _ = writeln!(out, "{what}: {name}");
                    if let Some(post) = c
                        .posts
                        .iter_mut()
                        .find(|x| x.kind == w.kind && x.id == w.id)
                    {
                        post.image = Some(name);
                        changed = true;
                    }
                    state.post_image_misses.remove(&key);
                    run.filled += 1;
                }
                Outcome::Missing(why) => {
                    let _ = writeln!(out, "{what}: {why}; tried again in {MISS_DAYS} days");
                    state
                        .post_image_misses
                        .insert(key, video_tags::timefmt::now_iso());
                    run.missed += 1;
                }
                Outcome::Failed(NetError::RateLimited(m)) => {
                    let _ = writeln!(out, "{what}: rate limited ({m}); stopping for today.");
                    run.code = combine(run.code, 2);
                    if changed {
                        store::rebuild_pictures(c);
                        store::write(p.user_data, file)?;
                    }
                    crate::state::write(p.user_data, state)?;
                    break 'creators;
                }
                Outcome::Failed(e) => {
                    discover::note_net(&e, &what, &mut run.code, out);
                    run.failed += 1;
                }
            }
            done += 1;
        }
        if changed && let Some(c) = file.creators.get_mut(slug) {
            store::rebuild_pictures(c);
            store::write(p.user_data, file)?;
        }
        crate::state::write(p.user_data, state)?;
    }
    run.left = work.len() - done;
    if run.left > 0 {
        run.code = combine(run.code, 2);
    }
    if run.forum_skipped > 0 {
        let _ = writeln!(
            out,
            "EroScripts: no session cookie (log in to EroScripts in the app); {} post(s) skipped.",
            run.forum_skipped
        );
        run.code = combine(run.code, 3);
    }
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Creator;

    fn post(kind: &str, id: &str) -> Post {
        Post {
            kind: kind.into(),
            id: id.into(),
            downloadable: true,
            ..Post::default()
        }
    }

    #[test]
    fn due_posts_skip_swiped_owned_and_recent_misses() {
        let mut file = CreatorsFile::empty();
        let with_image = Post {
            image: Some("ft-9.jpg".into()),
            ..post("faptap", "9")
        };
        let owned = Post {
            in_library: true,
            ..post("faptap", "3")
        };
        let not_downloadable = Post {
            downloadable: false,
            ..post("eroscripts", "4")
        };
        file.creators.insert(
            "b".into(),
            Creator {
                posts: vec![
                    post("faptap", "1"),
                    with_image,
                    owned,
                    not_downloadable,
                    post("eroscripts", "5"),
                    post("faptap", "6"),
                    post("faptap", "7"),
                ],
                ..Creator::default()
            },
        );
        file.creators.insert(
            "a".into(),
            Creator {
                posts: vec![post("faptap", "2")],
                ..Creator::default()
            },
        );
        file.creators.insert(
            "c".into(),
            Creator {
                posts: vec![post("faptap", "8")],
                sources: vec!["favourite".into()],
                ..Creator::default()
            },
        );
        file.creators.insert(
            "no".into(),
            Creator {
                posts: vec![post("faptap", "10")],
                ..Creator::default()
            },
        );
        let mut swipes = Swipes::default();
        swipes.creators.insert("no".into(), "no".into());
        swipes.posts.insert("faptap:6".into(), "want".into());
        let mut state = State::default();
        let now = instagram::parse_iso("2026-11-19T00:00:00Z").unwrap();
        state
            .post_image_misses
            .insert("faptap:7".into(), "2026-11-15T00:00:00.000Z".into());
        state
            .post_image_misses
            .insert("faptap:1".into(), "2026-11-01T00:00:00.000Z".into());
        let keys: Vec<String> = due(&file, &state, &swipes, now)
            .iter()
            .map(|w| format!("{}/{}", w.slug, store::post_key(&w.kind, &w.id)))
            .collect();
        assert_eq!(
            keys,
            ["c/faptap:8", "a/faptap:2", "b/faptap:1", "b/eroscripts:5"]
        );
        assert_eq!(missed(&state.post_image_misses, now), 1);
    }
}
