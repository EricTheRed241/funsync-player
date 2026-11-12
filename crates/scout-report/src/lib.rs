//! `npm run scout:report`: a localhost-only webpage listing every EroScripts
//! post the Discovery scout (`npm run eroscripts:scout`) has judged, with its
//! verdict, and the waiting list. Reads `eroscripts-scout-state.json`,
//! `eroscripts-scout.json` and `eroscripts-scout-rejected.json` in userData on
//! every request and never writes any file. Bound to 127.0.0.1 only.

pub mod cli;
pub mod report;
pub mod server;

use std::ffi::OsString;
use std::io::Write;

/// Run the CLI; returns the exit code (serving never returns).
pub fn run(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match run_inner(args, out, err) {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "scout-report: {e}");
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
    let user_data = opts
        .user_data
        .or_else(video_tags::host::default_user_data)
        .ok_or("no userData folder: pass --user-data or set FUNSYNC_USER_DATA")?;
    if !user_data.is_dir() {
        return Err(format!(
            "userData folder not found: {}",
            user_data.display()
        ));
    }
    let listener = server::bind(opts.port)
        .map_err(|e| format!("can't listen on 127.0.0.1:{}: {e}", opts.port))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    writeln!(out, "Scout report: http://127.0.0.1:{port}/")
        .and_then(|()| out.flush())
        .map_err(|e| e.to_string())?;
    server::serve(&listener, &user_data, err);
    Ok(())
}
