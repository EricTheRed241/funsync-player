//! `npm run access:pin`: set, clear or show the access PIN in
//! `<userData>/access-pin.json` (the format and tokens are
//! video_tags::access). Setting a PIN always makes a new random secret, which
//! logs every device out. The file is written atomically with mode 0600.

pub mod cli;

use cli::Command;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use video_tags::access::{self, AccessPin, SECRET_LEN};
use video_tags::{host, timefmt};

/// Run the CLI; returns the exit code.
pub fn run(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match run_inner(args, out) {
        Ok(()) => 0,
        Err(msg) => {
            let _ = writeln!(err, "access-pin: {msg}");
            1
        }
    }
}

fn run_inner(args: &[OsString], out: &mut dyn Write) -> Result<(), String> {
    let opts = cli::parse(args)?;
    let Some(command) = opts.command.filter(|_| !opts.help) else {
        return out.write_all(cli::USAGE.as_bytes()).map_err(output);
    };
    let user_data = match opts.user_data {
        Some(dir) => dir,
        None => {
            host::default_user_data().ok_or("can't find the userData folder; pass --user-data")?
        }
    };
    if !user_data.is_dir() {
        return Err(format!("{} is not a folder", user_data.display()));
    }
    let path = access::file_path(&user_data);
    let text = match command {
        Command::Set(pin) => {
            let pin = AccessPin::new(&pin, random_secret()?, &timefmt::now_iso())?;
            write_atomic(&user_data, &pin)?;
            format!(
                "Access PIN set in {}.\n\
                 Other devices on the home network now need it on every FunSync site; a login\n\
                 lasts 30 days. Every device that was logged in has to log in again. This Mac\n\
                 never needs it.\n",
                path.display()
            )
        }
        Command::Clear => match fs::remove_file(&path) {
            Ok(()) => format!(
                "Access PIN removed ({} deleted): FunSync sites on the home network are open\n\
                 to every device again.\n",
                path.display()
            ),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                "No access PIN was set; FunSync sites on the home network are open.\n".to_string()
            }
            Err(e) => return Err(format!("removing {}: {e}", path.display())),
        },
        Command::Status { show } => match AccessPin::load(&user_data) {
            Ok(Some(pin)) => {
                let mut text = format!(
                    "Access PIN set (changed {}) in {}.\n",
                    pin.changed_at,
                    path.display()
                );
                if show {
                    text.push_str(&format!("PIN: {}\n", pin.pin));
                }
                text
            }
            Ok(None) => format!(
                "No access PIN ({} doesn't exist): FunSync sites on the home network are\n\
                 open to every device.\n",
                path.display()
            ),
            Err(e) => {
                return Err(format!(
                    "{e}\nOther devices are refused until it's fixed: \
                     npm run access:pin -- set <pin> (or -- clear)."
                ));
            }
        },
    };
    out.write_all(text.as_bytes()).map_err(output)
}

fn output(e: io::Error) -> String {
    format!("output: {e}")
}

/// SECRET_LEN random bytes from the OS.
fn random_secret() -> Result<[u8; SECRET_LEN], String> {
    let mut secret = [0u8; SECRET_LEN];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut secret))
        .map_err(|e| format!("/dev/urandom: {e}"))?;
    Ok(secret)
}

/// Write the PIN file atomically (a temp file in userData, renamed over it),
/// readable and writable by this user only.
pub fn write_atomic(user_data: &Path, pin: &AccessPin) -> Result<(), String> {
    let target = access::file_path(user_data);
    let tmp = user_data.join(format!("{}.{}.tmp", access::FILE_NAME, std::process::id()));
    let result = (|| -> io::Result<()> {
        let _ = fs::remove_file(&tmp);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&tmp)?;
        file.write_all(pin.to_json().as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|e| format!("writing {}: {e}", target.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_random() {
        let (a, b) = (random_secret().unwrap(), random_secret().unwrap());
        assert_ne!(a, b);
        assert_ne!(a, [0; SECRET_LEN]);
    }
}
