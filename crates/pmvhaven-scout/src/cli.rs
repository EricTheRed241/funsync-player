//! Flags and `--help`.

use std::ffi::OsString;
use std::path::PathBuf;

pub const USAGE: &str = "\
pmvhaven-scout: find PMVHaven videos that have a funscript, score them like the
EroScripts deck (<userData>/pmvhaven-scout.json, read by the scout:report swipe page),
and download the ones swiped right

Usage:
  npm run pmvhaven:scout [-- --pages <n>]
  npm run pmvhaven:scout -- --fetch [--dry-run] [--limit <n>]

Options:
  --user-data <dir>  FunSync's userData folder
                     (default $FUNSYNC_USER_DATA, else ~/Library/Application Support/funsync-player)
  --pages <n>        Read at most n list pages of 100 videos (default: all)
  --fetch            Download the videos swiped right (<userData>/pmvhaven-scout-swipes.json)
                     that aren't fetched or in the library yet: video and script
  --dry-run          With --fetch: print the plan; nothing is downloaded
  --limit <n>        With --fetch: download at most n videos this run
  -h, --help         This text

A scan pages through PMVHaven's videos with a funscript (no login; 1 s between
requests) and reads each one's details (its description and scripts), kept in
<userData>/pmvhaven-scout-cache.json and read again after pmvhavenScout.detailMaxAgeDays.
It drops videos PMVHaven flags as gay, trans or extreme, audio pieces (an audioWords
word in the title or tags), and what the shared screening turns down (excluded words,
blocked creators / tags / title words, furry, SFW, under the minimum length), then
picks a script: the uploader's own, else the best community script (hand-made before
auto-generated before ones for other devices such as rotary machines; then most
likes, then newest). Videos are VR when their frame is two eyes side by side (32:9),
or 2:1 / 1:1 with a vrWords word in the title or tags. Scores are candidateScore's
(tag scores, VR, artist in the library, favourite, hand-made axes, likes at
pmvhavenScout.popularityPer a point); pmv / compilation from pmvWords /
compilationWords add those tags. fetchedAt is kept from the previous file.

--fetch takes each right-swiped video (highest score first) that isn't fetched or in
the library: the mp4 into <library parent>/_staging as a .part, then
<Other>/<artist>/<Artist> - <Title> (PMVHaven)<VR suffix>/ with the script (and its
axis companions) and a pmvhaven.json sidecar; generated twist / roll / pitch
(scripts/upgrade-multi-axis.mjs); fetchedAt in pmvhaven-scout.json; at the end the
app is asked to rescan. It stops at the library cap (libraryMaxVideos). A video only
downloads when the library drive keeps swipeDeck.minFreeGb (content-preferences.json)
free after it; one that doesn't fit is left for later and the next tried (--dry-run
prints the check).

Exit codes: 0 ok, 1 error, 2 rate limited (HTTP 429) or work left (details still
to read, the library cap, --limit reached, a video left for lack of disk space).
";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub help: bool,
    pub user_data: Option<PathBuf>,
    pub pages: Option<u32>,
    pub fetch: bool,
    pub dry_run: bool,
    pub limit: Option<usize>,
}

pub fn parse(args: &[OsString]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let arg = arg
            .to_str()
            .ok_or_else(|| format!("argument is not UTF-8: {}", arg.to_string_lossy()))?;
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v)),
            _ => (arg, None),
        };
        let takes_value = matches!(flag, "--user-data" | "--pages" | "--limit");
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
        match flag {
            "-h" | "--help" => o.help = true,
            "--user-data" => o.user_data = Some(PathBuf::from(value()?)),
            "--pages" => o.pages = Some(positive(flag, &value()?)? as u32),
            "--limit" => o.limit = Some(positive(flag, &value()?)?),
            "--fetch" => o.fetch = true,
            "--dry-run" => o.dry_run = true,
            _ => return Err(format!("unknown argument {arg} (see --help)")),
        }
    }
    if o.help {
        return Ok(o);
    }
    if o.fetch && o.pages.is_some() {
        return Err("--fetch doesn't scan: drop --pages".into());
    }
    if o.dry_run && !o.fetch {
        return Err("--dry-run goes with --fetch".into());
    }
    if o.limit.is_some() && !o.fetch {
        return Err("--limit goes with --fetch".into());
    }
    Ok(o)
}

fn positive(flag: &str, v: &str) -> Result<usize, String> {
    match v.parse::<usize>() {
        Ok(n) if n > 0 && n <= u32::MAX as usize => Ok(n),
        _ => Err(format!("{flag} takes a whole number above 0, got {v:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_flags() {
        let o = parse(&args(&["--user-data", "/u", "--pages=3"])).unwrap();
        assert_eq!(o.user_data, Some(PathBuf::from("/u")));
        assert_eq!(o.pages, Some(3));
        assert!(!o.fetch);
        assert_eq!(parse(&args(&[])).unwrap(), Options::default());
        let o = parse(&args(&["--fetch", "--dry-run", "--limit", "1"])).unwrap();
        assert!(o.fetch && o.dry_run);
        assert_eq!(o.limit, Some(1));
        assert!(parse(&args(&["-h", "--pages", "2"])).unwrap().help);
        assert!(parse(&args(&["--help", "--bogus"])).is_err());
    }

    #[test]
    fn rejects_bad_combinations() {
        for bad in [
            &["--fetch", "--pages", "2"][..],
            &["--dry-run"],
            &["--limit", "2"],
            &["--fetch", "--limit", "0"],
            &["--pages", "x"],
            &["--pages"],
            &["--fetch=1"],
            &["--user-data"],
            &["--bogus"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }
}
