//! Flags and `--help`.

use std::ffi::OsString;
use std::path::PathBuf;

pub const USAGE: &str = "\
access-pin: the PIN every FunSync site on the home network asks other devices for
(the app's backend on 5123, the web remote, the headset server, scout:report --lan)

Usage:
  npm run access:pin -- set <pin>       Set or change the PIN (4 to 12 digits)
  npm run access:pin -- clear           Remove the PIN: the sites are open again
  npm run access:pin -- status [--show] Whether a PIN is set, and since when

Options:
  --user-data <dir>    FunSync's userData folder
                       (default $FUNSYNC_USER_DATA, else ~/Library/Application Support/funsync-player)
  --show               With status: print the PIN too
  -h, --help           This text

The PIN lives in <userData>/access-pin.json (readable by this user only), with a
random secret that signs logins. A device that enters the PIN stays logged in for 30
days, on every port of this Mac's address. Setting a PIN, even the same one, makes a
new secret, so every device has to log in again. This Mac itself (127.0.0.1) never
needs the PIN. Without the file every site is open, as before; a damaged file
refuses every other device until it's set again or cleared.

Exit codes: 0 ok, 1 error (2 and 3 are not used).
";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Set(String),
    Clear,
    Status { show: bool },
}

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub help: bool,
    pub user_data: Option<PathBuf>,
    /// None only with `help`.
    pub command: Option<Command>,
}

pub fn parse(args: &[OsString]) -> Result<Options, String> {
    let mut help = false;
    let mut user_data = None;
    let mut show = false;
    let mut words: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let arg = arg
            .to_str()
            .ok_or_else(|| format!("argument is not UTF-8: {}", arg.to_string_lossy()))?;
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v)),
            _ => (arg, None),
        };
        match flag {
            "-h" | "--help" => help = true,
            "--show" => show = true,
            "--user-data" => {
                let dir = match inline {
                    Some(v) => v.to_string(),
                    None => it
                        .next()
                        .and_then(|v| v.to_str())
                        .map(str::to_string)
                        .ok_or("--user-data needs a value")?,
                };
                user_data = Some(PathBuf::from(dir));
                continue;
            }
            f if f.starts_with('-') && f.len() > 1 => {
                return Err(format!("unknown argument {arg} (see --help)"));
            }
            _ => {
                words.push(arg.to_string());
                continue;
            }
        }
        if inline.is_some() {
            return Err(format!("{flag} takes no value"));
        }
    }
    if help {
        return Ok(Options {
            help,
            user_data,
            command: None,
        });
    }
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let command = match words.as_slice() {
        ["set", pin] => Command::Set(pin.to_string()),
        ["set"] => return Err("set needs the PIN: set <pin>".into()),
        ["clear"] => Command::Clear,
        ["status"] => Command::Status { show },
        [] => return Err("nothing to do: give set <pin>, clear or status (see --help)".into()),
        [cmd, ..] if matches!(*cmd, "set" | "clear" | "status") => {
            return Err(format!("too many arguments for {cmd} (see --help)"));
        }
        [cmd, ..] => return Err(format!("unknown command {cmd} (see --help)")),
    };
    if show && !matches!(command, Command::Status { .. }) {
        return Err("--show goes with status".into());
    }
    Ok(Options {
        help,
        user_data,
        command: Some(command),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(list: &[&str]) -> Result<Options, String> {
        parse(&list.iter().map(OsString::from).collect::<Vec<_>>())
    }

    fn command(list: &[&str]) -> Command {
        parse_strs(list).unwrap().command.unwrap()
    }

    #[test]
    fn parses_commands() {
        assert_eq!(command(&["set", "0123"]), Command::Set("0123".into()));
        assert_eq!(command(&["clear"]), Command::Clear);
        assert_eq!(command(&["status"]), Command::Status { show: false });
        assert_eq!(
            command(&["--show", "status"]),
            Command::Status { show: true }
        );
        let o = parse_strs(&["status", "--user-data", "/tmp/ud"]).unwrap();
        assert_eq!(o.user_data, Some(PathBuf::from("/tmp/ud")));
        let o = parse_strs(&["--user-data=/tmp/x", "clear"]).unwrap();
        assert_eq!(o.user_data, Some(PathBuf::from("/tmp/x")));
        let o = parse_strs(&["--help"]).unwrap();
        assert!(o.help && o.command.is_none());
        assert!(parse_strs(&["-h", "bogus"]).unwrap().help);
    }

    #[test]
    fn rejects_bad_arguments() {
        for bad in [
            &[][..],
            &["set"],
            &["set", "1", "2"],
            &["clear", "now"],
            &["unlock"],
            &["status", "--verbose"],
            &["set", "1234", "--show"],
            &["clear", "--show"],
            &["--show=yes", "status"],
            &["status", "--user-data"],
        ] {
            assert!(parse_strs(bad).is_err(), "{bad:?}");
        }
    }
}
