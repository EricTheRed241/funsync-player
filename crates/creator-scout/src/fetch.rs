//! `--fetch` (hourly, the `downloads-creators` job): the posts swiped `want` on the Creators
//! site go to the scouts that download them, run from the repo with `FUNSYNC_USER_DATA` set
//! to this run's userData:
//!
//! - EroScripts posts are **handed over**: `npm run eroscripts:scout -- --add <ids>` (the ids
//!   not on its waiting list, `<userData>/eroscripts-scout.json`, yet) puts them on the
//!   waiting list, and the hourly `eroscripts:scout --fetch --auto` (`downloads-eroscripts`),
//!   which takes the creators' wants along with the right swipes, downloads them; so only one
//!   process fetches EroScripts posts. A post on the list afterwards gets `handedAt` and isn't
//!   tried here again (no try counted); one the scout didn't list counts a try.
//! - FapTap videos: `npm run faptap:scout -- --add <ids>`, then `--fetch --ids <ids>`.
//!
//! A post counts as fetched (`doneAt`) once the library has it, a handed-over one too; one
//! that doesn't arrive is tried again once [`RETRY_HOURS`] have passed, [`MAX_TRIES`] times
//! in all. SLR-hosted posts are left to the weekly `slr:scout --fetch`.

use crate::instagram::parse_iso;
use crate::library::LibraryIds;
use crate::state::{MAX_TRIES, RETRY_HOURS, State, Swipes};
use crate::store::{CreatorsFile, Post};
use serde_json::Value;
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// eroscripts:scout's waiting list in userData.
pub const WAITING_FILE: &str = "eroscripts-scout.json";

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
/// not in the library, not fetched or handed over before, tried fewer than
/// [`MAX_TRIES`] times and not within [`RETRY_HOURS`] of `now` (seconds since
/// the epoch; an unreadable last try counts as due). In swipe-key order.
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
                    && r.handed_at.is_none()
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

