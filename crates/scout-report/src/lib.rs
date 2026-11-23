//! `npm run scout:report`: the swipe deck (`/swipe`; `/` redirects there) over the
//! EroScripts Discovery scout's waiting list (`npm run eroscripts:scout`), the scenes the
//! site scouts found (SexLikeReal, VRBangers / VRConk, PMVHaven, FapTap) and the posts of the
//! creators you said yes to on the Creators site (crate::creators), in three tabs:
//! Multi, VR and Flat, a fourth, Free space: library videos to delete when a drive is
//! full (crate::free_space), and a fifth, Duplicates: pairs of library videos that may be one
//! video, side by side, to keep one of (crate::duplicates); both through the running app's
//! backend. A bar under the tabs shows the library drive's free space; while it's full the
//! three download tabs take no swipes (crate::drive). The Downloads page (`/downloads`,
//! crate::downloads) lists every right swipe: waiting, downloaded lately, or turned down.
//! Reads `eroscripts-scout-state.json`, `eroscripts-scout.json`,
//! `eroscripts-scout-frames.json`, `slr-scout.json`, `vrb-scout.json`,
//! `pmvhaven-scout.json`, `faptap-scout.json`, `creators.json` and `config.json` in userData on every
//! request (the Duplicates tab also `scene-frame-cache.json`, `scene-probe-cache.json`,
//! `vr-dims-cache.json` and `eroscripts-links.json`), and serves the scouts' preview frames
//! (`scout-frames/`) and the creators' pictures (`creator-pictures/`, crate::creator_pictures).
//! The only files it writes are the deck's swipes, one per source:
//! `eroscripts-scout-swipes.json`, `slr-scout-swipes.json`, `vrb-scout-swipes.json`,
//! `pmvhaven-scout-swipes.json` and `faptap-scout-swipes.json` (which the scouts' `--fetch`
//! reads), the `posts` of `creator-swipes.json` (which `creators:scout --fetch` reads; the
//! Creators site writes its `creators`), `free-space-swipes.json` (the Free space tab's marks) and
//! `duplicate-swipes.json` (the Duplicates tab's); the previous copy of each kept as `.bak`.
//! Videos are deleted only by the app, through its backend. Bound to 127.0.0.1, or to every
//! interface with `--lan`; other devices need the access PIN when one is set
//! (`access-pin.json`, crate::gate).

pub mod cli;
pub mod creator_pictures;
pub mod creators;
pub mod deck;
pub mod downloads;
pub mod drive;
pub mod duplicates;
pub mod frames;
pub mod free_space;
pub mod gate;
pub mod library;
pub mod report;
pub mod scenes;
pub mod server;
pub mod swipes;

use std::ffi::OsString;
use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;

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
    let (listener, user_data) = listen(opts, "Scout report", out)?;
    server::serve(&listener, &user_data, err);
    Ok(())
}

/// A site's start (scout-report, creator-report): the userData folder (`--user-data`,
/// else the default; it must exist), the listener (`--port`, `--lan`), and the banner
/// on `out`, `<title>: http://127.0.0.1:<port>/` first.
pub fn listen(
    opts: cli::Options,
    title: &str,
    out: &mut dyn Write,
) -> Result<(TcpListener, PathBuf), String> {
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
    let listener = server::bind(opts.port, opts.lan).map_err(|e| {
        let host = if opts.lan { "0.0.0.0" } else { "127.0.0.1" };
        format!("can't listen on {host}:{}: {e}", opts.port)
    })?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let mut banner = format!("{title}: http://127.0.0.1:{port}/\n");
    if opts.lan {
        match server::lan_address() {
            Some(ip) => banner.push_str(&format!("On the home network: http://{ip}:{port}/\n")),
            None => banner.push_str(&format!(
                "On the home network: port {port} on every interface (no network address found)\n"
            )),
        }
    }
    out.write_all(banner.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|e| e.to_string())?;
    Ok((listener, user_data))
}
