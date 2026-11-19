//! `npm run status:site`: one page that says whether each of FunSync's local sites is
//! up: the backend, the web remote, the swipe deck, the Creators deck and the
//! Collection (crate::probe::SITES). Every load of `/api/status` probes them from this Mac
//! (loopback, so the access PIN never gets in the way) and checks whether each port
//! also answers on the Mac's network address, i.e. whether a phone can reach it.
//! Bound to 127.0.0.1, or to every interface with `--lan`; other devices need the
//! access PIN when one is set, through scout-report's gate and login page. Writes nothing.

pub mod cli;
pub mod probe;
pub mod server;

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
    let (listener, user_data) = scout_report::listen(opts, "Status", out)?;
    server::serve(&listener, &user_data, err);
    Ok(())
}
