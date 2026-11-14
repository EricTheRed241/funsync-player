//! Flags and `--help`.

use std::ffi::OsString;
use std::path::PathBuf;

pub const DEFAULT_PORT: u16 = 5131;

pub const USAGE: &str = "\
scout-report: browse every EroScripts post the Discovery scout (npm run eroscripts:scout)
has judged, with its verdict, and the waiting list, on a webpage; swipe through the
waiting list to pick what the scout downloads

Usage:
  npm run scout:report [-- --lan] [-- --port N] [-- --user-data <dir>]

Options:
  --user-data <dir>    FunSync's userData folder
                       (default $FUNSYNC_USER_DATA, else ~/Library/Application Support/funsync-player)
  --port N             Port to listen on (default 5131; 0 picks a free one)
  --lan                Serve the home network too (every interface, 0.0.0.0), not just this Mac
  -h, --help           This text

The page is http://127.0.0.1:<port>/, bound to the loopback interface only; with
--lan it also answers on the Mac's network address, which the banner prints. It has
no login: anyone on the network can read it, and swipe, while it runs. Every load of
/api/report rereads eroscripts-scout-state.json (verdicts, feed coverage), eroscripts-scout.json
(the waiting list) and eroscripts-scout-rejected.json (scenes turned down); a missing
file shows as an empty section.

The swipe deck, http://127.0.0.1:<port>/swipe, shows the waiting posts not in the
library, loosely highest score first, one card at a time; tabs pick the stack:
Multi-axis (default), Single-axis or SLR. Each deal shifts every score by up to 4 either
way on Multi-axis (MULTI_JITTER: it stays close to score order), up to 10 on
Single-axis (SINGLE_JITTER: low scorers come up too) and up to 2 on SLR (SLR_JITTER);
the page's ⚙ sets each amplitude from 0 (strictly by score) to 30, per device
(/api/deck?jitter=N, /api/slr/deck?jitter=N).
On a wide window (900 px and up) the card puts the image on the left and every
detail beside it: all tags with their tag scores, likes, hosts, size, dates and the
scene list. Right = wanted: the next
npm run eroscripts:scout -- --fetch --auto downloads it whatever its score or axes.
Left = hidden from the deck only (the scout treats the post as before). Skip (up)
puts the card back a few cards later, saving nothing. Undo forgets the swipe or skip.
Swipes go to eroscripts-scout-swipes.json (atomically; the previous copy is kept as
eroscripts-scout-swipes.json.bak).

The SLR tab deals the SexLikeReal scenes in slr-scout.json (npm run slr:scout) with
their bonuses (hand-made script, PMV, compilation) and shows the account's download
quota the scout last read: hand-crafted and AI scripts left this month, videos left in
the 7-day window. Its swipes go to slr-scout-swipes.json (and .bak), by SLR scene id;
right = wanted, left = hidden. These two swipes files are the only files the CLI
writes. Stop it with Ctrl-C.

Exit codes: 0 ok, 1 error (2 and 3 are not used).
";

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub help: bool,
    pub user_data: Option<PathBuf>,
    pub port: u16,
    /// Listen on every interface (0.0.0.0) instead of loopback only.
    pub lan: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            help: false,
            user_data: None,
            port: DEFAULT_PORT,
            lan: false,
        }
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
        let mut value = |name: &str| -> Result<String, String> {
            match inline {
                Some(v) => Ok(v.to_string()),
                None => it
                    .next()
                    .and_then(|v| v.to_str())
                    .map(str::to_string)
                    .ok_or_else(|| format!("{name} needs a value")),
            }
        };
        match flag {
            "-h" | "--help" if inline.is_none() => o.help = true,
            "--lan" if inline.is_none() => o.lan = true,
            "--user-data" => o.user_data = Some(PathBuf::from(value(flag)?)),
            "--port" => {
                let v = value(flag)?;
                o.port = v
                    .parse()
                    .map_err(|_| format!("--port must be 0..65535, got {v}"))?;
            }
            "--help" => return Err("--help takes no value".into()),
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
        assert_eq!(parse(&args(&[])).unwrap(), Options::default());
        assert_eq!(Options::default().port, 5131);
        let o = parse(&args(&["--user-data", "/u", "--port", "8000"])).unwrap();
        assert_eq!(o.user_data, Some(PathBuf::from("/u")));
        assert_eq!(o.port, 8000);
        let o = parse(&args(&["--user-data=/v", "--port=0"])).unwrap();
        assert_eq!(o.user_data, Some(PathBuf::from("/v")));
        assert_eq!(o.port, 0);
        assert!(parse(&args(&["--help"])).unwrap().help);
        assert!(!parse(&args(&[])).unwrap().lan);
        assert!(parse(&args(&["--lan"])).unwrap().lan);
        assert!(parse(&args(&["-h"])).unwrap().help);
    }

    #[test]
    fn rejects_bad_arguments() {
        for bad in [
            &["--port"][..],
            &["--port", "70000"],
            &["--port", "x"],
            &["--user-data"],
            &["--help=1"],
            &["--lan=1"],
            &["--host", "0.0.0.0"],
            &["--bogus"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }
}
