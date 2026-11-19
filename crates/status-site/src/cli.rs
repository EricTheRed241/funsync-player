//! Flags and `--help`: scout-report's flags (scout_report::cli), on port 5130.

use std::ffi::OsString;

pub const DEFAULT_PORT: u16 = 5130;

pub const USAGE: &str = "\
status-site: one page showing whether FunSync's local sites are up

Usage:
  npm run status:site [-- --lan] [-- --port N] [-- --user-data <dir>]

Options:
  --user-data <dir>    FunSync's userData folder, for the access PIN
                       (default $FUNSYNC_USER_DATA, else ~/Library/Application Support/funsync-player)
  --port N             Port to listen on (default 5130; 0 picks a free one)
  --lan                Serve the home network too (every interface, 0.0.0.0), not just this Mac
  -h, --help           This text

The page is http://127.0.0.1:<port>/; with --lan it also answers on the Mac's network
address, which the banner prints. It lists the backend (5123 /health), the web remote
(5123 /remote/), the swipe deck (5131, npm run scout:report), the Creators deck and the
Posts deck (5132, npm run creators:site), each up or down with its HTTP status and
response time, and whether its port answers on the network address too (a site started
without --lan answers on this Mac only). Every load of /api/status probes them again
from this Mac; the page reloads it every 15 s. With an access PIN set (npm run
access:pin) other devices first get the /login page, as on the other sites. It writes
nothing. Stop it with Ctrl-C.

Exit codes: 0 ok, 1 error (2 and 3 are not used).
";

pub fn parse(args: &[OsString]) -> Result<scout_report::cli::Options, String> {
    scout_report::cli::parse_with_port(args, DEFAULT_PORT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_port_5130() {
        assert_eq!(parse(&[]).unwrap().port, 5130);
        let o = parse(&["--port".into(), "8000".into(), "--lan".into()]).unwrap();
        assert_eq!((o.port, o.lan), (8000, true));
    }
}
