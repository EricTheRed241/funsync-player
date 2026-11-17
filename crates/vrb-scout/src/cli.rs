//! Flags and `--help`.

use crate::sites::{self, SITES, Site};
use std::ffi::OsString;
use std::path::PathBuf;

pub const USAGE: &str = "\
vrb-scout: find VRBangers and VRConk VR scenes that come with the studio's free toy
script, score them like the EroScripts deck (<userData>/vrb-scout.json, read by the
scout:report swipe page), and download the scenes swiped right

Usage:
  npm run vrb:scout [-- --sites vrbangers,vrconk]
  npm run vrb:scout -- --fetch [--dry-run] [--limit <n>] [--credentials <file>] [--headful]

Options:
  --user-data <dir>     FunSync's userData folder
                        (default $FUNSYNC_USER_DATA, else ~/Library/Application Support/funsync-player)
  --sites <list>        Only these sites, comma-separated (vrbangers, vrconk; default all).
                        A scan keeps the other sites' scenes from the previous file
  --fetch               Download the scenes swiped right (<userData>/vrb-scout-swipes.json)
                        that aren't fetched or in the library yet: script and video
  --dry-run             With --fetch: sign in and print the plan (chosen download, its
                        link without the token, target folder); nothing is downloaded
  --limit <n>           With --fetch: download at most n scenes this run
  --credentials <file>  With --fetch: the VRBangers account, \"Username: …\" and
                        \"password: …\" lines (default $FUNSYNC_VRB_CREDENTIALS, else
                        vb_hack.txt at the repo root)
  --headful             With --fetch: show the Chrome window
  -h, --help            This text

A scan walks each site's video listing (content.<site>/api/content/v1, public, 1 s
between requests to a host), keeps the scenes with a toy script and reads the details
(categories, gallery, fps) of new or changed ones only; the rest come from the previous
file. Scenes are screened (screening in renderer/js/content-preferences.json: excluded
words, blocked creators / tags / title words, furry and SFW tags, minimum length) and
scored like the EroScripts deck (candidateScore: tag scores from the categories with the
Tags view's changes, VR, performer in the library, favourite bonus, likes from the
site's views at vrbScout.popularityPer a point, plus slrScout's PMV and compilation
bonuses). inLibrary marks scenes the library already has (a vrb.json, eroscripts.json or
slr.json sidecar naming them, a folder named after them, or the site's file name);
fetchedAt is kept from the previous file.

--fetch takes each right-swiped scene (highest score first): the public toy script,
then the video with the member's signed link from the scene page (headless Google
Chrome signed in to the site, profile <userData>/vrbangers-chrome): the best within the
VR caps (videoCaps: 8K, 60 fps), stereo before mono, curl into <library parent>/_staging,
then <Other>/<first performer>/<Performers> - <Title> (<Site>)_<projection>/ with a
vrb.json sidecar; generated twist / roll / pitch (scripts/upgrade-multi-axis.mjs);
fetchedAt in vrb-scout.json; at the end the app is asked to rescan. It stops at the
library cap (libraryMaxVideos). VRConk scenes need a VRConk membership: when the
account can't sign in there or gets no download links, they are skipped.

Exit codes: 0 ok, 1 error, 2 rate limited (HTTP 429), or --fetch stopped by the library
cap or --limit with work left, 3 not logged in (VRBangers refused the account).
";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub help: bool,
    pub user_data: Option<PathBuf>,
    /// `--sites` (None: all).
    pub sites: Option<Vec<String>>,
    pub fetch: bool,
    pub dry_run: bool,
    pub limit: Option<usize>,
    pub credentials: Option<PathBuf>,
    pub headful: bool,
}

impl Options {
    /// The chosen sites, in table order.
    pub fn sites(&self) -> Vec<&'static Site> {
        SITES
            .iter()
            .filter(|s| {
                self.sites
                    .as_ref()
                    .is_none_or(|l| l.iter().any(|k| k == s.key))
            })
            .collect()
    }
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
        let takes_value = matches!(
            flag,
            "--user-data" | "--sites" | "--limit" | "--credentials"
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
        match flag {
            "-h" | "--help" => o.help = true,
            "--user-data" => o.user_data = Some(PathBuf::from(value()?)),
            "--sites" => {
                let list: Vec<String> = value()?
                    .split(',')
                    .map(|s| s.trim().to_ascii_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect();
                if list.is_empty() {
                    return Err("--sites needs at least one site".into());
                }
                if let Some(bad) = list.iter().find(|k| sites::by_key(k).is_none()) {
                    let known: Vec<&str> = SITES.iter().map(|s| s.key).collect();
                    return Err(format!(
                        "unknown site {bad:?} (known: {})",
                        known.join(", ")
                    ));
                }
                o.sites = Some(list);
            }
            "--limit" => {
                let v = value()?;
                let n: usize = v
                    .parse()
                    .map_err(|_| format!("--limit takes a whole number, got {v:?}"))?;
                if n == 0 {
                    return Err("--limit must be at least 1".into());
                }
                o.limit = Some(n);
            }
            "--credentials" => o.credentials = Some(PathBuf::from(value()?)),
            "--fetch" => o.fetch = true,
            "--dry-run" => o.dry_run = true,
            "--headful" => o.headful = true,
            _ => return Err(format!("unknown argument {arg} (see --help)")),
        }
    }
    if o.help {
        return Ok(o);
    }
    if !o.fetch {
        for (set, flag) in [
            (o.dry_run, "--dry-run"),
            (o.limit.is_some(), "--limit"),
            (o.credentials.is_some(), "--credentials"),
            (o.headful, "--headful"),
        ] {
            if set {
                return Err(format!("{flag} goes with --fetch"));
            }
        }
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
            "--sites",
            "VRConk, vrbangers",
        ]))
        .unwrap();
        assert_eq!(o.user_data, Some(PathBuf::from("/u")));
        let keys: Vec<&str> = o.sites().iter().map(|s| s.key).collect();
        assert_eq!(keys, ["vrbangers", "vrconk"]);
        let o = parse(&args(&["--sites=vrconk"])).unwrap();
        assert_eq!(o.sites().len(), 1);
        assert_eq!(parse(&args(&[])).unwrap().sites().len(), SITES.len());
        let o = parse(&args(&[
            "--fetch",
            "--dry-run",
            "--limit",
            "3",
            "--credentials=/c",
            "--headful",
        ]))
        .unwrap();
        assert!(o.fetch && o.dry_run && o.headful);
        assert_eq!(o.limit, Some(3));
        assert_eq!(o.credentials, Some(PathBuf::from("/c")));
        assert!(parse(&args(&["-h", "--dry-run"])).unwrap().help);
    }

    #[test]
    fn rejects_bad_flags() {
        for bad in [
            &["--dry-run"][..],
            &["--limit", "2"],
            &["--headful"],
            &["--credentials", "/c"],
            &["--fetch", "--limit", "0"],
            &["--fetch", "--limit", "x"],
            &["--sites", "blowvr"],
            &["--sites", ","],
            &["--sites"],
            &["--fetch=1"],
            &["--bogus"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }
}
