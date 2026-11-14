//! Flags and `--help`.

use std::ffi::OsString;
use std::path::PathBuf;

pub const USAGE: &str = "\
slr-scout: find SexLikeReal VR scenes of chosen performers, score them, and record the
account's download quota (<userData>/slr-scout.json, read by the scout:report swipe page)

Usage:
  npm run slr:scout [-- --actor <name>]... [--no-forum]
  npm run slr:scout -- --quota
  npm run slr:scout -- --login [--credentials <file>]

Options:
  --user-data <dir>     FunSync's userData folder
                        (default $FUNSYNC_USER_DATA, else ~/Library/Application Support/funsync-player)
  --actor <name>        A performer to scout, as SLR spells her (repeatable; default the
                        slrScout actors in renderer/js/content-preferences.json)
  --no-forum            Don't search EroScripts for hand-made scripts (matches from the
                        previous run are kept)
  --quota               Only refresh the download quota in the existing file
  --login               Sign in to SLR in a visible Chrome window ($FUNSYNC_CHROME_PATH,
                        profile <userData>/slr-chrome), save the session to
                        <userData>/slr-session.json (mode 0600), then refresh the quota
  --credentials <file>  For --login: \"username: …\" and \"password: …\" lines
                        (default $FUNSYNC_SLR_CREDENTIALS, else slr_creds.txt at the repo root)
  -h, --help            This text

A full run walks SLR's catalogue for each performer (details paced at 350 ms), keeps
the scenes slrScout asks for (vrOnly, requireCreampie: a Creampie category or
timestamp), flags PMVs and compilations, and collects hand-made scripts: SLR's
hand-crafted ones and EroScripts posts in Free or Paid Scripts whose title contains
the scene's title (forum session from config.json, read-only, paced at 4 s). Score =
the slrScout bonuses that apply + min(favorites / popularityPer, popularityMax).
The quota (hand-crafted scripts, AI scripts, videos) needs a session; when it can't
be read, the previous quota is kept and quotaError says why.

Exit codes: 0 ok, 1 error, 2 rate limited (HTTP 429), 3 not logged in (no or
expired SLR session; the scenes are still written).
";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub help: bool,
    pub user_data: Option<PathBuf>,
    pub actors: Vec<String>,
    pub no_forum: bool,
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
            "--quota" => o.quota = true,
            "--login" => o.login = true,
            _ => return Err(format!("unknown argument {arg} (see --help)")),
        }
    }
    if o.help {
        return Ok(o);
    }
    if (o.quota || o.login) && (!o.actors.is_empty() || o.no_forum) {
        return Err("--quota and --login don't scout: drop --actor and --no-forum".into());
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
        assert!(o.no_forum && !o.quota && !o.login);
        assert_eq!(parse(&args(&[])).unwrap(), Options::default());
        assert!(parse(&args(&["--quota"])).unwrap().quota);
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
