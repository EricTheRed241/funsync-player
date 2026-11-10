//! Flags and `--help`.

use std::ffi::OsString;
use std::path::PathBuf;

pub const USAGE: &str = "\
pornstar-tags: tag library videos with the pornstars they show (<userData>/pornstar-tags.json,
merged into the library's tags by FunSync on every scan)

Usage:
  npm run pornstar:tags [-- --dry-run]          show what would change (the default)
  npm run pornstar:tags -- --apply [--no-rescan]
  npm run pornstar:tags -- --daily [--no-rescan]
  npm run pornstar:tags -- --list

Options:
  --user-data <dir>    FunSync's userData folder
                       (default ~/Library/Application Support/funsync-player)
  --dry-run            Recompute and show the tags, scene counts and video changes;
                       write nothing (the default)
  --apply              Write the file when it changed (atomically; the previous one is
                       kept as pornstar-tags.json.bak), then ask the running app to rescan
  --daily              --apply, but only once a day (UTC): exits 0 doing nothing when
                       pornstar-tags.json or pornstar-tags.last-run shows a run today
  --no-rescan          Don't ask the app to rescan after writing
  --list               Print the current file: tags with scene counts, applied or
                       pending, and the tagged videos
  -h, --help           This text

The list and the threshold are pornstarTags in renderer/js/content-preferences.json:
each performer's name, aliases and scene phrases are matched as whole words
(case, full-width letters and punctuation don't matter) against each video's path
below its library source (artist, scene folder, file name), its eroscripts.json
title, its faptap.json title and performers, and its EroScripts topic URL slug
(library.scriptTags in config.json). Performers the VRBangers scout names in its
scene folders (\"<Performers> - <Title> (VRBangers)\") join the list. A tag is
applied once it shows in minScenes distinct scene folders; below that it's pending.
FapTap performers not in the list are reported as candidates, never applied.

Only videos with a .funscript beside them count. The CLI reads config.json and
the library and never writes either; it's safe while the app is open.

Exit codes: 0 ok, 1 error (2 and 3 are not used).
";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub help: bool,
    pub user_data: Option<PathBuf>,
    pub list: bool,
    pub apply: bool,
    pub daily: bool,
    pub no_rescan: bool,
}

pub fn parse(args: &[OsString]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut dry_run = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let arg = arg
            .to_str()
            .ok_or_else(|| format!("argument is not UTF-8: {}", arg.to_string_lossy()))?;
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v)),
            _ => (arg, None),
        };
        match flag {
            "-h" | "--help" => o.help = true,
            "--user-data" => {
                let v = match inline {
                    Some(v) => v.to_string(),
                    None => it
                        .next()
                        .and_then(|v| v.to_str())
                        .map(str::to_string)
                        .ok_or("--user-data needs a value")?,
                };
                o.user_data = Some(PathBuf::from(v));
            }
            "--list" => o.list = true,
            "--dry-run" => dry_run = true,
            "--apply" => o.apply = true,
            "--daily" => o.daily = true,
            "--no-rescan" => o.no_rescan = true,
            _ => return Err(format!("unknown argument {arg} (see --help)")),
        }
        if inline.is_some() && flag != "--user-data" {
            return Err(format!("{flag} takes no value"));
        }
    }
    if o.help {
        return Ok(o);
    }
    if [dry_run, o.apply, o.daily, o.list]
        .iter()
        .filter(|&&b| b)
        .count()
        > 1
    {
        return Err("--dry-run, --apply, --daily and --list exclude each other".into());
    }
    if o.no_rescan && !(o.apply || o.daily) {
        return Err("--no-rescan goes with --apply or --daily".into());
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
        let o = parse(&args(&["--user-data", "/u", "--apply", "--no-rescan"])).unwrap();
        assert_eq!(o.user_data, Some(PathBuf::from("/u")));
        assert!(o.apply && o.no_rescan && !o.daily);
        let o = parse(&args(&["--user-data=/v", "--daily"])).unwrap();
        assert_eq!(o.user_data, Some(PathBuf::from("/v")));
        assert!(o.daily);
        assert_eq!(parse(&args(&[])).unwrap(), Options::default());
        assert_eq!(parse(&args(&["--dry-run"])).unwrap(), Options::default());
        assert!(parse(&args(&["--list"])).unwrap().list);
        assert!(parse(&args(&["-h", "--bogus=1"])).is_err());
        assert!(parse(&args(&["--help"])).unwrap().help);
    }

    #[test]
    fn rejects_bad_combinations() {
        for bad in [
            &["--apply", "--dry-run"][..],
            &["--apply", "--daily"],
            &["--list", "--apply"],
            &["--no-rescan"],
            &["--apply=1"],
            &["--user-data"],
            &["--bogus"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }
}