/// The topic ids on eroscripts:scout's waiting list (`candidates[].id` of
/// [`WAITING_FILE`]); a missing or unreadable file lists none.
pub fn waiting_ids(user_data: &Path) -> BTreeSet<String> {
    let Ok(raw) = std::fs::read(user_data.join(WAITING_FILE)) else {
        return BTreeSet::new();
    };
    let v: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    v["candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| match &c["id"] {
            Value::Number(n) => Some(n.to_string()),
            Value::String(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

/// The npm commands for a kind's ids: EroScripts only `--add` (the hourly
/// `eroscripts:scout --fetch --auto` downloads them), FapTap `--add`, then
/// `--fetch --ids`.
pub fn commands(npm: &str, kind: &str, ids: &[String]) -> Option<Vec<String>> {
    let ids = ids.join(",");
    match kind {
        "eroscripts" => Some(vec![format!(
            "{npm} run -s eroscripts:scout -- --add {ids}"
        )]),
        "faptap" => Some(vec![
            format!("{npm} run -s faptap:scout -- --add {ids}"),
            format!("{npm} run -s faptap:scout -- --fetch --ids {ids}"),
        ]),
        _ => None,
    }
}

/// Run a command line with sh in `repo`, `FUNSYNC_USER_DATA` set to
/// `user_data`; its output goes to stderr.
fn sh(line: &str, repo: &Path, user_data: &Path) -> Result<i32, String> {
    use std::os::fd::AsFd;
    let to_stderr = std::io::stderr()
        .as_fd()
        .try_clone_to_owned()
        .map_err(|e| format!("can't run {line}: {e}"))?;
    let status = Command::new("/bin/sh")
        .arg("-c")
        .arg(line)
        .current_dir(repo)
        .env("FUNSYNC_USER_DATA", user_data)
        .stdin(Stdio::null())
        .stdout(Stdio::from(to_stderr))
        .status()
        .map_err(|e| format!("can't run {line}: {e}"))?;
    Ok(status.code().unwrap_or(1))
}

pub struct FetchRun {
    pub code: i32,
    pub fetched: usize,
    /// EroScripts posts put on the waiting list this run.
    pub handed: usize,
    pub failed: usize,
}

/// Fetch the wanted posts (EroScripts ones handed over); `rescan` reads the
/// library again afterwards.
#[allow(clippy::too_many_arguments)]
pub fn fetch(
    file: &mut CreatorsFile,
    swipes: &Swipes,
    state: &mut State,
    user_data: &Path,
    npm: &str,
    repo: &Path,
    dry_run: bool,
    rescan: &dyn Fn() -> LibraryIds,
    out: &mut dyn Write,
) -> Result<FetchRun, String> {
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let now = video_tags::timefmt::now_iso();
    if !dry_run {
        // Handed-over posts eroscripts:scout has downloaded since.
        for p in file
            .creators
            .values()
            .flat_map(|c| &c.posts)
            .filter(|p| p.in_library)
        {
            if let Some(rec) = state
                .fetch
                .get_mut(&p.key())
                .filter(|r| r.handed_at.is_some() && r.done_at.is_none())
            {
                rec.done_at = Some(now.clone());
            }
        }
    }
    let list = wanted(file, swipes, state, now_unix);
    let mut run = FetchRun {
        code: 0,
        fetched: 0,
        handed: 0,
        failed: 0,
    };
    if list.is_empty() {
        let _ = writeln!(out, "Nothing to fetch.");
        return Ok(run);
    }
    let waiting = waiting_ids(user_data);
    let mut codes = Vec::new();
    for kind in ["eroscripts", "faptap"] {
        let posts: Vec<&Wanted> = list.iter().filter(|w| w.kind == kind).collect();
        if posts.is_empty() {
            continue;
        }
        // EroScripts posts already on the waiting list need no --add.
        let ids: Vec<String> = posts
            .iter()
            .filter(|w| kind != "eroscripts" || !waiting.contains(&w.id))
            .map(|w| w.id.clone())
            .collect();
        let cmds = match commands(npm, kind, &ids) {
            Some(cmds) if !ids.is_empty() => cmds,
            _ => Vec::new(),
        };
        if dry_run {
            let verb = if kind == "eroscripts" {
                "hand over"
            } else {
                "fetch"
            };
            for w in &posts {
                let _ = writeln!(out, "  would {verb} {}: {}", w.key, w.title);
            }
            for c in &cmds {
                let _ = writeln!(out, "  would run: {c}");
            }
            continue;
        }
        if cmds.is_empty() {
            continue;
        }
        let _ = if kind == "eroscripts" {
            writeln!(
                out,
                "Handing {} EroScripts post(s) to eroscripts:scout: {}",
                ids.len(),
                ids.join(",")
            )
        } else {
            writeln!(
                out,
                "Fetching {} {kind} post(s): {}",
                ids.len(),
                ids.join(",")
            )
        };
        let mut code = 0;
        for c in &cmds {
            code = sh(c, repo, user_data)?;
            if code != 0 {
                break;
            }
        }
        codes.push((kind, code));
    }
    if dry_run {
        return Ok(run);
    }
    let lib = rescan();
    lib.mark(file);
    let listed = waiting_ids(user_data);
    for w in &list {
        let code = codes
            .iter()
            .find(|(k, _)| *k == w.kind)
            .map_or(0, |(_, c)| *c);
        let rec = state.fetch.entry(w.key.clone()).or_default();
        let have = file
            .creators
            .values()
            .flat_map(|c| &c.posts)
            .any(|p| p.key() == w.key && p.in_library);
        if have {
            rec.tries += 1;
            rec.last_try = Some(now.clone());
            rec.done_at = Some(now.clone());
            rec.error = None;
            run.fetched += 1;
            let _ = writeln!(out, "  ✓ {}: {}", w.key, w.title);
        } else if w.kind == "eroscripts" && listed.contains(&w.id) {
            rec.handed_at = Some(now.clone());
            rec.error = None;
            run.handed += 1;
            let _ = writeln!(
                out,
                "  → {}: {} (on the EroScripts waiting list; eroscripts:scout --fetch --auto downloads it)",
                w.key, w.title
            );
        } else {
            rec.tries += 1;
            rec.last_try = Some(now.clone());
            rec.error = Some(if w.kind == "eroscripts" {
                format!("eroscripts:scout --add didn't list it (exit {code})")
            } else {
                format!("not in the library after the fetch (exit {code})")
            });
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
            state.fetch.get(&w.key).is_some_and(|r| {
                r.done_at.is_none() && r.handed_at.is_none() && r.tries < MAX_TRIES
            })
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
    use std::fs;

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
                    post("eroscripts", "11", true, false),
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
            ("eroscripts:11", "want"),
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
        // Handed over: eroscripts:scout downloads it now.
        state.fetch.insert(
            "eroscripts:11".into(),
            FetchRecord {
                handed_at: Some("x".into()),
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
        assert_eq!(
            commands("npm", "eroscripts", &["1".into(), "9".into()]).unwrap(),
            ["npm run -s eroscripts:scout -- --add 1,9".to_string()]
        );
        assert!(commands("npm", "other", &[]).is_none());
    }

    struct Tmp(PathBuf);

    impl Tmp {
        fn new(name: &str) -> Tmp {
            let dir = std::env::temp_dir()
                .join(format!("creator-scout-fetch-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Tmp(dir)
        }

        /// A fake npm: logs its arguments; on eroscripts:scout --add, runs `on_add`.
        fn npm(&self, on_add: &str) -> String {
            let script = self.0.join("npm.sh");
            fs::write(
                &script,
                format!(
                    "echo \"$*\" >> '{}'\nif [ \"$3\" = eroscripts:scout ] && [ \"$5\" = --add ]; then\n{on_add}\nfi\n",
                    self.0.join("npm.log").display()
                ),
            )
            .unwrap();
            format!("sh '{}'", script.display())
        }

        fn log(&self) -> Vec<String> {
            fs::read_to_string(self.0.join("npm.log"))
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect()
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Two EroScripts posts swiped want.
    fn setup() -> (CreatorsFile, Swipes) {
        let mut file = CreatorsFile::empty();
        file.creators.insert(
            "a".into(),
            Creator {
                name: "A".into(),
                posts: vec![
                    post("eroscripts", "400000", true, false),
                    post("eroscripts", "400001", true, false),
                ],
                ..Creator::default()
            },
        );
        let mut swipes = Swipes::default();
        for k in ["eroscripts:400000", "eroscripts:400001"] {
            swipes.posts.insert(k.into(), "want".into());
        }
        (file, swipes)
    }

    fn fetch_with(
        t: &Tmp,
        npm: &str,
        file: &mut CreatorsFile,
        swipes: &Swipes,
        state: &mut State,
        lib: LibraryIds,
    ) -> (FetchRun, String) {
        let mut out = Vec::new();
        let run = fetch(
            file,
            swipes,
            state,
            &t.0,
            npm,
            &t.0,
            false,
            &|| lib.clone(),
            &mut out,
        )
        .unwrap();
        (run, String::from_utf8(out).unwrap())
    }

    #[test]
    fn eroscripts_posts_on_the_waiting_list_are_handed_over() {
        let t = Tmp::new("handed");
        let (mut file, swipes) = setup();
        let mut state = State::default();
        // 400001 is waiting already; --add lists 400000 (into the userData it was given).
        fs::write(
            t.0.join(WAITING_FILE),
            r#"{"candidates": [{"id": 400001}]}"#,
        )
        .unwrap();
        let npm = t.npm(
            "echo '{\"candidates\": [{\"id\": 400001}, {\"id\": \"400000\"}]}' > \"$FUNSYNC_USER_DATA/eroscripts-scout.json\"",
        );
        let (run, out) = fetch_with(
            &t,
            &npm,
            &mut file,
            &swipes,
            &mut state,
            LibraryIds::default(),
        );
        assert_eq!(
            (run.code, run.fetched, run.handed, run.failed),
            (0, 0, 2, 0),
            "{out}"
        );
        assert_eq!(t.log(), ["run -s eroscripts:scout -- --add 400000"]);
        for key in ["eroscripts:400000", "eroscripts:400001"] {
            let r = &state.fetch[key];
            assert!(r.handed_at.is_some(), "{key}");
            assert_eq!(
                (r.tries, r.last_try.as_deref(), r.done_at.as_deref()),
                (0, None, None)
            );
        }
        assert!(out.contains("  → eroscripts:400000: T400000 (on the EroScripts waiting list"));

        // Handed over: not tried again; done once the library has it.
        let now = parse_iso("2026-11-24T12:00:00Z").unwrap();
        assert!(wanted(&file, &swipes, &state, now).is_empty());
        file.creators.get_mut("a").unwrap().posts[0].in_library = true;
        let (run, out) = fetch_with(
            &t,
            &npm,
            &mut file,
            &swipes,
            &mut state,
            LibraryIds::default(),
        );
        assert_eq!(run.code, 0);
        assert!(out.contains("Nothing to fetch."), "{out}");
        assert!(state.fetch["eroscripts:400000"].done_at.is_some());
        assert!(state.fetch["eroscripts:400001"].done_at.is_none());
        assert_eq!(t.log().len(), 1);
    }

    #[test]
    fn an_eroscripts_post_the_scout_does_not_list_counts_a_try() {
        let t = Tmp::new("unlisted");
        let (mut file, swipes) = setup();
        let mut state = State::default();
        let npm = t.npm("exit 0");
        let (run, out) = fetch_with(
            &t,
            &npm,
            &mut file,
            &swipes,
            &mut state,
            LibraryIds::default(),
        );
        assert_eq!(
            (run.code, run.fetched, run.handed, run.failed),
            (2, 0, 0, 2),
            "{out}"
        );
        assert_eq!(t.log(), ["run -s eroscripts:scout -- --add 400000,400001"]);
        let r = &state.fetch["eroscripts:400000"];
        assert_eq!(r.tries, 1);
        assert!(r.last_try.is_some() && r.handed_at.is_none() && r.done_at.is_none());
        assert_eq!(
            r.error.as_deref(),
            Some("eroscripts:scout --add didn't list it (exit 0)")
        );
        assert!(
            out.contains("  ✗ eroscripts:400000: T400000 (try 1/5)"),
            "{out}"
        );
    }
}
