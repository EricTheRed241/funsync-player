//! Command line: `serve` (default), `import` and `autostart`.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

pub const DEFAULT_PORT: u16 = 5300;

pub const USAGE: &str = "\
funsync-server: FunSync in Rust (library server + web UI)

Usage:
  funsync-server [serve] [--port N] [--lan] [--supervise] [--data-dir <dir>]
  funsync-server import [--from <old userData>] [--force] [--data-dir <dir>]
  funsync-server autostart install [serve flags…] | uninstall | status

serve    Serve http://127.0.0.1:<port>/ (default 5300) and scan the library
         at start. --lan listens on every interface; other devices then need
         the access PIN when the old app's access-pin.json sets one.
         --supervise also runs the local sites of deploy/sites.json and the
         job runner, as the desktop app does (a site already running is left
         alone).
import   Copy library sources, playlists, categories and ratings from the old
         app's config.json (read-only) into <data-dir>/library.json.
         Refuses to replace an earlier import without --force.
autostart  A LaunchAgent (com.funsync.rust) that runs `serve` with the given
         flags at login and again whenever it stops.

--data-dir  default $FUNSYNC_RUST_DATA, else
            ~/Library/Application Support/funsync-rust
--from      default $FUNSYNC_USER_DATA, else
            ~/Library/Application Support/funsync-player

Exit codes: 0 done, 1 error, 2 bad usage.";

#[derive(Debug, PartialEq)]
pub enum Command {
    Help,
    Serve {
        addr: SocketAddr,
        data_dir: PathBuf,
        supervise: bool,
    },
    Import {
        from: PathBuf,
        data_dir: PathBuf,
        force: bool,
    },
    Autostart {
        action: String,
        serve_args: Vec<String>,
    },
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    if args.first().map(String::as_str) == Some("autostart") {
        let action = args
            .get(1)
            .cloned()
            .ok_or("autostart needs install, uninstall or status")?;
        let serve_args = args[2..].to_vec();
        // The flags must be valid `serve` flags.
        if !matches!(parse(&serve_args)?, Command::Serve { .. }) {
            return Err("autostart takes serve flags only".into());
        }
        return Ok(Command::Autostart { action, serve_args });
    }
    let mut it = args.iter().peekable();
    let sub = match it.peek().map(|s| s.as_str()) {
        Some("serve") | Some("import") => it.next().cloned(),
        _ => None,
    };
    let mut port = DEFAULT_PORT;
    let mut lan = false;
    let mut data_dir = None;
    let mut from = None;
    let mut force = false;
    let mut supervise = false;
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().cloned().ok_or(format!("{name} needs a value"));
        match a.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "--port" => port = val("--port")?.parse().map_err(|_| "--port: not a port")?,
            "--lan" if sub.as_deref() != Some("import") => lan = true,
            "--supervise" if sub.as_deref() != Some("import") => supervise = true,
            "--data-dir" => data_dir = Some(PathBuf::from(val("--data-dir")?)),
            "--from" if sub.as_deref() == Some("import") => {
                from = Some(PathBuf::from(val("--from")?))
            }
            "--force" if sub.as_deref() == Some("import") => force = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    let data_dir = data_dir
        .or_else(crate::data::default_data_dir)
        .ok_or("no data folder: set --data-dir or HOME")?;
    if sub.as_deref() == Some("import") {
        let from = from
            .or_else(crate::data::default_old_user_data)
            .ok_or("no old data folder: set --from")?;
        return Ok(Command::Import {
            from,
            data_dir,
            force,
        });
    }
    Ok(Command::Serve {
        addr: SocketAddr::from((
            if lan {
                Ipv4Addr::UNSPECIFIED
            } else {
                Ipv4Addr::LOCALHOST
            },
            port,
        )),
        data_dir,
        supervise,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses() {
        assert_eq!(
            parse(&s(&["--port", "5301", "--data-dir", "/d"])).unwrap(),
            Command::Serve {
                addr: "127.0.0.1:5301".parse().unwrap(),
                data_dir: "/d".into(),
                supervise: false,
            }
        );
        assert_eq!(
            parse(&s(&[
                "import",
                "--from",
                "/o",
                "--force",
                "--data-dir",
                "/d"
            ]))
            .unwrap(),
            Command::Import {
                from: "/o".into(),
                data_dir: "/d".into(),
                force: true
            }
        );
        assert!(parse(&s(&["--force"])).is_err());
        assert_eq!(parse(&s(&["--help"])).unwrap(), Command::Help);
    }
}
