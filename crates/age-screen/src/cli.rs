//! Flags and `--help`.

use std::ffi::OsString;
use std::path::PathBuf;

pub const USAGE: &str = "\
age-screen: screen the library for videos that may depict minors

Usage:
  npm run age:screen -- --run       [--limit N] [options]
  npm run age:screen -- --weekly    [options]   (--run when the last complete run is old enough)
  npm run age:screen -- --dry-run   [--limit N] [options]
  npm run age:screen -- --list      [--user-data <dir>]

Needs FunSync running (its backend serves the video list and applies the
ratings and playlist adds). Each video is screened once: again only when its
file size changes. For each new video, 12 frames spread over it (the left eye
of VR) make a contact sheet; a judge (an omp run with one subagent per batch
of 15 sheets) gives each video p_minor, the probability that it depicts
someone under 18, judged from the sheet and the video's path in the library.
Thresholds (ageScreen in renderer/js/content-preferences.json):
  p_minor >= purgeAt (0.8)   rated 1 star, so the app deletes it from disk;
                             a favourite goes to the review playlist instead
  p_minor >= reviewAt (0.5)  added to the review playlist (\"Underage review\")
  below                      clear
Videos rated at or below ratings.deleteAtOrBelow (2), AppleDouble '._'
files and files that aren't on disk are skipped.

Modes:
  --run                Screen every unscreened video, apply, record them in
                       <userData>/age-screen.json
  --weekly             --run when the last complete run is ageScreen.intervalDays
                       (7) or more ago, else print \"not due\" and exit 0
  --dry-run            Make the sheets and judge them, print what would be done;
                       no ratings, no playlist adds, no state written (the sheets
                       stay in <userData>/age-screen-work for a look)
  --list               Print the screened videos that weren't clear
  -h, --help           This text

Options:
  --user-data <dir>    FunSync's userData folder
                       (default $FUNSYNC_USER_DATA, else ~/Library/Application Support/funsync-player)
  --backend <url>      The app's backend (default $FUNSYNC_BACKEND_URL, else
                       http://127.0.0.1:5123)
  --limit N            Screen at most N videos this run (smoke tests); the rest
                       wait, and a --weekly run stays due
  --max-cost <usd>     Spending cap of the default judge (default 30)
  --judge-cmd <cmd>    Another judge command (run by sh, in the work folder, with
                       the prompt as its last argument); it must write
                       results/NN.json for every batches/NN.json

A judge failure or a missing or invalid results file leaves those videos
unscreened: the next run tries them again. Applying a verdict waits up to 3
minutes for an app that is restarting before that video counts as unscreened.

Exit codes: 0 ok (or not due), 1 error, 2 backend not reachable or videos left
unscreened.
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Run,
    Weekly,
    DryRun,
    List,
}

#[derive(Debug, PartialEq)]
pub struct Options {
    pub help: bool,
    pub mode: Option<Mode>,
    pub user_data: Option<PathBuf>,
    pub backend: Option<String>,
    pub limit: Option<usize>,
    pub max_cost: Option<f64>,
    pub judge_cmd: Option<String>,
}

pub fn parse(args: &[OsString]) -> Result<Options, String> {
    let mut o = Options {
        help: false,
        mode: None,
        user_data: None,
        backend: None,
        limit: None,
        max_cost: None,
        judge_cmd: None,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let arg = arg
            .to_str()
            .ok_or_else(|| format!("argument is not UTF-8: {}", arg.to_string_lossy()))?;
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v)),
            _ => (arg, None),
        };
        let takes_value = matches!(
            flag,
            "--user-data" | "--backend" | "--limit" | "--max-cost" | "--judge-cmd"
        );
        if inline.is_some() && !takes_value {
            return Err(format!("{flag} takes no value"));
        }
        let mut value = || -> Result<String, String> {
            match inline {
                Some(v) => Ok(v.to_string()),
                None => it
                    .next()
                    .and_then(|v| v.to_str())
                    .map(str::to_string)
                    .ok_or_else(|| format!("{flag} needs a value")),
            }
        };
        let mut set_mode = |m: Mode| -> Result<(), String> {
            if o.mode.is_some_and(|old| old != m) {
                return Err("give one of --run, --weekly, --dry-run, --list".into());
            }
            o.mode = Some(m);
            Ok(())
        };
        match flag {
            "-h" | "--help" => o.help = true,
            "--run" => set_mode(Mode::Run)?,
            "--weekly" => set_mode(Mode::Weekly)?,
            "--dry-run" => set_mode(Mode::DryRun)?,
            "--list" => set_mode(Mode::List)?,
            "--user-data" => o.user_data = Some(PathBuf::from(value()?)),
            "--backend" => o.backend = Some(value()?),
            "--limit" => {
                let v = value()?;
                o.limit = Some(
                    v.parse::<usize>()
                        .ok()
                        .filter(|&n| n > 0)
                        .ok_or_else(|| format!("--limit {v}: give a whole number above 0"))?,
                );
            }
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
            _ => return Err(format!("unknown argument {arg} (see --help)")),
        }
    }
    if o.help {
        return Ok(o);
    }
    let Some(mode) = o.mode else {
        return Err("nothing to do: give --run, --weekly, --dry-run or --list (see --help)".into());
    };
    if mode == Mode::List
        && (o.backend.is_some()
            || o.limit.is_some()
            || o.max_cost.is_some()
            || o.judge_cmd.is_some())
    {
        return Err("--list prints the state file; use it alone (with --user-data)".into());
    }
    if mode == Mode::Weekly && o.limit.is_some() {
        return Err("--limit is for --run and --dry-run".into());
    }
    if o.judge_cmd.is_some() && o.max_cost.is_some() {
        return Err("--max-cost applies to the default judge only".into());
    }
    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_flags() {
        let o = parse(&args(&[
            "--dry-run",
            "--user-data",
            "/u",
            "--backend=http://127.0.0.1:5200",
            "--limit",
            "3",
            "--max-cost=2.5",
        ]))
        .unwrap();
        assert_eq!(o.mode, Some(Mode::DryRun));
        assert_eq!(o.user_data, Some(PathBuf::from("/u")));
        assert_eq!(o.backend.as_deref(), Some("http://127.0.0.1:5200"));
        assert_eq!(o.limit, Some(3));
        assert_eq!(o.max_cost, Some(2.5));
        let o = parse(&args(&["--run", "--judge-cmd", "./judge.sh --x"])).unwrap();
        assert_eq!(o.judge_cmd.as_deref(), Some("./judge.sh --x"));
        assert!(parse(&args(&["--help"])).unwrap().help);
        assert_eq!(
            parse(&args(&["--weekly"])).unwrap().mode,
            Some(Mode::Weekly)
        );
    }

    #[test]
    fn rejects_bad_combinations() {
        for bad in [
            &[][..],
            &["--run", "--list"],
            &["--weekly", "--dry-run"],
            &["--list", "--limit", "2"],
            &["--weekly", "--limit", "2"],
            &["--run", "--limit", "0"],
            &["--run", "--limit", "x"],
            &["--run", "--max-cost", "-1"],
            &["--run", "--judge-cmd", "j", "--max-cost", "1"],
            &["--run", "--judge-cmd", " "],
            &["--run=yes"],
            &["--run", "--user-data"],
            &["--frobnicate"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }
}
