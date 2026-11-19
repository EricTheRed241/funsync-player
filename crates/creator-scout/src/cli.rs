//! Flags and `--help`.

use std::ffi::OsString;
use std::path::PathBuf;

pub const DEFAULT_PICTURES: usize = 40;
pub const DEFAULT_PROFILES: usize = 30;
pub const DEFAULT_FOLLOWS: usize = 10;
pub const DEFAULT_POST_IMAGES: usize = 300;

pub const USAGE: &str = "\
creator-scout: find independent creators (OnlyFans / Fansly / camgirls /
cosplayers / influencers) behind new EroScripts script posts and FapTap videos,
and the people you like (favourite artists, the pornstar list, creators swiped
yes); confirm their profiles, collect their pictures and posts for the Creators
site (npm run creators:site), follow them on Instagram, and fetch the posts
swiped in.

Usage:
  npm run creators:scout -- --import-seed <file>   [--user-data <dir>]
  npm run creators:scout -- --discover             [options]
  npm run creators:scout -- --profiles [N]         [options]
  npm run creators:scout -- --pictures [N]         [--user-data <dir>]
  npm run creators:scout -- --post-images [N]      [--user-data <dir>]
  npm run creators:scout -- --follow [N]           [--user-data <dir>]
  npm run creators:scout -- --login [--credentials <file>] [--user-data <dir>]
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
                        new creator, finds her Instagram / Twitch handles, profile
                        pages and pictures by web search. A new creator is kept
                        only when one of them checks out live as hers (below).
                        Then her posts: the EroScripts post's video hosts, tags,
                        likes, first image and linked video pages; FapTap videos
                        found by her name; and her pictures. Posts are scored
                        like the EroScripts deck (candidateScore); a creator's
                        score is her posts' average. First run: the 5 newest
                        pages of each feed.
  --profiles [N]        The people you like become creators (the creators swiped
                        yes on the Creators site, your favourite artists:
                        content-preferences.json favourites with the app's
                        changes, and the pornstar list), and creators with fewer
                        than 3 pictures get some. Each creator's sources (swiped /
                        favourite / pornstar) are set from them. Up to N of them
                        (default 30; a creator whose Instagram pictures were never
                        fetched waits for --pictures) go to the judge in one run,
                        which finds her Instagram / Twitch handles, profile pages
                        (OnlyFans, Fansly, Pornhub, X, Reddit, ManyVids, a link
                        page, IAFD / Babepedia / Boobpedia / FreeOnes) and
                        pictures. A profile is kept when the page answers and its
                        own name or handle matches hers; pictures come from those
                        pages, from the judge (only on a page that passed) and
                        from the video pages her posts link, 9 at most besides
                        Instagram's, none under 300 px or wider than 3:1. A new
                        creator also gets her EroScripts script posts (forum
                        search, her name in the title) and FapTap videos, 30 at
                        most. Nothing confirmed, or still under 3 pictures: looked
                        up again after 30 days.
  --pictures [N]        Instagram pictures (the 9 newest of the latest 12 posts,
                        from her profile page as Chrome loads it) for up to N
                        creators (default 40), those with a source first (swiped,
                        favourite, pornstar list), stalest first, each at most
                        every 3 days, creators swiped no skipped; 8-15 s between
                        creators. Needs --login for the posts; without a session
                        only the profile photo is saved (exit 3).
  --post-images [N]     Fill in the pictures of up to N posts (default 300) filed
                        without one, so their cards show the scene: a FapTap
                        video's thumbnail (its API), an EroScripts post's first
                        image (the topic; needs the EroScripts session, without
                        it those posts wait: exit 3). Only downloadable posts not
                        in the library and not swiped, of creators not swiped no.
                        A post without a picture, or whose picture won't
                        download, is tried again after 7 days.
  --follow [N]          Follow on Instagram (the --login account) up to N creators
                        (default 10) with a confirmed Instagram account who are a
                        favourite or swiped yes: a Chrome (headless, the --login
                        profile and session; a visible one when headless meets a
                        login wall) opens her profile and clicks Follow; 20-40 s
                        between them; never unfollows. A login wall stops it
                        (exit 3), \"Try Again Later\" too (exit 2, next day).
  --login               Open Chrome (profile <userData>/instagram-chrome) on
                        Instagram's login page and sign in with the credentials
                        file (below); without one, sign in yourself. A check
                        Instagram asks for (a code, \"save login info\") is done in
                        the window. The session is saved to
                        <userData>/instagram-session.json (0600) once it appears
                        (10 minutes at most).
  --status              Creators, posts, the Instagram session and its age, the
                        cursors, the profiles step, the follows, the posts waiting
                        for their picture and the fetch queue.
  --fetch               Fetch the posts swiped \"want\" on the Creators site that
                        are downloadable and not in the library: EroScripts posts
                        through npm run eroscripts:scout (--add, then --fetch
                        --ids), FapTap videos through npm run faptap:scout. A post
                        that doesn't reach the library is tried again next run,
                        5 times at most.
  --dry-run             With --fetch: print what would be fetched, change nothing
  --daily               --discover, --profiles 30, --pictures 40, --follow 10 and
                        --fetch; a failing step doesn't stop the next
  -h, --help            This text

Options:
  --user-data <dir>     FunSync's userData folder
                        (default $FUNSYNC_USER_DATA, else ~/Library/Application Support/funsync-player)
  --max-cost <usd>      Spending cap of the default judge (default 5; 10 for
                        --profiles)
  --judge-cmd <cmd>     Another judge command (run by sh in <userData>/creator-scout-work
                        with the prompt as its last argument); it must write
                        results.json for titles.json (--discover) and
                        people-results-<n>.json for each people-<n>.json (--profiles)
  --repo <dir>          The repo the npm scripts run in (default: this checkout)
  --credentials <file>  The Instagram account for --login: \"username: …\" and
                        \"password: …\" lines (default $FUNSYNC_INSTAGRAM_CREDENTIALS,
                        else instagram_creds.txt at the repo root; never printed)
  --npm <cmd>           The npm command for --fetch (default npm)

