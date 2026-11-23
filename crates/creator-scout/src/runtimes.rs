//! `--runtimes N`: EroScripts posts filed without their length get it from
//! their main script, so their cards on the swipe deck show the runtime: the
//! topic's first post (with the forum session; paced), its stroke script
//! (the one matching the post title, else the first), downloaded to the
//! work folder and timed by its last action. A post without a readable
//! stroke script is tried again after [`MISS_DAYS`] days.

use crate::discover::{self, combine};
use crate::post_images::{self, Waiting, json_unless_gone, now_secs};
use crate::profiles::Pacers;
use crate::state::{State, Swipes};
use crate::store::{self, CreatorsFile, Post};
use crate::{eroscripts, judge};
use serde::Deserialize;
use slr_scout::forum::{self, Attachment};
use slr_scout::http::{Http, NetError, Pacer};
use std::fs;
use std::io::Write;
use std::path::Path;

pub use post_images::MISS_DAYS;

fn wants_runtime(p: &Post, swipes: &Swipes) -> bool {
    p.kind == "eroscripts" && p.duration_s.is_none() && post_images::open_post(p, swipes)
}

/// The EroScripts posts whose runtime is due: none yet, downloadable, not
/// in the library, not swiped, of a creator not swiped no, not missed in the
/// last [`MISS_DAYS`] days; in `--post-images`' order.
pub fn due(file: &CreatorsFile, state: &State, swipes: &Swipes, now: i64) -> Vec<Waiting> {
    post_images::waiting(file, swipes, &state.runtime_misses, now, |p| {
        wants_runtime(p, swipes)
    })
}

#[derive(Deserialize)]
struct Funscript {
    actions: Vec<Action>,
}

#[derive(Deserialize)]
struct Action {
    at: f64,
}

/// A funscript's length in whole seconds: its latest action (`at`, ms),
/// rounded. `None` for no actions, a length of 0 or a file that isn't a
/// funscript.
pub fn script_length_s(raw: &[u8]) -> Option<u64> {
    let raw = raw.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(raw);
    let script: Funscript = serde_json::from_slice(raw).ok()?;
    let last = script
        .actions
        .iter()
        .map(|a| a.at)
        .filter(|at| at.is_finite())
        .fold(0.0, f64::max);
    let secs = (last / 1000.0).round() as u64;
    (secs > 0).then_some(secs)
}

/// The post's main script: the stroke script [`forum::pick_scripts`] picks
/// for its title, else the first stroke script.
pub fn main_script<'a>(scripts: &'a [Attachment], title: &str) -> Option<&'a Attachment> {
    forum::pick_scripts(scripts, title)
        .map(|(main, _)| main)
        .or_else(|| {
            scripts
                .iter()
                .find(|a| matches!(forum::script_axis(&a.name), Some((_, None))))
        })
}

#[derive(Debug, Default, PartialEq)]
pub struct Run {
    pub code: i32,
    pub filled: usize,
    pub missed: usize,
    pub failed: usize,
    /// Due posts this run didn't get to.
    pub left: usize,
    /// Posts skipped for want of the forum session.
    pub forum_skipped: usize,
}

enum Outcome {
    /// Seconds, and the script's name.
    Timed(u64, String),
    /// No stroke script, or it won't download or read: recorded.
    Missing(String),
    /// The forum didn't answer as it should: tried again next run.
    Failed(NetError),
}

