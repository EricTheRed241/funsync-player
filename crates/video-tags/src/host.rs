//! Shared by every Rust CLI: the userData folder (`FUNSYNC_USER_DATA`, else
//! the app's), the backend's HTTP base, and pid lock files.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};

/// The userData folder when no `--user-data` is given.
pub const USER_DATA_ENV: &str = "FUNSYNC_USER_DATA";

/// `FUNSYNC_USER_DATA`, else Electron's userData folder for the app
/// (`app.getPath('userData')`).
pub fn default_user_data() -> Option<PathBuf> {
    user_data_from(std::env::var_os(USER_DATA_ENV))
}

fn user_data_from(env: Option<OsString>) -> Option<PathBuf> {
    if let Some(dir) = env.filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home()?.join("Library/Application Support")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| Some(home()?.join(".config")))?
    };
    Some(base.join("funsync-player"))
}

/// `http://host[:port][/]`, split into the `Host` header and the
/// `host:port` to connect to (port 80 when none is given).
#[derive(Debug, Clone, PartialEq)]
pub struct HttpBase {
    pub host: String,
    pub authority: String,
}

impl HttpBase {
    pub fn parse(url: &str) -> Result<Self, String> {
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| format!("{url}: only http://host:port is supported"))?;
        let host = rest.trim_end_matches('/');
        if host.is_empty() || host.contains('/') {
            return Err(format!("{url}: give http://host:port without a path"));
        }
        let authority = if host
            .rsplit_once(':')
            .is_some_and(|(_, p)| p.parse::<u16>().is_ok())
        {
            host.to_string()
        } else {
            format!("{host}:80")
        };
        Ok(HttpBase {
            host: host.to_string(),
            authority,
        })
    }

    /// The first address the host name resolves to.
    pub fn resolve(&self) -> Result<SocketAddr, String> {
        self.authority
            .to_socket_addrs()
            .map_err(|e| format!("{}: {e}", self.authority))?
            .next()
            .ok_or_else(|| format!("{}: no address", self.authority))
    }
}

/// Whether a process with this pid runs on this machine (`kill(pid, 0)`).
/// Another user's process (EPERM) runs too.
pub fn pid_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: i32, sig: std::ffi::c_int) -> std::ffi::c_int;
        }
        // 0 would signal the process group, and pids above i32::MAX don't exist.
        let Ok(pid) = i32::try_from(pid) else {
            return false;
        };
        if pid == 0 {
            return false;
        }
        // SAFETY: signal 0 only checks that the process exists.
        if unsafe { kill(pid, 0) } == 0 {
            return true;
        }
        io::Error::last_os_error().kind() == io::ErrorKind::PermissionDenied
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Who holds a lock file: a bare `<pid>`. Files from older versions hold
/// `<pid>@<host>`; the host is ignored.
#[derive(Debug, Clone, PartialEq)]
pub struct LockHolder {
    pub pid: u32,
}

impl LockHolder {
    /// This process.
    pub fn me() -> Self {
        LockHolder {
            pid: std::process::id(),
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let pid = text.split_once('@').map_or(text, |(pid, _)| pid);
        Some(LockHolder {
            pid: pid.parse().ok()?,
        })
    }

    /// The lock file's contents.
    pub fn to_text(&self) -> String {
        self.pid.to_string()
    }

    /// Whether the lock still holds: while its pid runs.
    pub fn holds(&self, alive: impl Fn(u32) -> bool) -> bool {
        alive(self.pid)
    }
}

impl fmt::Display for LockHolder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "pid {}", self.pid)
    }
}

/// A lock file holding `<pid>`; removed on drop.
#[derive(Debug)]
pub struct PidLock {
    path: PathBuf,
}

impl Drop for PidLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Debug, PartialEq)]
pub enum LockError {
    /// Another run holds it.
    Held(LockHolder),
    Io(String),
}

