//! Flags and `--help`: scout-report's flags (scout_report::cli), on port 5132.

use scout_report::cli::Options;
use std::ffi::OsString;

pub const DEFAULT_PORT: u16 = 5132;

pub const USAGE: &str = "\
creator-report: swipe through the content creators npm run creators:scout found, and
see the ones you like with their socials, on a webpage

Usage:
  npm run creators:site [-- --lan] [-- --port N] [-- --user-data <dir>]

Options:
  --user-data <dir>    FunSync's userData folder
                       (default $FUNSYNC_USER_DATA, else ~/Library/Application Support/funsync-player)
  --port N             Port to listen on (default 5132; 0 picks a free one)
  --lan                Serve the home network too (every interface, 0.0.0.0), not just this Mac
  -h, --help           This text

The page is http://127.0.0.1:<port>/ (it opens the Creators tab, /creators), bound to
the loopback interface only; with --lan it also answers on the Mac's network address,
which the banner prints. With an access PIN set (npm run access:pin -- set <pin>,
<userData>/access-pin.json, reread on every request) other devices first get the
/login page (the scout report's); a login lasts 30 days. This Mac never needs the PIN.

Creators tab (/creators): one card per creator in <userData>/creators.json (written by
npm run creators:scout; reread whenever it changes) not swiped yet, best score first,
each deal shuffled by up to 2 score points either way (Shuffle deals a new order). A
card shows the creator's pictures (Instagram first; tap the sides or Space to browse),
Instagram and Twitch accounts with followers and last post / last live, and their
EroScripts (free downloadable, paid) and FapTap counts, score and top posts.
Right = yes, left = no, down = skip (back a few cards later), Backspace = undo.

Their downloadable posts (free EroScripts posts with a fetchable video, FapTap videos)
are swiped on the swipe deck (npm run scout:report), with a Creator badge and a score
bonus; a right swipe there downloads at the next daily npm run creators:scout.

Swiped tab (/swiped): the creators already swiped, newest first, with their full cards
one at a time (left / right = previous / next, Y / N = change to yes / no) or as a
list; filter All / Yes / No, search by name or handle.

Collection tab (/collection): the creators you like (swiped yes, favourite artists,
pornstar list) with their Instagram, Twitch and other confirmed profiles, as a list
or cards; changing one to no takes her posts not swiped yet off the swipe deck.

Creator swipes go to <userData>/creator-swipes.json (atomically, one at a time; the
previous copy is kept as creator-swipes.json.bak), the only file the CLI writes. Pictures are
served from <userData>/creator-pictures/<slug>/ at /pictures/<slug>/<file> and nowhere
else. Stop it with Ctrl-C.

Exit codes: 0 ok, 1 error (2 and 3 are not used).
";

pub fn parse(args: &[OsString]) -> Result<Options, String> {
    scout_report::cli::parse_with_port(args, DEFAULT_PORT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn defaults_to_port_5132() {
        let o = parse(&args(&[])).unwrap();
        assert_eq!(
            (o.port, o.lan, o.help, o.user_data),
            (5132, false, false, None)
        );
        let o = parse(&args(&["--port=0", "--lan", "--user-data", "/u"])).unwrap();
        assert_eq!(o.port, 0);
        assert!(o.lan);
        assert_eq!(o.user_data, Some("/u".into()));
        assert!(parse(&args(&["--bogus"])).is_err());
    }
}