/// A post's runtime from its main script, downloaded to `work` and removed.
fn post_runtime(
    http: &dyn Http,
    pacer: &mut Pacer,
    cookie: &str,
    work: &Path,
    id: &str,
    title: &str,
) -> Outcome {
    let Ok(topic) = id.parse::<u64>() else {
        return Outcome::Missing("not a topic id".into());
    };
    pacer.wait();
    let req = eroscripts::topic_request(eroscripts::FORUM, cookie, topic);
    let v = match json_unless_gone(http, &req) {
        Ok(Some(v)) => v,
        Ok(None) => return Outcome::Missing("gone from the forum".into()),
        Err(e) => return Outcome::Failed(e),
    };
    let scripts = forum::post_scripts(
        v["post_stream"]["posts"][0]["cooked"]
            .as_str()
            .unwrap_or_default(),
    );
    let Some(main) = main_script(&scripts, title) else {
        return Outcome::Missing("no stroke script attached".into());
    };
    let dest = work.join(format!("runtime-{topic}.funscript"));
    // The real client resumes a partial file: start from nothing.
    let _ = fs::remove_file(&dest);
    let got = forum::download_upload(http, pacer, cookie, &main.url, &dest);
    let raw = fs::read(&dest);
    let _ = fs::remove_file(&dest);
    match got {
        Ok(()) => {}
        Err(e @ NetError::RateLimited(_)) => return Outcome::Failed(e),
        Err(NetError::Failed(m)) => return Outcome::Missing(format!("script: {m}")),
    }
    match raw.ok().as_deref().and_then(script_length_s) {
        Some(secs) => Outcome::Timed(secs, main.name.clone()),
        None => Outcome::Missing(format!("{}: no readable actions", main.name)),
    }
}

pub struct Runtimes<'a> {
    pub http: &'a dyn Http,
    pub user_data: &'a Path,
    /// The forum session cookie (`_t`).
    pub cookie: Option<&'a str>,
    /// Canned network (tests): no pacing.
    pub fake: bool,
}

