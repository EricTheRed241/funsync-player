//! Flags and `--help`.

use std::ffi::OsString;
use std::path::PathBuf;

pub const DEFAULT_PORT: u16 = 5131;

pub const USAGE: &str = "\
scout-report: the swipe deck: swipe through the EroScripts Discovery scout's waiting
list (npm run eroscripts:scout) and the site scouts' scenes to pick what the scouts
download

Usage:
  npm run scout:report [-- --lan] [-- --port N] [-- --user-data <dir>]

Options:
  --user-data <dir>    FunSync's userData folder
                       (default $FUNSYNC_USER_DATA, else ~/Library/Application Support/funsync-player)
  --port N             Port to listen on (default 5131; 0 picks a free one)
  --lan                Serve the home network too (every interface, 0.0.0.0), not just this Mac
  -h, --help           This text

The page is http://127.0.0.1:<port>/swipe (/ redirects there), bound to the loopback
interface only; with --lan it also answers on the Mac's network address, which the
banner prints. With an access PIN set (npm run access:pin -- set <pin>,
<userData>/access-pin.json, reread on every request) other devices first get the
/login page; a login lasts 30 days and also unlocks the app's backend on the same
address. Without a PIN anyone on the network can swipe while it runs; this Mac never
needs the PIN.

The deck deals one card at a time, loosely
highest score first, from every source: the EroScripts waiting posts not in the
library, and the scenes of slr-scout.json (npm run slr:scout), vrb-scout.json
(VRBangers and VRConk, npm run vrb:scout), pmvhaven-scout.json (npm run pmvhaven:scout)
and faptap-scout.json (npm run faptap:scout) not downloaded (fetchedAt) and not in the
library (inLibrary). Four tabs pick the stack: SLR (everything that downloads from
SexLikeReal, so costs the account's quota: SLR scenes, posts and creator posts whose video
is on SLR), Multi (default: multi-axis posts and scenes whose script has axes besides
stroke), VR (the rest that's VR) and Flat (the rest); no other tab shows SLR videos. A
scene a waiting post brings (the same SLR scene id, or the same VRBangers page) only shows
as the post: its script is free. Each deal shifts every score by up to 4 either way on
Multi (MULTI_JITTER: it stays close to score order), up to 3 on VR (VR_JITTER), 2 on SLR
(SLR_JITTER) and up to 10 on Flat (FLAT_JITTER: low scorers come up too); the page's ⚙
sets each amplitude from 0 (strictly by score) to 30, per device (/api/deck?jitter=N).
On a wide window (900 px and up) the card puts the image on the left and every
detail beside it: all tags with their tag scores, likes, hosts, size, dates and the
scene list (a scene: performers, studio, script, score parts). Right = wanted: the
next download of that source's scout takes it (EroScripts: npm run eroscripts:scout --
--fetch --auto, whatever its score or axes; a scene: that scout's --fetch). Left =
hidden from the deck only. Skip (up) puts the card back a few cards later, saving
nothing. Undo forgets the swipe or skip, on any tab. Swipes go to the source's own file,
by the site's id (atomically; the previous copy is kept as .bak):
eroscripts-scout-swipes.json, slr-scout-swipes.json, vrb-scout-swipes.json,
pmvhaven-scout-swipes.json and faptap-scout-swipes.json. These are the only files the
CLI writes. The SLR tab also shows the SLR account's download quota slr:scout last read:
videos left in the 7-day window, hand-crafted and AI scripts left this month.

Under the tabs a bar shows the library drive's free space in GB (df on the library
sources in config.json; with sources on several drives, the one with the least free).
Below swipeDeck.minFreeGb free (renderer/js/content-preferences.json) the drive counts
as full: Multi, VR, Flat and SLR take no swipes (the page turns them off, the swipe routes
answer 507) until space is freed; skip, undo and the Free space tab still work. The
bar rereads /api/drive every minute.

On a wide window a card also has a Watch link: a post's first video on its host
(rule34video, pixeldrain's viewer page, MEGA, SLR, …), a scene's video file (PMVHaven) or
page; each scene in the list links to its video too.

The Downloads page, /downloads (the Downloads link in the top bar; /api/downloads as
JSON), lists every right swipe: Waiting (not in the library yet; the next download run
takes it), Downloaded (in the last 30 days, newest first) and Won't download (turned down
when fetched, deleted later, or gone from its list), each with its picture, links and
when it was swiped / downloaded; above them the download job (eroscripts-scout) as the
job runner last saw it, with its log's last line while it runs.

A card's pictures are the post's or scene's images, then the preview frames the scouts
took at 25, 50 and 75 % of a video (<userData>/scout-frames/eroscripts/<topic id>/,
listed in the post's record of eroscripts-scout-frames.json, and
scout-frames/slr/<scene id>/, in the scene's frames record in slr-scout.json), at most
five. They're served at
/frames/<eroscripts|slr>/<id>/<25|50|75>.jpg and nowhere else. Stop it with Ctrl-C.

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
    parse_with_port(args, DEFAULT_PORT)
}

/// The same flags for a site whose port defaults to `default_port` (creator-report).
pub fn parse_with_port(args: &[OsString], default_port: u16) -> Result<Options, String> {
    let mut o = Options {
        port: default_port,
        ..Options::default()
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
