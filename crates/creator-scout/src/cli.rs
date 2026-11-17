//! Flags and `--help`.

use std::ffi::OsString;
use std::path::PathBuf;

pub const DEFAULT_PICTURES: usize = 40;

pub const USAGE: &str = "\
creator-scout: find independent creators (OnlyFans / Fansly / camgirls /
cosplayers / influencers with an Instagram or Twitch account) behind new
EroScripts script posts and FapTap videos, collect their pictures and posts
for the Creators site (npm run creators:site), and fetch the posts swiped in.

Usage:
  npm run creators:scout -- --import-seed <file>   [--user-data <dir>]
  npm run creators:scout -- --discover             [options]
  npm run creators:scout -- --pictures [N]         [--user-data <dir>]
  npm run creators:scout -- --login                [--user-data <dir>]
  npm run creators:scout -- --status               [--user-data <dir>]
  npm run creators:scout -- --fetch [--dry-run]    [options]
  npm run creators:scout -- --daily                [options]

Modes:
  --import-seed <file>  Merge a research snapshot (content-creators.json) into
                        creators.json; existing creators and posts are kept.
                        No network.
  --discover            New EroScripts posts tagged real + non-vr (Free and Paid
                        Scripts) and new FapTap videos since the last run; a judge
                        (an omp run) names each one's on-screen creator and, for a
                        new creator, Instagram / Twitch handle candidates found by
                        web search. A new creator is kept only with an Instagram or
                        Twitch account that exists and matches the name. Then her
                        posts: the EroScripts post's video hosts, tags, likes and
                        first image; FapTap videos found by her name. Posts are
                        scored like the EroScripts deck (candidateScore); a
                        creator's score is her posts' average. First run: the 5
                        newest pages of each feed.
  --pictures [N]        Instagram pictures (the 9 newest of the latest 12 posts)
                        for up to N creators (default 40), stalest first, each at
                        most every 3 days, creators swiped no skipped; 8-15 s
                        between creators. Needs --login for the posts; without a
                        session only the profile photo is saved (exit 3).
  --login               Open Chrome (profile <userData>/instagram-chrome) on
                        Instagram's login page; sign in yourself. The session is
                        saved to <userData>/instagram-session.json (0600) once it
                        appears (10 minutes at most).
  --status              Creators, posts, the Instagram session and its age, the
                        cursors and the fetch queue.
  --fetch               Fetch the posts swiped \"want\" on the Creators site that
                        are downloadable and not in the library: EroScripts posts
                        through npm run eroscripts:scout (--add, then --fetch
                        --ids), FapTap videos through npm run faptap:scout. A post
                        that doesn't reach the library is tried again next run,
                        5 times at most.
  --dry-run             With --fetch: print what would be fetched, change nothing
  --daily               --discover, --pictures 40 and --fetch; a failing step
                        doesn't stop the next
  -h, --help            This text

Options:
  --user-data <dir>     FunSync's userData folder
                        (default $FUNSYNC_USER_DATA, else ~/Library/Application Support/funsync-player)
  --max-cost <usd>      Spending cap of the default judge (default 5)
  --judge-cmd <cmd>     Another judge command (run by sh in <userData>/creator-scout-work
                        with the prompt as its last argument); it must write
                        results.json for titles.json
  --repo <dir>          The repo the npm scripts run in (default: this checkout)
  --npm <cmd>           The npm command for --fetch (default npm)

Files in <userData>: creators.json (read by the Creators site), creator-pictures/,
creator-scout-state.json, instagram-session.json; creator-swipes.json is read
(the site writes it). config.json is only read (the EroScripts session cookie,
the library sources, tag scores). Lock: creator-scout.lock.

Exit codes: 0 ok, 1 error, 2 rate limited or work left (more pages, creators
or fetches than this run took), 3 not logged in (EroScripts session missing,
no Instagram session, or Instagram asked to log in).
";

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    ImportSeed(PathBuf),
    Discover,
    Pictures(usize),
    Login,
    Status,
    Fetch,
    Daily,
}

#[derive(Debug, Default, PartialEq)]
pub struct Options {
    pub help: bool,
    pub mode: Option<Mode>,
    pub dry_run: bool,
    pub user_data: Option<PathBuf>,
    pub max_cost: Option<f64>,
    pub judge_cmd: Option<String>,
    pub repo: Option<PathBuf>,
    pub npm: Option<String>,
}

