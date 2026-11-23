//! `--fetch` (hourly, in the `downloads` job): the posts swiped `want` on
//! the Creators site go through the scouts that download them (`npm run
//! eroscripts:scout` / `faptap:scout`: `--add <ids>`, then `--fetch --ids
//! <ids>`), run from the repo. A post counts as fetched once the library
//! has it; one that doesn't arrive is tried again once [`RETRY_HOURS`] have
//! passed, [`MAX_TRIES`] times in all. SLR-hosted posts are left to the
//! weekly `slr:scout --fetch`.

use crate::instagram::parse_iso;
use crate::library::LibraryIds;
use crate::state::{MAX_TRIES, RETRY_HOURS, State, Swipes};
use crate::store::{CreatorsFile, Post};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The repo the npm scripts live in: this crate's workspace root.
pub fn default_repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A post to fetch.
#[derive(Debug, Clone, PartialEq)]
pub struct Wanted {
    pub key: String,
    pub kind: String,
    pub id: String,
    pub title: String,
}

/// Posts swiped `want` that can be fetched: downloadable, not SLR-hosted,
/// not in the library, not fetched before, tried fewer than [`MAX_TRIES`]
/// times and not within [`RETRY_HOURS`] of `now` (seconds since the epoch;
/// an unreadable last try counts as due). In swipe-key order.
pub fn wanted(file: &CreatorsFile, swipes: &Swipes, state: &State, now: i64) -> Vec<Wanted> {
    let posts: Vec<&Post> = file.creators.values().flat_map(|c| &c.posts).collect();
    swipes
        .posts
        .iter()
        .filter(|(_, choice)| *choice == "want")
        .filter_map(|(key, _)| posts.iter().find(|p| p.key() == *key))
        .filter(|p| p.downloadable && !p.in_library && !p.hosts.iter().any(|h| h == "slr"))
        .filter(|p| {
            state.fetch.get(&p.key()).is_none_or(|r| {
                r.done_at.is_none()
                    && r.tries < MAX_TRIES
                    && r.last_try
                        .as_deref()
                        .and_then(parse_iso)
                        .is_none_or(|t| now - t >= RETRY_HOURS * 3600)
            })
        })
        .map(|p| Wanted {
            key: p.key(),
            kind: p.kind.clone(),
            id: p.id.clone(),
            title: p.title.clone(),
        })
        .collect()
}

fn script_name(kind: &str) -> Option<&'static str> {
    match kind {
        "eroscripts" => Some("eroscripts:scout"),
        "faptap" => Some("faptap:scout"),
        _ => None,
    }
}

/// The two npm commands for a kind's ids.
pub fn commands(npm: &str, kind: &str, ids: &[String]) -> Option<[String; 2]> {
    let script = script_name(kind)?;
    let ids = ids.join(",");
    Some([
        format!("{npm} run -s {script} -- --add {ids}"),
        format!("{npm} run -s {script} -- --fetch --ids {ids}"),
    ])
}

/// Run a command line with sh in `repo`; its output goes to stderr.
fn sh(line: &str, repo: &Path) -> Result<i32, String> {
    use std::os::fd::AsFd;
    let to_stderr = std::io::stderr()
        .as_fd()
        .try_clone_to_owned()
        .map_err(|e| format!("can't run {line}: {e}"))?;
    let status = Command::new("/bin/sh")
        .arg("-c")
        .arg(line)
        .current_dir(repo)
        .stdin(Stdio::null())
        .stdout(Stdio::from(to_stderr))
        .status()
        .map_err(|e| format!("can't run {line}: {e}"))?;
    Ok(status.code().unwrap_or(1))
}

pub struct FetchRun {
    pub code: i32,
    pub fetched: usize,
    pub failed: usize,
}