Files in <userData>: creators.json (read by the Creators site), creator-pictures/,
creator-scout-state.json, instagram-session.json; creator-swipes.json is read
(the site writes it). config.json is only read (the EroScripts session cookie,
the library sources, tag scores, favourite artists). Lock: creator-scout.lock.

Exit codes: 0 ok, 1 error, 2 rate limited or work left (more pages, creators
or fetches than this run took), 3 not logged in (EroScripts session missing,
no Instagram session, or Instagram asked to log in or to confirm the account).
";

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    ImportSeed(PathBuf),
    Discover,
    Pictures(usize),
    PostImages(usize),
    Profiles(usize),
    Follow(usize),
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
    pub credentials: Option<PathBuf>,
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
                | "--post-images"
                | "--profiles"
                | "--follow"
                | "--credentials"
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
                    "give one of --import-seed, --discover, --profiles, --pictures, --post-images, --follow, --login, --status, --fetch, --daily"
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
            "--pictures" | "--post-images" | "--profiles" | "--follow" => {
                // The count is optional: a following number is it.
                let n = match inline.as_deref() {
                    Some(v) => Some(v.to_string()),
                    None => args.get(i).filter(|v| v.parse::<usize>().is_ok()).map(|v| {
                        i += 1;
                        v.to_string()
                    }),
                };
                let n = n
                    .map(|v| {
                        v.parse::<usize>()
                            .ok()
                            .filter(|&n| n > 0)
                            .ok_or_else(|| format!("{flag} {v}: give a whole number above 0"))
                    })
                    .transpose()?;
                set_mode(match flag {
                    "--pictures" => Mode::Pictures(n.unwrap_or(DEFAULT_PICTURES)),
                    "--post-images" => Mode::PostImages(n.unwrap_or(DEFAULT_POST_IMAGES)),
                    "--profiles" => Mode::Profiles(n.unwrap_or(DEFAULT_PROFILES)),
                    _ => Mode::Follow(n.unwrap_or(DEFAULT_FOLLOWS)),
                })?
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
            "--credentials" => o.credentials = Some(PathBuf::from(value()?)),
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
            "nothing to do: give --import-seed, --discover, --profiles, --pictures, --post-images, --follow, --login, --status, --fetch or --daily (see --help)"
                .into(),
        );
    };
    if o.dry_run && *mode != Mode::Fetch {
        return Err("--dry-run goes with --fetch".into());
    }
    let judging = matches!(mode, Mode::Discover | Mode::Profiles(_) | Mode::Daily);
    if !judging && (o.judge_cmd.is_some() || o.max_cost.is_some()) {
        return Err("--judge-cmd and --max-cost go with --discover, --profiles and --daily".into());
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
        assert_eq!(
            p(&["--profiles"]).unwrap().mode,
            Some(Mode::Profiles(DEFAULT_PROFILES))
        );
        assert_eq!(
            p(&["--profiles", "3", "--judge-cmd", "j"]).unwrap().mode,
            Some(Mode::Profiles(3))
        );
        assert_eq!(
            p(&["--follow"]).unwrap().mode,
            Some(Mode::Follow(DEFAULT_FOLLOWS))
        );
        assert_eq!(p(&["--follow=2"]).unwrap().mode, Some(Mode::Follow(2)));
        assert_eq!(
            p(&["--post-images"]).unwrap().mode,
            Some(Mode::PostImages(DEFAULT_POST_IMAGES))
        );
        assert_eq!(
            p(&["--post-images", "5"]).unwrap().mode,
            Some(Mode::PostImages(5))
        );
        assert!(p(&["--post-images", "--npm", "x"]).is_err());
        assert!(p(&["--profiles=0"]).is_err());
        assert!(p(&["--profiles", "--pictures"]).is_err());
        assert!(p(&["--follow", "--judge-cmd", "j"]).is_err());
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