pub fn parse(args: &[OsString]) -> Result<Options, String> {
    let mut o = Options::default();
    let args: Vec<&str> = args
        .iter()
        .map(|a| {
            a.to_str()
                .ok_or_else(|| format!("argument is not UTF-8: {}", a.to_string_lossy()))
        })
        .collect::<Result<_, _>>()?;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i];
        i += 1;
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (arg, None),
        };
        let takes_value = matches!(
            flag,
            "--import-seed"
                | "--user-data"
                | "--max-cost"
                | "--judge-cmd"
                | "--repo"
                | "--npm"
                | "--pictures"
        );
        if inline.is_some() && !takes_value {
            return Err(format!("{flag} takes no value"));
        }
        let mut value = || -> Result<String, String> {
            if let Some(v) = inline.clone() {
                return Ok(v);
            }
            let v = args
                .get(i)
                .ok_or_else(|| format!("{flag} needs a value"))?
                .to_string();
            i += 1;
            Ok(v)
        };
        let mut set_mode = |m: Mode| -> Result<(), String> {
            if o.mode.is_some() {
                return Err(
                    "give one of --import-seed, --discover, --pictures, --login, --status, --fetch, --daily"
                        .into(),
                );
            }
            o.mode = Some(m);
            Ok(())
        };
        match flag {
            "-h" | "--help" => o.help = true,
            "--import-seed" => set_mode(Mode::ImportSeed(PathBuf::from(value()?)))?,
            "--discover" => set_mode(Mode::Discover)?,
            "--pictures" => {
                // The count is optional: a following number is it.
                let n = match inline.as_deref() {
                    Some(v) => Some(v.to_string()),
                    None => args.get(i).filter(|v| v.parse::<usize>().is_ok()).map(|v| {
                        i += 1;
                        v.to_string()
                    }),
                };
                let n =
                    match n {
                        Some(v) => v.parse::<usize>().ok().filter(|&n| n > 0).ok_or_else(|| {
                            format!("--pictures {v}: give a whole number above 0")
                        })?,
                        None => DEFAULT_PICTURES,
                    };
                set_mode(Mode::Pictures(n))?
            }
            "--login" => set_mode(Mode::Login)?,
            "--status" => set_mode(Mode::Status)?,
            "--fetch" => set_mode(Mode::Fetch)?,
            "--daily" => set_mode(Mode::Daily)?,
            "--dry-run" => o.dry_run = true,
            "--user-data" => o.user_data = Some(PathBuf::from(value()?)),
            "--max-cost" => {
                let v = value()?;
                o.max_cost = Some(
                    v.parse::<f64>()
                        .ok()
                        .filter(|c| c.is_finite() && *c > 0.0)
                        .ok_or_else(|| format!("--max-cost {v}: give an amount above 0"))?,
                );
            }
            "--judge-cmd" => {
                let v = value()?;
                if v.trim().is_empty() {
                    return Err("--judge-cmd is empty".into());
                }
                o.judge_cmd = Some(v);
            }
            "--repo" => o.repo = Some(PathBuf::from(value()?)),
            "--npm" => {
                let v = value()?;
                if v.trim().is_empty() {
                    return Err("--npm is empty".into());
                }
                o.npm = Some(v);
            }
            _ => return Err(format!("unknown argument {arg} (see --help)")),
        }
    }
    if o.help {
        return Ok(o);
    }
    let Some(mode) = &o.mode else {
        return Err(
            "nothing to do: give --import-seed, --discover, --pictures, --login, --status, --fetch or --daily (see --help)"
                .into(),
        );
    };
    if o.dry_run && *mode != Mode::Fetch {
        return Err("--dry-run goes with --fetch".into());
    }
    let judging = matches!(mode, Mode::Discover | Mode::Daily);
    if !judging && (o.judge_cmd.is_some() || o.max_cost.is_some()) {
        return Err("--judge-cmd and --max-cost go with --discover and --daily".into());
    }
    if o.judge_cmd.is_some() && o.max_cost.is_some() {
        return Err("--max-cost applies to the default judge only".into());
    }
    let fetching = matches!(mode, Mode::Fetch | Mode::Daily);
    if !fetching && (o.repo.is_some() || o.npm.is_some()) {
        return Err("--repo and --npm go with --fetch and --daily".into());
    }
    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Options, String> {
        parse(&args.iter().map(OsString::from).collect::<Vec<_>>())
    }

    #[test]
    fn modes_and_values() {
        assert_eq!(p(&["--discover"]).unwrap().mode, Some(Mode::Discover));
        assert_eq!(
            p(&["--pictures"]).unwrap().mode,
            Some(Mode::Pictures(DEFAULT_PICTURES))
        );
        assert_eq!(
            p(&["--pictures", "5", "--user-data", "/u"]).unwrap().mode,
            Some(Mode::Pictures(5))
        );
        assert_eq!(p(&["--pictures=7"]).unwrap().mode, Some(Mode::Pictures(7)));
        assert!(p(&["--pictures", "0"]).is_err());
        let o = p(&["--import-seed", "/s.json", "--user-data=/u"]).unwrap();
        assert_eq!(o.mode, Some(Mode::ImportSeed(PathBuf::from("/s.json"))));
        assert_eq!(o.user_data, Some(PathBuf::from("/u")));
        let o = p(&["--fetch", "--dry-run", "--npm", "fake-npm", "--repo", "/r"]).unwrap();
        assert!(o.dry_run);
        assert_eq!(o.npm.as_deref(), Some("fake-npm"));
        assert!(p(&["--help"]).unwrap().help);
    }

    #[test]
    fn rejects_bad_combinations() {
        assert!(p(&[]).is_err());
        assert!(p(&["--discover", "--fetch"]).is_err());
        assert!(p(&["--discover", "--dry-run"]).is_err());
        assert!(p(&["--status", "--judge-cmd", "x"]).is_err());
        assert!(p(&["--discover", "--judge-cmd", "x", "--max-cost", "2"]).is_err());
        assert!(p(&["--pictures", "--npm", "x"]).is_err());
        assert!(p(&["--status=1"]).is_err());
        assert!(p(&["--bogus"]).is_err());
        assert!(p(&["--import-seed"]).is_err());
        assert!(p(&["--daily", "--max-cost", "3", "--npm", "n"]).is_ok());
    }
}