/// Fetch the wanted posts; `rescan` reads the library again afterwards.
#[allow(clippy::too_many_arguments)]
pub fn fetch(
    file: &mut CreatorsFile,
    swipes: &Swipes,
    state: &mut State,
    npm: &str,
    repo: &Path,
    dry_run: bool,
    rescan: &dyn Fn() -> LibraryIds,
    out: &mut dyn Write,
) -> Result<FetchRun, String> {
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let list = wanted(file, swipes, state, now_unix);
    let mut run = FetchRun {
        code: 0,
        fetched: 0,
        failed: 0,
    };
    if list.is_empty() {
        let _ = writeln!(out, "Nothing to fetch.");
        return Ok(run);
    }
    let mut codes = Vec::new();
    for kind in ["eroscripts", "faptap"] {
        let ids: Vec<String> = list
            .iter()
            .filter(|w| w.kind == kind)
            .map(|w| w.id.clone())
            .collect();
        if ids.is_empty() {
            continue;
        }
        let Some(cmds) = commands(npm, kind, &ids) else {
            continue;
        };
        if dry_run {
            for w in list.iter().filter(|w| w.kind == kind) {
                let _ = writeln!(out, "  would fetch {}: {}", w.key, w.title);
            }
            for c in &cmds {
                let _ = writeln!(out, "  would run: {c}");
            }
            continue;
        }
        let _ = writeln!(
            out,
            "Fetching {} {kind} post(s): {}",
            ids.len(),
            ids.join(",")
        );
        let mut code = sh(&cmds[0], repo)?;
        if code == 0 {
            code = sh(&cmds[1], repo)?;
        }
        codes.push((kind, code));
    }
    if dry_run {
        return Ok(run);
    }
    let lib = rescan();
    lib.mark(file);
    let now = video_tags::timefmt::now_iso();
    for w in &list {
        let code = codes
            .iter()
            .find(|(k, _)| *k == w.kind)
            .map_or(0, |(_, c)| *c);
        let rec = state.fetch.entry(w.key.clone()).or_default();
        rec.tries += 1;
        rec.last_try = Some(now.clone());
        let have = file
            .creators
            .values()
            .flat_map(|c| &c.posts)
            .any(|p| p.key() == w.key && p.in_library);
        if have {
            rec.done_at = Some(now.clone());
            rec.error = None;
            run.fetched += 1;
            let _ = writeln!(out, "  ✓ {}: {}", w.key, w.title);
        } else {
            rec.error = Some(format!("not in the library after the fetch (exit {code})"));
            run.failed += 1;
            let last = if rec.tries >= MAX_TRIES {
                ", given up"
            } else {
                ""
            };
            let _ = writeln!(
                out,
                "  ✗ {}: {} (try {}/{MAX_TRIES}{last})",
                w.key, w.title, rec.tries
            );
        }
    }
    run.code = codes
        .iter()
        .map(|(_, c)| *c)
        .find(|c| *c != 0 && *c != 2)
        .unwrap_or(0);
    if run.code == 0 {
        let retry = list.iter().any(|w| {
            state
                .fetch
                .get(&w.key)
                .is_some_and(|r| r.done_at.is_none() && r.tries < MAX_TRIES)
        });
        if retry || codes.iter().any(|(_, c)| *c == 2) {
            run.code = 2;
        }
    }
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::FetchRecord;
    use crate::store::Creator;

    fn post(kind: &str, id: &str, downloadable: bool, in_library: bool) -> Post {
        Post {
            kind: kind.into(),
            id: id.into(),
            title: format!("T{id}"),
            downloadable,
            in_library,
            ..Post::default()
        }
    }

    #[test]
    fn picks_wanted_fetchable_posts() {
        let mut file = CreatorsFile::empty();
        file.creators.insert(
            "a".into(),
            Creator {
                name: "A".into(),
                posts: vec![
                    post("eroscripts", "1", true, false),
                    post("eroscripts", "2", false, false),
                    post("eroscripts", "3", true, true),
                    post("faptap", "4", true, false),
                    post("faptap", "5", true, false),
                    post("eroscripts", "6", true, false),
                    post("eroscripts", "7", true, false),
                    post("eroscripts", "8", true, false),
                    post("eroscripts", "9", true, false),
                    Post {
                        hosts: vec!["slr".into()],
                        ..post("eroscripts", "10", true, false)
                    },
                ],
                ..Creator::default()
            },
        );
        let mut swipes = Swipes::default();
        for (k, c) in [
            ("eroscripts:1", "want"),
            ("eroscripts:2", "want"),
            ("eroscripts:3", "want"),
            ("faptap:4", "want"),
            ("faptap:5", "hide"),
            ("eroscripts:6", "want"),
            ("eroscripts:7", "want"),
            ("eroscripts:8", "want"),
            ("eroscripts:9", "want"),
            ("eroscripts:10", "want"),
            ("eroscripts:99", "want"),
        ] {
            swipes.posts.insert(k.into(), c.into());
        }
        let mut state = State::default();
        state.fetch.insert(
            "eroscripts:6".into(),
            FetchRecord {
                tries: MAX_TRIES,
                ..FetchRecord::default()
            },
        );
        state.fetch.insert(
            "eroscripts:7".into(),
            FetchRecord {
                tries: 1,
                done_at: Some("x".into()),
                ..FetchRecord::default()
            },
        );
        let now = parse_iso("2026-11-23T12:00:00Z").unwrap();
        for (k, at) in [
            ("eroscripts:8", "2026-11-23T11:00:00.000Z"),
            ("eroscripts:9", "2026-11-22T23:00:00.000Z"),
        ] {
            state.fetch.insert(
                k.into(),
                FetchRecord {
                    tries: 1,
                    last_try: Some(at.into()),
                    ..FetchRecord::default()
                },
            );
        }
        let keys: Vec<String> = wanted(&file, &swipes, &state, now)
            .into_iter()
            .map(|w| w.key)
            .collect();
        assert_eq!(keys, ["eroscripts:1", "eroscripts:9", "faptap:4"]);
        assert_eq!(
            commands("npm", "faptap", &["4".into(), "8".into()]).unwrap(),
            [
                "npm run -s faptap:scout -- --add 4,8".to_string(),
                "npm run -s faptap:scout -- --fetch --ids 4,8".to_string()
            ]
        );
        assert!(commands("npm", "other", &[]).is_none());
    }
}
