//! What changes when a CLI runs somewhere other than the Mac that runs the
//! app (a Kubernetes pod with userData and the library mounted at the same
//! paths): the userData folder and the app's backend URL come from the
//! environment, and lock files record the host that wrote them, since a pid
//! means something only on that host. Shared by every Rust CLI.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The userData folder when no `--user-data` is given.
pub const USER_DATA_ENV: &str = "FUNSYNC_USER_DATA";
/// Base URL of the running app's backend (`http://host:port`); flags win.
pub const BACKEND_URL_ENV: &str = "FUNSYNC_BACKEND_URL";
/// A lock written on another host counts as held until it is this old.
pub const FOREIGN_LOCK_MAX_AGE: Duration = Duration::from_secs(12 * 3600);

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

/// The app's backend: `FUNSYNC_BACKEND_URL`, else `http://127.0.0.1:<port>`.
pub fn backend_url(port: u16) -> String {
    backend_url_from(std::env::var(BACKEND_URL_ENV).ok(), port)
}

fn backend_url_from(env: Option<String>, port: u16) -> String {
    match env.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(url) => url.trim_end_matches('/').to_string(),
        None => format!("http://127.0.0.1:{port}"),
    }
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

/// This machine's host name (`gethostname`, as Node's `os.hostname()`); in a
/// pod, the pod's name.
pub fn hostname() -> String {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn gethostname(name: *mut std::ffi::c_char, len: usize) -> std::ffi::c_int;
        }
        let mut buf = [0u8; 256];
        // SAFETY: `buf` is writable for `buf.len()` bytes; gethostname writes
        // at most that many.
        if unsafe { gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            return String::from_utf8_lossy(&buf[..end]).into_owned();
        }
    }
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_default()
}

/// Whether a process with this pid runs on this host (`kill(pid, 0)`, which
/// needs no `kill` binary: a slim container image has none). Another user's
/// process (EPERM) runs too.
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

/// Who holds a lock file: `<pid>@<host>`, or a bare `<pid>` (written before
/// hosts were recorded; read as this host's).
#[derive(Debug, Clone, PartialEq)]
pub struct LockHolder {
    pub pid: u32,
    pub host: Option<String>,
}

impl LockHolder {
    /// This process on this host.
    pub fn me() -> Self {
        LockHolder {
            pid: std::process::id(),
            host: Some(hostname()),
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let (pid, host) = match text.split_once('@') {
            Some((pid, host)) => (pid, Some(host.to_string())),
            None => (text, None),
        };
        Some(LockHolder {
            pid: pid.parse().ok()?,
            host,
        })
    }

    /// The lock file's contents.
    pub fn to_text(&self) -> String {
        match &self.host {
            Some(host) => format!("{}@{host}", self.pid),
            None => self.pid.to_string(),
        }
    }

    /// Whether the lock still holds, `age` after it was written: on this
    /// host while its pid runs; on another host (whose pids can't be checked
    /// from here) until [`FOREIGN_LOCK_MAX_AGE`].
    pub fn holds(&self, age: Duration, this_host: &str, alive: impl Fn(u32) -> bool) -> bool {
        match &self.host {
            Some(host) if host != this_host => age < FOREIGN_LOCK_MAX_AGE,
            _ => alive(self.pid),
        }
    }
}

impl fmt::Display for LockHolder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.host {
            Some(host) => write!(f, "pid {} on {host}", self.pid),
            None => write!(f, "pid {}", self.pid),
        }
    }
}

/// A lock file holding `<pid>@<host>`; removed on drop.
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
    let this_host = me.host.clone().unwrap_or_default();
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
                let age = fs::metadata(path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| SystemTime::now().duration_since(t).ok())
                    .unwrap_or(Duration::ZERO);
                match holder {
                    Some(h) if h.holds(age, &this_host, pid_alive) => {
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
            user_data_from(Some("/pod/userData".into())),
            Some(PathBuf::from("/pod/userData"))
        );
        let default = user_data_from(None);
        assert!(default.is_some_and(|d| d.ends_with("funsync-player")));
        assert_eq!(user_data_from(Some("".into())), user_data_from(None));
    }

    #[test]
    fn backend_url_env_wins_over_the_local_port() {
        assert_eq!(backend_url_from(None, 5200), "http://127.0.0.1:5200");
        assert_eq!(
            backend_url_from(Some("http://funsync-app:5123/".into()), 5200),
            "http://funsync-app:5123"
        );
        assert_eq!(
            backend_url_from(Some("  ".into()), 5123),
            "http://127.0.0.1:5123"
        );
    }

    #[test]
    fn parses_http_bases() {
        assert_eq!(
            HttpBase::parse("http://host.docker.internal:5123/"),
            Ok(HttpBase {
                host: "host.docker.internal:5123".into(),
                authority: "host.docker.internal:5123".into()
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
        let h = LockHolder::parse("42@funsync-scout-29123-abcde\n").unwrap();
        assert_eq!(h.pid, 42);
        assert_eq!(h.host.as_deref(), Some("funsync-scout-29123-abcde"));
        assert_eq!(h.to_text(), "42@funsync-scout-29123-abcde");
        assert_eq!(
            LockHolder::parse("42"),
            Some(LockHolder {
                pid: 42,
                host: None
            })
        );
        assert_eq!(LockHolder::parse("not a pid"), None);
        assert_eq!(LockHolder::parse("@host"), None);
        assert_eq!(
            LockHolder::me().to_text(),
            format!("{}@{}", std::process::id(), hostname())
        );
    }

    #[test]
    fn a_foreign_lock_holds_by_age_and_a_local_one_by_pid() {
        let hour = Duration::from_secs(3600);
        let never = |_| panic!("a foreign pid is never checked here");
        let foreign = LockHolder::parse("42@mac-mini").unwrap();
        assert!(foreign.holds(Duration::ZERO, "pod-1", never));
        assert!(foreign.holds(12 * hour - Duration::from_secs(1), "pod-1", never));
        assert!(!foreign.holds(12 * hour, "pod-1", never));
        let local = LockHolder::parse("42@pod-1").unwrap();
        assert!(local.holds(100 * hour, "pod-1", |pid| pid == 42));
        assert!(!local.holds(Duration::ZERO, "pod-1", |_| false));
        // The old format is this host's: its pid decides.
        let old = LockHolder::parse("42").unwrap();
        assert!(old.holds(100 * hour, "pod-1", |_| true));
        assert!(!old.holds(Duration::ZERO, "pod-1", |_| false));
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

    /// Set a file's mtime `age` ago.
    fn age_file(path: &Path, age: Duration) {
        let f = fs::OpenOptions::new().write(true).open(path).unwrap();
        f.set_modified(SystemTime::now() - age).unwrap();
    }

    #[test]
    fn take_lock_respects_foreign_locks_until_they_expire() {
        let dir = temp("lock");
        let path = dir.join("x.lock");
        // Another host's lock, an hour old: held, left in place.
        fs::write(&path, "1@another-host-than-this-one").unwrap();
        age_file(&path, Duration::from_secs(3600));
        assert_eq!(
            take_lock(&path).unwrap_err(),
            LockError::Held(LockHolder::parse("1@another-host-than-this-one").unwrap())
        );
        assert!(path.exists());
        // 13 hours old: expired, taken over.
        age_file(&path, Duration::from_secs(13 * 3600));
        let held = take_lock(&path).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            LockHolder::me().to_text()
        );
        // Held by this live process on this host: refused.
        assert!(matches!(take_lock(&path), Err(LockError::Held(h)) if h == LockHolder::me()));
        drop(held);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
