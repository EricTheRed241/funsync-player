//! Flags and `--help`.

use std::ffi::OsString;
use std::path::PathBuf;

pub const USAGE: &str = "\
library-thumbs: give every library video a thumbnail ahead of time, so the desktop,
the web remote and HereSphere/DeoVR never wait on one

Usage:
  npm run library:thumbs [-- --limit <n> --backend <url>]

Options:
  --backend <url>      The running app's backend
                       (default http://127.0.0.1:<port>, the port from config.json)
  --user-data <dir>    FunSync's userData folder, for config.json
                       (default $FUNSYNC_USER_DATA, else ~/Library/Application Support/funsync-player)
  --limit <n>          At most n videos (in the backend's list order)
  -h, --help           This text

Asks the backend for the list of registered videos (GET /api/remote/videos),
then for each video's thumbnail (GET /api/media/thumb/<id>), one at a time.
A cached thumbnail answers at once; a missing one is generated with the app's
own frame picker and cached in thumb-cache. Each failure is printed (path and
HTTP status) and a summary ends the run. A video whose file is gone (404) is
counted as missing and skipped. Needs the app to be running; writes nothing
itself.

Exit codes: 0 ok (missing files included), 1 error (app not running, bad
flags), 2 some thumbnails failed (try again next run).
";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub help: bool,
    pub user_data: Option<PathBuf>,
    pub backend: Option<String>,
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
            "-h" | "--help" if inline.is_none() => o.help = true,
            "--user-data" => o.user_data = Some(PathBuf::from(value()?)),
            "--backend" => o.backend = Some(value()?),
            "--limit" => {
                let v = value()?;
                let n = v
                    .parse::<usize>()
                    .ok()
                    .filter(|&n| n > 0)
                    .ok_or_else(|| format!("--limit needs a positive number, not {v}"))?;
                o.limit = Some(n);
            }
            _ => return Err(format!("unknown argument {arg} (see --help)")),
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
            "--limit=3",
            "--backend",
            "http://x:1",
        ]))
        .unwrap();
        assert_eq!(o.user_data, Some(PathBuf::from("/u")));
        assert_eq!(o.limit, Some(3));
        assert_eq!(o.backend.as_deref(), Some("http://x:1"));
        assert_eq!(parse(&args(&[])).unwrap(), Options::default());
        assert!(parse(&args(&["-h"])).unwrap().help);
        for bad in [
            &["--limit", "0"][..],
            &["--limit", "x"],
            &["--limit"],
            &["--bogus"],
            &["--help=1"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }
}
