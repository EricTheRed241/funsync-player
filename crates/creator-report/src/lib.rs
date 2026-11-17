//! `npm run creators:site`: the Creators site. Two swipe decks on one page:
//! the Creators tab (`/`), one card per content creator in `creators.json`
//! (written by `npm run creators:scout`), swiped yes or no; and the Posts tab
//! (`/posts`), the downloadable posts of the creators swiped yes, swiped want
//! (the next daily `creators:scout` run downloads it) or hide. Reads
//! `creators.json` (again whenever its modification time or size changes) and
//! serves the scout's pictures (`creator-pictures/`); the only file it writes
//! is `creator-swipes.json` (crate::swipes). Bound to 127.0.0.1, or to every
//! interface with `--lan`; other devices need the access PIN when one is set,
//! through scout-report's gate and login page.

pub mod cli;
pub mod creators;
pub mod deck;
pub mod pictures;
pub mod server;
pub mod swipes;

use std::ffi::OsString;
use std::io::Write;

/// Run the CLI; returns the exit code (serving never returns).
pub fn run(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match run_inner(args, out, err) {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "{}: {e}", server::NAME);
            1
        }
    }
}

fn run_inner(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> Result<(), String> {
    let opts = cli::parse(args)?;
    if opts.help {
        out.write_all(cli::USAGE.as_bytes())
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    let (listener, user_data) = scout_report::listen(opts, "Creators", out)?;
    server::serve(&listener, &user_data, err);
    Ok(())
}
