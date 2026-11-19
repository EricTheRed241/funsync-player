//! `--discover`: the feeds since the cursors → the judge → verified new
//! creators (one profile confirmed hers is enough: Instagram, Twitch or a
//! public profile page, crate::profiles) → their posts and pictures. The
//! cursors move only when every title of the run was judged and every new
//! creator checked, so nothing is skipped when a step fails (the next run
//! judges those titles again; posts already filed aren't).

use crate::eroscripts::{self, Topic};
use crate::faptap::{self, Video};
use crate::judge::{self, Batch, Handles, Item, Known};
use crate::library::{LibraryIds, Scoring};
use crate::profiles::{self, Finds, Pacers};
use crate::state::{State, Unverified};
use crate::store::{self, Creator, CreatorsFile, EroscriptsChecked, FaptapChecked};
use slr_scout::http::{Http, NetError, Pacer};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Write;
use std::path::Path;

/// A name searched without finding an account isn't searched again for
/// this long.
pub const UNVERIFIED_DAYS: i64 = 30;

/// Exit codes of steps combined: the first that is neither 0 nor 2, else
/// 2 when any is, else 0.
pub fn combine(a: i32, b: i32) -> i32 {
    match (a, b) {
        (a, _) if a != 0 && a != 2 => a,
        (_, b) if b != 0 && b != 2 => b,
        (a, b) => a.max(b),
    }
}

pub struct Discover<'a> {
    pub http: &'a dyn Http,
    pub user_data: &'a Path,
    /// The forum session cookie (`_t`).
    pub cookie: Option<String>,
    pub judge_cmd: String,
    /// Canned network (tests): no pacing.
    pub fake: bool,
    pub scoring: &'a Scoring,
    pub library: &'a LibraryIds,
}

enum Title<'a> {
    Topic(&'a Topic),
    Video(&'a Video),
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// What filing a post needs (`--discover` and `--favourites`): the
/// network, where pictures go, the forum session cookie, the scores.
pub struct Filing<'a> {
    pub http: &'a dyn Http,
    pub user_data: &'a Path,
    pub cookie: Option<&'a str>,
    pub scoring: &'a Scoring,
}

impl Discover<'_> {
    fn filing(&self) -> Filing<'_> {
        Filing {
            http: self.http,
            user_data: self.user_data,
            cookie: self.cookie.as_deref(),
            scoring: self.scoring,
        }
    }
}

/// Download a post picture (forum ones with the cookie) into her folder
/// unless it is there.
pub fn save_image(
    http: &dyn Http,
    user_data: &Path,
    cookie: Option<&str>,
    pacer: &mut Pacer,
    url: &str,
    slug: &str,
    name: &str,
) -> Result<bool, String> {
    let dir = store::pictures_dir(user_data, slug);
    let dest = dir.join(name);
    if dest.is_file() {
        return Ok(false);
    }
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = dir.join(format!(".{name}.part"));
    let r = eroscripts::download_image(
        http,
        pacer,
        eroscripts::FORUM,
        cookie.unwrap_or_default(),
        url,
        &part,
    )
    .and_then(|_| fs::rename(&part, &dest).map_err(|e| e.to_string()));
    if r.is_err() {
        let _ = fs::remove_file(&part);
    }
    r.map(|_| true)
}

/// Read a topic's details into its post (hosts, image); `Err` leaves the
/// post as listed and the topic waiting in `pendingDetails`.
pub fn add_details(
    f: &Filing,
    forum: &mut Pacer,
    c: &mut Creator,
    slug: &str,
    id: u64,
) -> Result<(), NetError> {
    let cookie = f.cookie.unwrap_or_default();
    let v = eroscripts::topic_json(f.http, forum, eroscripts::FORUM, cookie, id)?;
    let Some((mut post, image)) = eroscripts::detailed_post(&v, eroscripts::FORUM) else {
        // No longer a script post (moved, AI-tagged): drop it.
        c.posts
            .retain(|x| !(x.kind == "eroscripts" && x.id == id.to_string()));
        return Ok(());
    };
    if let Some(url) = image {
        let name = store::eroscripts_picture(id, 0);
        // A picture that won't download leaves the post without one.
        if save_image(f.http, f.user_data, f.cookie, forum, &url, slug, &name).is_ok() {
            post.image = Some(name);
        }
    }
    post.score = Some(f.scoring.score(&post, &c.name));
    c.upsert_post(post);
    Ok(())
}

