//! Flags and `--help`.

use std::ffi::OsString;
use std::path::PathBuf;

pub const USAGE: &str = "\
video-tags: keep <userData>/video-tags.json, the tags FunSync adds to videos on every library scan

Usage:
  npm run video:tags -- --list
  npm run video:tags -- --proposals <file.json> [--replace] [--dry-run | --apply] [--no-rescan]
  npm run video:tags -- --remove <video path> [--remove <video path> ...] [--dry-run | --apply] [--no-rescan]

Options:
  --user-data <dir>    FunSync's userData folder
                       (default ~/Library/Application Support/funsync-player)
  --list               Print the current entries
  --proposals <file>   Tags to add: {\"videos\": {\"<path>\": {\"tags\": [...]}}} or
                       [{\"path\": ..., \"tags\": [...]}]; an optional \"by\" (per entry or
                       top level) is stored, other fields are ignored
  --replace            Set each proposed entry's tags exactly (an empty list removes
                       the entry) instead of adding to them
  --remove <path>      Remove a video's entry (repeatable)
  --dry-run            Show what would change, write nothing (the default)
  --apply              Write the file (atomically; the previous one is kept as
                       video-tags.json.bak), then ask the running app to rescan
  --no-rescan          Don't ask the app to rescan after --apply
  -h, --help           This text

Every proposed entry is checked: tags must be slugs (a-z, 0-9, '-'; lower-cased),
the path absolute, an existing regular file, not an AppleDouble '._' file, and
inside one of the library sources in config.json (read only). Each path segment
is stored in the filesystem's own spelling (NFC vs NFD). Any invalid entry:
nothing is written. The app applies only the tags in VIDEO_TAGGER_TAGS
(renderer/js/content-preferences.js) and ignores the rest.

The CLI never writes config.json or the library.

Exit codes: 0 ok, 1 error or invalid input (2 and 3 are not used).
";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub help: bool,
    pub user_data: Option<PathBuf>,
    pub list: bool,
    pub proposals: Option<PathBuf>,
    pub apply: bool,
    pub replace: bool,
    pub remove: Vec<String>,
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
        match flag {
            "-h" | "--help" => o.help = true,
            "--user-data" => o.user_data = Some(PathBuf::from(value()?)),
            "--list" => o.list = true,
            "--proposals" => o.proposals = Some(PathBuf::from(value()?)),
            "--dry-run" => dry_run = true,
            "--apply" => o.apply = true,
            "--replace" => o.replace = true,
            "--remove" => o.remove.push(value()?),
            "--no-rescan" => o.no_rescan = true,
            _ => return Err(format!("unknown argument {arg} (see --help)")),
        }
        if inline.is_some() && !matches!(flag, "--user-data" | "--proposals" | "--remove") {
            return Err(format!("{flag} takes no value"));
        }
    }
    if o.help {
        return Ok(o);
    }
    if dry_run && o.apply {
        return Err("--dry-run and --apply exclude each other".into());
    }
    if o.list && (o.proposals.is_some() || !o.remove.is_empty() || o.apply || o.replace) {
        return Err("--list prints the file; use it alone (with --user-data)".into());
    }
    if o.replace && o.proposals.is_none() {
        return Err("--replace needs --proposals".into());
    }
    if !o.list && o.proposals.is_none() && o.remove.is_empty() {
        return Err("nothing to do: give --list, --proposals or --remove (see --help)".into());
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
            "--user-data",
            "/u",
            "--proposals=/p.json",
            "--replace",
            "--apply",
            "--no-rescan",
        ]))
        .unwrap();
        assert_eq!(o.user_data, Some(PathBuf::from("/u")));
        assert_eq!(o.proposals, Some(PathBuf::from("/p.json")));
        assert!(o.replace && o.apply && o.no_rescan);
        let o = parse(&args(&["--remove", "/a.mp4", "--remove=/b.mp4"])).unwrap();
        assert_eq!(o.remove, vec!["/a.mp4".to_string(), "/b.mp4".to_string()]);
        assert!(!o.apply);
        assert!(parse(&args(&["--help", "--bogus-later-ignored"])).is_err());
        assert!(parse(&args(&["--help"])).unwrap().help);
    }

    #[test]
    fn rejects_bad_combinations() {
        for bad in [
            &["--proposals", "p", "--dry-run", "--apply"][..],
            &["--list", "--apply"],
            &["--replace", "--remove", "/a"],
            &[],
            &["--proposals"],
            &["--list=yes"],
            &["--frobnicate"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
        assert!(parse(&args(&["--list"])).is_ok());
    }
}
