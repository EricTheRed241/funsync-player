//! Flags and `--help`.

use std::ffi::OsString;
use std::path::PathBuf;

pub const USAGE: &str = "\
slr-scout: find SexLikeReal VR scenes of the wanted performers, score them like the
EroScripts deck, record the account's download quota (<userData>/slr-scout.json, read
by the scout:report swipe page), and download the scenes swiped right

Usage:
  npm run slr:scout [-- --actor <name>]... [--no-forum]
  npm run slr:scout -- --fetch [--dry-run]
  npm run slr:scout -- --quota
  npm run slr:scout -- --login [--credentials <file>]

Options:
  --user-data <dir>     FunSync's userData folder
                        (default $FUNSYNC_USER_DATA, else ~/Library/Application Support/funsync-player)
  --actor <name>        Scout only this performer, as SLR spells her (repeatable)
  --no-forum            Don't search EroScripts for hand-made scripts (matches from the
                        previous run are kept)
  --fetch               Download the scenes swiped right (<userData>/slr-scout-swipes.json)
                        that aren't fetched yet: video and script, into the library
  --dry-run             With --fetch: print the plan (download choice, script, target
                        path); no download request is sent
  --quota               Only refresh the download quota in the existing file
  --login               Sign in to SLR in a visible Chrome window ($FUNSYNC_CHROME_PATH,
                        profile <userData>/slr-chrome), save the session to
                        <userData>/slr-session.json (mode 0600), then refresh the quota
  --credentials <file>  For --login: \"username: …\" and \"password: …\" lines
                        (default $FUNSYNC_SLR_CREDENTIALS, else slr_creds.txt at the repo root)
  -h, --help            This text

A full run looks up the performers of slrScout (renderer/js/content-preferences.json):
its actors, the favourite artists (includeFavourites: FAVOURITES with the Artists view's
changes in config.json) and the listed pornstars (includePornstars, with their aliases);
names SLR doesn't know are skipped and listed. It walks each one's scenes (each scene
once; details paced at 500 ms, under SLR's 120 requests a minute; a rate-limited request
waits for the reset and is retried), keeps the ones slrScout asks for (vrOnly,
requireCreampie), and collects hand-made scripts: SLR's hand-crafted ones and EroScripts
posts in Free or Paid Scripts whose title contains the scene's title (title searches per
performer with scenes; forum session from config.json, read-only, paced at 4 s).
Scenes score like the EroScripts deck (candidateScore: tag scores from SLR's
categories with the Tags view's changes, VR, artist in the library, favourite bonus,
hand-made multi-axis, likes from SLR favourites at slrScout.popularityPer a point) plus
the slrScout bonuses (hand-made script, PMV, compilation). scriptSource says where a
download takes the script from: a free EroScripts post, else SLR's hand-crafted
script, else its AI script. fetchedAt is kept from the previous file.
The quota (hand-crafted scripts, AI scripts, videos) needs a session; when it can't
be read, the previous quota is kept and quotaError says why.

--fetch takes each right-swiped scene (highest score first): SLR's download within the
video caps (maxResolution, else optimalResolution; never the trailer) into
<library parent>/_staging, then <Other>/<first performer>/<Performers> - <Title>
(<Studio>)<VR suffix>/ with the script and an slr.json sidecar; generated twist / roll /
pitch (scripts/upgrade-multi-axis.mjs); fetchedAt in slr-scout.json; at the end the app
is asked to rescan. A scene without a script to take is skipped. It stops at the
weekly video quota and at the library cap (libraryMaxVideos).

Exit codes: 0 ok, 1 error, 2 rate limited (HTTP 429), or --fetch stopped by a quota or
the library cap with work left, 3 not logged in (no or expired SLR session; a scout run
still writes the scenes).
";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub help: bool,
    pub user_data: Option<PathBuf>,
    pub actors: Vec<String>,
    pub no_forum: bool,
    pub fetch: bool,
    pub dry_run: bool,
    pub quota: bool,
    pub login: bool,
    pub credentials: Option<PathBuf>,
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
        let takes_value = matches!(flag, "--user-data" | "--actor" | "--credentials");
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
            "--actor" => {
                let name = value()?.trim().to_string();
                if name.is_empty() {
                    return Err("--actor needs a name".into());
                }
                o.actors.push(name);
            }
            "--credentials" => o.credentials = Some(PathBuf::from(value()?)),
            "--no-forum" => o.no_forum = true,
            "--fetch" => o.fetch = true,
            "--dry-run" => o.dry_run = true,
            "--quota" => o.quota = true,
            "--login" => o.login = true,
            _ => return Err(format!("unknown argument {arg} (see --help)")),
        }
    }
    if o.help {
        return Ok(o);
    }
    let scouting = !o.actors.is_empty() || o.no_forum;
    if (o.quota || o.login) && (scouting || o.fetch) {
        return Err(
            "--quota and --login don't scout or fetch: drop --actor, --no-forum and --fetch".into(),
        );
    }
    if o.fetch && scouting {
        return Err("--fetch doesn't scout: drop --actor and --no-forum".into());
    }
    if o.dry_run && !o.fetch {
        return Err("--dry-run goes with --fetch".into());
    }
    if o.credentials.is_some() && !o.login {
        return Err("--credentials goes with --login".into());
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
            "--actor",
            "Miss Lexa",
            "--actor=Lexi Luna",
            "--no-forum",
        ]))
        .unwrap();
        assert_eq!(o.user_data, Some(PathBuf::from("/u")));
        assert_eq!(o.actors, ["Miss Lexa", "Lexi Luna"]);
        assert!(o.no_forum && !o.quota && !o.login && !o.fetch);
        assert_eq!(parse(&args(&[])).unwrap(), Options::default());
        assert!(parse(&args(&["--quota"])).unwrap().quota);
        let o = parse(&args(&["--fetch", "--dry-run"])).unwrap();
        assert!(o.fetch && o.dry_run);
        let o = parse(&args(&["--login", "--credentials", "/c"])).unwrap();
        assert!(o.login);
        assert_eq!(o.credentials, Some(PathBuf::from("/c")));
        assert!(parse(&args(&["--help", "--bogus"])).is_err());
        assert!(
            parse(&args(&["-h", "--quota", "--actor", "x"]))
                .unwrap()
                .help
        );
    }

    #[test]
    fn rejects_bad_combinations() {
        for bad in [
            &["--quota", "--actor", "x"][..],
            &["--login", "--no-forum"],
            &["--quota", "--fetch"],
            &["--fetch", "--actor", "x"],
            &["--fetch", "--no-forum"],
            &["--dry-run"],
            &["--fetch=1"],
            &["--credentials", "/c"],
            &["--actor"],
            &["--actor", " "],
            &["--quota=1"],
            &["--user-data"],
            &["--bogus"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }
}