/// Take the lock at `path`. One that no longer holds (see
/// [`LockHolder::holds`]) or is unreadable is stale and taken over.
pub fn take_lock(path: &Path) -> Result<PidLock, LockError> {
    let me = LockHolder::me();
    for _ in 0..3 {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut f) => {
                f.write_all(me.to_text().as_bytes())
                    .map_err(|e| LockError::Io(format!("{}: {e}", path.display())))?;
                return Ok(PidLock {
                    path: path.to_path_buf(),
                });
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let holder = fs::read_to_string(path)
                    .ok()
                    .and_then(|s| LockHolder::parse(&s));
                match holder {
                    Some(h) if h.holds(pid_alive) => {
                        return Err(LockError::Held(h));
                    }
                    _ => {
                        let _ = fs::remove_file(path);
                    }
                }
            }
            Err(e) => return Err(LockError::Io(format!("{}: {e}", path.display()))),
        }
    }
    Err(LockError::Io(format!("can't take {}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_data_env_wins_over_the_app_default() {
        assert_eq!(
            user_data_from(Some("/tmp/userData".into())),
            Some(PathBuf::from("/tmp/userData"))
        );
        let default = user_data_from(None);
        assert!(default.is_some_and(|d| d.ends_with("funsync-player")));
        assert_eq!(user_data_from(Some("".into())), user_data_from(None));
    }

    #[test]
    fn parses_http_bases() {
        assert_eq!(
            HttpBase::parse("http://localhost:5123/"),
            Ok(HttpBase {
                host: "localhost:5123".into(),
                authority: "localhost:5123".into()
            })
        );
        assert_eq!(HttpBase::parse("http://app").unwrap().authority, "app:80");
        assert!(HttpBase::parse("https://app:1").is_err());
        assert!(HttpBase::parse("http://app:1/api").is_err());
        assert!(HttpBase::parse("http://").is_err());
        assert!(
            HttpBase::parse("http://127.0.0.1:5123")
                .unwrap()
                .resolve()
                .is_ok()
        );
    }

    #[test]
    fn reads_new_and_old_lock_formats() {
        assert_eq!(LockHolder::parse("42\n"), Some(LockHolder { pid: 42 }));
        assert_eq!(
            LockHolder::parse("42@mac-mini.local"),
            Some(LockHolder { pid: 42 })
        );
        assert_eq!(LockHolder::parse("not a pid"), None);
        assert_eq!(LockHolder::parse("@host"), None);
        assert_eq!(LockHolder { pid: 42 }.to_text(), "42");
        assert_eq!(LockHolder { pid: 42 }.to_string(), "pid 42");
        assert_eq!(LockHolder::me().to_text(), std::process::id().to_string());
    }

    #[test]
    fn a_lock_holds_while_its_pid_runs() {
        let h = LockHolder::parse("42@another-host").unwrap();
        assert!(h.holds(|pid| pid == 42));
        assert!(!h.holds(|_| false));
    }

    #[test]
    fn pid_alive_checks_this_hosts_processes() {
        assert!(pid_alive(std::process::id()));
        // pid 1 (init / launchd): another user's process still runs.
        assert!(pid_alive(1));
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let dead = child.id();
        child.wait().unwrap();
        assert!(!pid_alive(dead));
        assert!(!pid_alive(0));
        assert!(!pid_alive(u32::MAX));
    }

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("video-tags-host-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn take_lock_takes_over_ended_runs_and_refuses_live_ones() {
        let dir = temp("lock");
        let path = dir.join("x.lock");
        // An old-format lock of a process that has ended: taken over.
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let dead = child.id();
        child.wait().unwrap();
        fs::write(&path, format!("{dead}@another-host")).unwrap();
        let held = take_lock(&path).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            LockHolder::me().to_text()
        );
        // Held by this live process: refused.
        assert!(matches!(take_lock(&path), Err(LockError::Held(h)) if h == LockHolder::me()));
        drop(held);
        assert!(!path.exists());
        // pid 1 runs: its lock is refused and left in place.
        fs::write(&path, "1").unwrap();
        assert_eq!(
            take_lock(&path).unwrap_err(),
            LockError::Held(LockHolder { pid: 1 })
        );
        assert!(path.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