/// File a FapTap video under `c`, its thumbnail saved as her picture.
pub fn add_video(f: &Filing, faptap: &mut Pacer, c: &mut Creator, slug: &str, v: &Video) {
    let mut post = faptap::post(v);
    if let (Some(url), Some(name)) = (&v.thumbnail, store::faptap_picture(&v.id)) {
        let dir = store::pictures_dir(f.user_data, slug);
        faptap.wait();
        if crate::instagram::save(f.http, url, &dir, &name).is_ok() {
            post.image = Some(name);
        }
    }
    post.score = Some(f.scoring.score(&post, &c.name));
    c.upsert_post(post);
}

pub fn note_net(e: &NetError, what: &str, code: &mut i32, out: &mut dyn Write) {
    let _ = writeln!(out, "{what}: {e}");
    *code = combine(
        *code,
        match e {
            NetError::RateLimited(_) => 2,
            NetError::Failed(_) => 1,
        },
    );
}

pub fn discover(
    d: &Discover,
    file: &mut CreatorsFile,
    state: &mut State,
    out: &mut dyn Write,
) -> Result<i32, String> {
    let mut p = Pacers::new(d.fake);
    let mut code = 0;
    let now = video_tags::timefmt::now_iso();
    let mut touched: BTreeSet<String> = BTreeSet::new();

    // Posts filed earlier whose details couldn't be read.
    if d.cookie.is_some() {
        for (id, slug) in state.pending_details.clone() {
            let (Ok(topic), Some(c)) = (id.parse::<u64>(), file.creators.get_mut(&slug)) else {
                state.pending_details.remove(&id);
                continue;
            };
            match add_details(&d.filing(), &mut p.forum, c, &slug, topic) {
                Ok(()) => {
                    state.pending_details.remove(&id);
                    touched.insert(slug);
                }
                Err(e) => {
                    note_net(&e, &format!("EroScripts post {id}"), &mut code, out);
                    if matches!(e, NetError::RateLimited(_)) {
                        break;
                    }
                }
            }
        }
    }

    // The feeds.
    let es_cursor = file.eroscripts_checked.as_ref().map(|c| c.newest_topic_id);
    let es = match &d.cookie {
        None => {
            let _ = writeln!(
                out,
                "EroScripts: no session cookie (log in to EroScripts in the app); skipped."
            );
            code = combine(code, 3);
            None
        }
        Some(cookie) => {
            let pages = if es_cursor.is_some() {
                eroscripts::MAX_PAGES
            } else {
                eroscripts::FIRST_RUN_PAGES
            };
            match eroscripts::list_new(
                d.http,
                &mut p.forum,
                eroscripts::FORUM,
                cookie,
                es_cursor,
                pages,
            ) {
                Ok(l) => Some(l),
                Err(e) => {
                    note_net(&e, "EroScripts feed", &mut code, out);
                    None
                }
            }
        }
    };
    let ft_cursor = file
        .faptap_checked
        .as_ref()
        .map(|c| c.newest_video_id.clone());
    let ft = {
        let pages = if ft_cursor.is_some() {
            faptap::MAX_PAGES
        } else {
            faptap::FIRST_RUN_PAGES
        };
        match faptap::list_new(
            d.http,
            &mut p.faptap,
            faptap::SITE,
            ft_cursor.as_deref(),
            pages,
        ) {
            Ok(l) => Some(l),
            Err(e) => {
                note_net(&e, "FapTap feed", &mut code, out);
                None
            }
        }
    };
    for (more, what) in [
        (es.as_ref().is_some_and(|l| l.more_left), "EroScripts"),
        (ft.as_ref().is_some_and(|l| l.more_left), "FapTap"),
    ] {
        if more {
            let _ = writeln!(
                out,
                "{what}: more new pages than this run reads; the rest next run."
            );
            code = combine(code, 2);
        }
    }

    // New titles: those not filed yet.
    let filed = file.post_keys();
    let mut titles: BTreeMap<String, Title> = BTreeMap::new();
    for t in es.iter().flat_map(|l| &l.topics) {
        let key = store::post_key("eroscripts", &t.id.to_string());
        if !filed.contains(&key) {
            titles.insert(key, Title::Topic(t));
        }
    }
    for v in ft.iter().flat_map(|l| &l.videos) {
        let key = store::post_key("faptap", &v.id);
        if !filed.contains(&key) {
            titles.insert(key, Title::Video(v));
        }
    }
    let _ = writeln!(
        out,
        "EroScripts: {} new script post(s); FapTap: {} new video(s); {} to judge.",
        es.as_ref().map_or(0, |l| l.topics.len()),
        ft.as_ref().map_or(0, |l| l.videos.len()),
        titles.len()
    );

    let mut complete = true;
    let mut added_posts = 0usize;
    if !titles.is_empty() {
        let fresh_cutoff = now_secs() - UNVERIFIED_DAYS * 86_400;
        let batch = Batch {
            titles: titles
                .iter()
                .map(|(key, t)| match t {
                    Title::Topic(t) => Item {
                        key: key.clone(),
                        title: t.title.clone(),
                        tags: t.tags.clone(),
                        performers: Vec::new(),
                        uploader: None,
                    },
                    Title::Video(v) => Item {
                        key: key.clone(),
                        title: v.title.clone(),
                        tags: v.tags.clone(),
                        performers: v.performers.clone(),
                        uploader: v.uploader.clone(),
                    },
                })
                .collect(),
            known: file
                .creators
                .values()
                .map(|c| Known {
                    name: c.name.clone(),
                    aliases: c.aliases.clone(),
                })
                .collect(),
            skip: state
                .unverified
                .values()
                .filter(|u| crate::instagram::parse_iso(&u.at).is_some_and(|t| t >= fresh_cutoff))
                .map(|u| u.name.clone())
                .collect(),
        };
        let work = d.user_data.join(judge::WORK_DIR);
        let (batch_path, results_path) = judge::write_batch(&work, &batch)?;
        let keys: Vec<String> = titles.keys().cloned().collect();
        let _ = writeln!(out, "Judging {} title(s)...", keys.len());
        let results = judge::run(
            &d.judge_cmd,
            &judge::prompt(&batch_path, &results_path),
            &work,
        )
        .and_then(|_| judge::read_results(&results_path, &keys));
        let results = match results {
            Ok(r) => r,
            Err(e) => {
                let _ = writeln!(out, "Judge: {e}; these titles are judged again next run.");
                store::write(d.user_data, file)?;
                crate::state::write(d.user_data, state)?;
                return Ok(combine(code, 1));
            }
        };
        // Creator slug → (name, keys).
        let mut by_creator: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
        for (key, name) in &results.titles {
            let Some(name) = name else { continue };
            let slug = file.find(name).unwrap_or_else(|| store::slug(name));
            by_creator
                .entry(slug)
                .or_insert_with(|| (name.clone(), Vec::new()))
                .1
                .push(key.clone());
        }
        let handles: HashMap<String, &Handles> = results
            .creators
            .iter()
            .map(|h| (store::slug(&h.name), h))
            .collect();
        let _ = writeln!(
            out,
            "Judge: {} of {} title(s) by {} creator(s).",
            by_creator.values().map(|(_, k)| k.len()).sum::<usize>(),
            keys.len(),
            by_creator.len()
        );
        let mut new_creators: Vec<String> = Vec::new();
        let mut new_finds: HashMap<String, Finds> = HashMap::new();
        for (slug, (name, keys)) in &by_creator {
            if !file.creators.contains_key(slug) {
                let none = Handles::default();
                let h = handles.get(slug).copied().unwrap_or(&none);
                if h.instagram.is_empty() && h.twitch.is_empty() && h.profiles.is_empty() {
                    if !state.unverified.contains_key(slug) {
                        state.unverified.insert(
                            slug.clone(),
                            Unverified {
                                name: name.clone(),
                                at: now.clone(),
                            },
                        );
                    }
                    let _ = writeln!(out, "  {name}: no account or profile to check; skipped.");
                    continue;
                }
                let names = [name.clone()];
                let finds = match profiles::verify(d.http, &mut p, &names, h, false, false, &now) {
                    Ok(found) => found,
                    Err(e) => {
                        note_net(&e, &format!("  {name}"), &mut code, out);
                        complete = false;
                        if matches!(e, NetError::RateLimited(_)) {
                            break;
                        }
                        continue;
                    }
                };
                if !finds.any() {
                    state.unverified.insert(
                        slug.clone(),
                        Unverified {
                            name: name.clone(),
                            at: now.clone(),
                        },
                    );
                    let _ = writeln!(
                        out,
                        "  {name}: no Instagram, Twitch or profile page confirmed hers; skipped."
                    );
                    continue;
                }
                let _ = writeln!(out, "  New creator {name}: {}", finds.describe());
                state.unverified.remove(slug);
                file.creators.insert(
                    slug.clone(),
                    Creator {
                        name: name.clone(),
                        first_seen: Some(now.clone()),
                        instagram: finds.instagram.clone(),
                        twitch: finds.twitch.clone(),
                        profiles: finds.profiles.clone(),
                        ..Creator::default()
                    },
                );
                new_finds.insert(slug.clone(), finds);
                new_creators.push(slug.clone());
            }
            let Some(c) = file.creators.get_mut(slug) else {
                continue;
            };
            for key in keys {
                match titles.get(key) {
                    Some(Title::Topic(t)) => {
                        let mut post = eroscripts::listed_post(t);
                        post.score = Some(d.scoring.score(&post, &c.name));
                        c.upsert_post(post);
                        if let Err(e) = add_details(&d.filing(), &mut p.forum, c, slug, t.id) {
                            note_net(&e, &format!("  EroScripts post {}", t.id), &mut code, out);
                            state.pending_details.insert(t.id.to_string(), slug.clone());
                        }
                    }
                    Some(Title::Video(v)) => add_video(&d.filing(), &mut p.faptap, c, slug, v),
                    None => continue,
                }
                added_posts += 1;
            }
            touched.insert(slug.clone());
        }

        // A new creator's other FapTap videos, by her name.
        for slug in &new_creators {
            let Some(c) = file.creators.get(slug) else {
                continue;
            };
            let names: Vec<String> = std::iter::once(c.name.clone())
                .chain(c.aliases.iter().cloned())
                .collect();
            let found = match faptap::search(d.http, &mut p.faptap, faptap::SITE, &c.name) {
                Ok(v) => v,
                Err(e) => {
                    note_net(&e, &format!("  FapTap search {}", c.name), &mut code, out);
                    continue;
                }
            };
            let filed = file.post_keys();
            let mine: Vec<Video> = found
                .into_iter()
                .filter(|v| faptap::by_creator(v, &names))
                .filter(|v| !filed.contains(&store::post_key("faptap", &v.id)))
                .collect();
            if mine.is_empty() {
                continue;
            }
            let c = file.creators.get_mut(slug).ok_or("creator vanished")?;
            for v in &mine {
                add_video(&d.filing(), &mut p.faptap, c, slug, v);
            }
            added_posts += mine.len();
            let _ = writeln!(out, "  {}: {} FapTap video(s) by name", c.name, mine.len());
        }

        // A new creator's pictures: her profiles', the judge's on pages
        // that passed, her posts' video pages'.
        for slug in &new_creators {
            let (Some(c), Some(finds)) = (file.creators.get_mut(slug), new_finds.get(slug)) else {
                continue;
            };
            let mut found = finds.pictures.clone();
            match profiles::video_pictures(d.http, &mut p.pages, c) {
                Ok(v) => found.extend(v),
                Err(e) => note_net(&e, &format!("  {}: video pages", c.name), &mut code, out),
            }
            let n = profiles::add_pictures(d.http, d.user_data, slug, c, &found, out);
            if n > 0 {
                let _ = writeln!(out, "  {}: {n} picture(s)", c.name);
            }
        }
    }

    if complete {
        if let Some(n) = es.as_ref().and_then(|l| l.newest)
            && es_cursor.is_none_or(|c| n > c)
        {
            file.eroscripts_checked = Some(EroscriptsChecked { newest_topic_id: n });
        }
        if let Some(n) = ft.as_ref().and_then(|l| l.newest.clone())
            && ft_cursor
                .as_deref()
                .is_none_or(|c| faptap::cmp_ids(&n, c).is_gt())
        {
            file.faptap_checked = Some(FaptapChecked { newest_video_id: n });
        }
    } else {
        code = combine(code, 2);
    }
    d.library.mark(file);
    for (slug, c) in file.creators.iter_mut() {
        c.update_score();
        store::rebuild_pictures(c);
        if touched.contains(slug) {
            store::prune(d.user_data, slug, c);
        }
    }
    let _ = writeln!(
        out,
        "Discover: {added_posts} post(s) filed; {} creator(s), {} post(s) in all.",
        file.creators.len(),
        file.post_count()
    );
    store::write(d.user_data, file)?;
    crate::state::write(d.user_data, state)?;
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combines_codes() {
        assert_eq!(combine(0, 0), 0);
        assert_eq!(combine(0, 2), 2);
        assert_eq!(combine(2, 0), 2);
        assert_eq!(combine(2, 3), 3);
        assert_eq!(combine(1, 3), 1);
        assert_eq!(combine(3, 1), 3);
        assert_eq!(combine(0, 1), 1);
    }
}