/// `m:ss` (`h:mm:ss` from an hour).
fn clock(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// `--runtimes N`. Writes creators.json after each creator that got a
/// runtime, and the state after each creator.
pub fn run(
    r: &Runtimes,
    file: &mut CreatorsFile,
    state: &mut State,
    swipes: &Swipes,
    limit: usize,
    out: &mut dyn Write,
) -> Result<Run, String> {
    let work = due(file, state, swipes, now_secs());
    let mut run = Run::default();
    let Some(cookie) = r.cookie else {
        run.forum_skipped = work.len();
        if run.forum_skipped > 0 {
            let _ = writeln!(
                out,
                "EroScripts: no session cookie (log in to EroScripts in the app); {} post(s) skipped.",
                run.forum_skipped
            );
            run.code = 3;
        }
        return Ok(run);
    };
    let dir = r.user_data.join(judge::WORK_DIR);
    if !work.is_empty() {
        fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut pacer = Pacers::new(r.fake).forum;
    let take = work.len().min(limit);
    let mut done = 0;
    'creators: for chunk in work[..take].chunk_by(|a, b| a.slug == b.slug) {
        let slug = &chunk[0].slug;
        let mut changed = false;
        for w in chunk {
            let Some(c) = file.creators.get_mut(slug) else {
                continue 'creators;
            };
            let Some(post) = c
                .posts
                .iter_mut()
                .find(|x| x.kind == w.kind && x.id == w.id)
            else {
                continue;
            };
            let outcome = post_runtime(r.http, &mut pacer, cookie, &dir, &w.id, &post.title);
            let key = store::post_key(&w.kind, &w.id);
            let what = format!("  {}: {} {}", c.name, w.kind, w.id);
            match outcome {
                Outcome::Timed(secs, script) => {
                    let _ = writeln!(out, "{what}: {} ({script})", clock(secs));
                    post.duration_s = Some(secs);
                    changed = true;
                    state.runtime_misses.remove(&key);
                    run.filled += 1;
                }
                Outcome::Missing(why) => {
                    let _ = writeln!(out, "{what}: {why}; tried again in {MISS_DAYS} days");
                    state
                        .runtime_misses
                        .insert(key, video_tags::timefmt::now_iso());
                    run.missed += 1;
                }
                Outcome::Failed(NetError::RateLimited(m)) => {
                    let _ = writeln!(out, "{what}: rate limited ({m}); stopping for today.");
                    run.code = combine(run.code, 2);
                    if changed {
                        store::write(r.user_data, file)?;
                    }
                    crate::state::write(r.user_data, state)?;
                    break 'creators;
                }
                Outcome::Failed(e) => {
                    discover::note_net(&e, &what, &mut run.code, out);
                    run.failed += 1;
                }
            }
            done += 1;
        }
        if changed {
            store::write(r.user_data, file)?;
        }
        crate::state::write(r.user_data, state)?;
    }
    run.left = work.len() - done;
    if run.left > 0 {
        run.code = combine(run.code, 2);
    }
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Creator;

    #[test]
    fn script_length_is_the_latest_action_rounded() {
        let raw = br#"{"version":"1.0","actions":[{"at":0,"pos":0},{"at":125600,"pos":90},{"at":61000,"pos":10}]}"#;
        assert_eq!(script_length_s(raw), Some(126));
        let bom = b"\xEF\xBB\xBF{\"actions\":[{\"at\":1499.4,\"pos\":0}]}";
        assert_eq!(script_length_s(bom), Some(1));
        assert_eq!(script_length_s(br#"{"actions":[]}"#), None);
        assert_eq!(script_length_s(br#"{"actions":[{"at":0,"pos":0}]}"#), None);
        assert_eq!(
            script_length_s(br#"{"actions":[{"at":400,"pos":0}]}"#),
            None
        );
        assert_eq!(script_length_s(b"{\"actions\":[{\"at\":"), None);
        assert_eq!(script_length_s(b"<html>not found</html>"), None);
        assert_eq!(script_length_s(br#"{"version":"1.0"}"#), None);
    }

    fn att(name: &str) -> Attachment {
        Attachment {
            name: name.into(),
            url: format!("https://discuss.eroscripts.com/uploads/{name}"),
        }
    }

    #[test]
    fn main_script_by_title_else_the_first_stroke_script() {
        let scripts = [
            att("Intro.roll.funscript"),
            att("Intro.funscript"),
            att("Riding Session.funscript"),
            att("Riding Session.twist.funscript"),
        ];
        let pick = |title: &str| main_script(&scripts, title).map(|a| a.name.as_str());
        assert_eq!(
            pick("Lucy - Riding Session"),
            Some("Riding Session.funscript")
        );
        assert_eq!(pick("Something else"), Some("Intro.funscript"));
        assert_eq!(main_script(&[att("Only.roll.funscript")], "Only"), None);
        assert_eq!(main_script(&[], "x"), None);
    }

    #[test]
    fn due_posts_are_open_eroscripts_posts_without_a_runtime() {
        let post = |kind: &str, id: &str| Post {
            kind: kind.into(),
            id: id.into(),
            downloadable: true,
            ..Post::default()
        };
        let mut file = CreatorsFile::empty();
        file.creators.insert(
            "a".into(),
            Creator {
                posts: vec![
                    post("eroscripts", "1"),
                    Post {
                        duration_s: Some(60),
                        ..post("eroscripts", "2")
                    },
                    post("faptap", "3"),
                    Post {
                        in_library: true,
                        ..post("eroscripts", "4")
                    },
                    post("eroscripts", "5"),
                    post("eroscripts", "6"),
                ],
                ..Creator::default()
            },
        );
        let mut swipes = Swipes::default();
        swipes.posts.insert("eroscripts:6".into(), "want".into());
        let mut state = State::default();
        let now = crate::instagram::parse_iso("2026-11-23T00:00:00Z").unwrap();
        state
            .runtime_misses
            .insert("eroscripts:5".into(), "2026-11-20T00:00:00.000Z".into());
        let ids: Vec<String> = due(&file, &state, &swipes, now)
            .into_iter()
            .map(|w| w.id)
            .collect();
        assert_eq!(ids, ["1"]);
        assert_eq!(clock(59), "0:59");
        assert_eq!(clock(3725), "1:02:05");
    }
}
