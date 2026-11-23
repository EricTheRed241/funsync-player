//! One run of a job: its lock, the library check, the steps (with the job's
//! timeout), the log and the state.

use crate::jobs::Job;
use crate::localtime;
use crate::state::{self, Run};
use std::fs::{self, File};
use std::io::{self, IsTerminal, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use video_tags::host::{self, LockError, LockHolder};

/// After SIGTERM on timeout, SIGKILL follows this much later.
const KILL_GRACE: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Trigger {
    /// Started by the scheduler on a `start` request (an Orca automation).
    Requested,
    /// Started by the scheduler at the job's `schedule`.
    Scheduled,
    /// `run` in a terminal.
    Terminal,
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Who holds a job's lock now, if anyone.
pub fn holder(user_data: &Path, job: &str) -> Option<LockHolder> {
    holder_at(&state::lock_path(user_data, job))
}

/// Who holds the lock file at `path` now, if anyone.
pub fn holder_at(path: &Path) -> Option<LockHolder> {
    let holder = LockHolder::parse(&fs::read_to_string(path).ok()?)?;
    holder.holds(host::pid_alive).then_some(holder)
}

/// The job's exit code from its steps': the first that isn't 0 or 2 (an
/// error, 3 = a login expired), else 2 when a step left work, else 0.
pub fn combine(codes: &[i32]) -> i32 {
    if let Some(&c) = codes.iter().find(|&&c| c != 0 && c != 2) {
        return c;
    }
    if codes.contains(&2) { 2 } else { 0 }
}

pub fn result_name(code: i32) -> &'static str {
    match code {
        0 => "done",
        2 => "work left",
        3 => "login expired",
        _ => "failed",
    }
}

/// Library sources (config.json) that aren't there: an unplugged drive.
/// A job never runs against a missing library, which it would read as empty.
pub fn missing_sources(user_data: &Path) -> Result<Vec<PathBuf>, String> {
    let config = video_tags::config::read(user_data)?;
    Ok(config.sources.into_iter().filter(|p| !p.is_dir()).collect())
}

/// Run `job` now. Returns its exit code (2 also when it's already running).
pub fn run(user_data: &Path, job: &Job, trigger: Trigger, out: &mut dyn Write) -> i32 {
    let dir = state::dir(user_data);
    if let Err(e) = fs::create_dir_all(&dir) {
        let _ = writeln!(out, "{}: {e}", dir.display());
        return 1;
    }
    let _lock = match host::take_lock(&state::lock_path(user_data, &job.name)) {
        Ok(lock) => lock,
        Err(LockError::Held(h)) => {
            let _ = writeln!(out, "{} is already running ({h}).", job.name);
            return 2;
        }
        Err(LockError::Io(e)) => {
            let _ = writeln!(out, "{e}");
            return 1;
        }
    };
    let started = now();
    let mut st = state::read(user_data, &job.name);

    let log_path = match new_log(user_data, &job.name, started) {
        Ok(p) => p,
        Err(e) => {
            let _ = writeln!(out, "{e}");
            return 1;
        }
    };
    let log = match File::options().create(true).append(true).open(&log_path) {
        Ok(f) => Arc::new(Mutex::new(f)),
        Err(e) => {
            let _ = writeln!(out, "{}: {e}", log_path.display());
            return 1;
        }
    };
    let say = |out: &mut dyn Write, line: String| {
        let line = format!("[job-runner {}] {line}\n", localtime::fmt_local(now()));
        let _ = out.write_all(line.as_bytes());
        let _ = log.lock().unwrap().write_all(line.as_bytes());
    };
    let trigger_name = match trigger {
        Trigger::Requested => "requested",
        Trigger::Scheduled => "scheduled",
        Trigger::Terminal => "terminal",
    };
    st.last = Some(Run {
        trigger: trigger_name.into(),
        started,
        ended: None,
        exit: None,
        result: "running".into(),
        log: log_path.display().to_string(),
    });
    let _ = state::write(user_data, &job.name, &st);
    say(out, format!("{} ({trigger_name})", job.name));

    let (code, result) = match missing_sources(user_data) {
        Err(e) => {
            say(out, format!("can't read the library sources: {e}"));
            (1, "failed")
        }
        Ok(missing) if !missing.is_empty() => {
            let list: Vec<String> = missing.iter().map(|p| p.display().to_string()).collect();
            say(
                out,
                format!(
                    "library missing (drive unplugged?): {}; not running",
                    list.join(", ")
                ),
            );
            (1, "library missing")
        }
        Ok(_) => run_steps(user_data, job, trigger, started, &log, &say, out),
    };
    if let Some(last) = st.last.as_mut() {
        last.ended = Some(now());
        last.exit = Some(code);
        last.result = result.into();
    }
    if let Err(e) = state::write(user_data, &job.name, &st) {
        say(out, format!("can't save the job state: {e}"));
    }
    state::prune_logs(user_data, &job.name);
    code
}

fn run_steps(
    user_data: &Path,
    job: &Job,
    trigger: Trigger,
    started: i64,
    log: &Arc<Mutex<File>>,
    say: &dyn Fn(&mut dyn Write, String),
    out: &mut dyn Write,
) -> (i32, &'static str) {
    // Runs the scheduler starts own their process group, so a timeout stops
    // the whole step (npm, node, the binaries it starts). A run in a terminal
    // stays in the terminal's group, so Ctrl-C reaches the step.
    let own_group = trigger != Trigger::Terminal || !io::stdout().is_terminal();
    let deadline = started + job.timeout;
    let mut codes = Vec::new();
    for step in &job.steps {
        say(out, format!("$ {}", step.join(" ")));
        let mut cmd = Command::new(&step[0]);
        cmd.args(&step[1..])
            .env(host::USER_DATA_ENV, user_data)
            .env("FUNSYNC_CONFIG_READONLY", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if own_group {
            cmd.process_group(0);
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                say(out, format!("can't start {}: {e}", step[0]));
                codes.push(1);
                continue;
            }
        };
        let tees = [
            tee(
                child
                    .stdout
                    .take()
                    .map(|r| Box::new(r) as Box<dyn Read + Send>),
                log,
                false,
            ),
            tee(
                child
                    .stderr
                    .take()
                    .map(|r| Box::new(r) as Box<dyn Read + Send>),
                log,
                true,
            ),
        ];
        let status = wait(&mut child, deadline, own_group);
        for t in tees.into_iter().flatten() {
            let _ = t.join();
        }
        match status {
            Waited::Exited(code) => {
                say(out, format!("exit {code}"));
                codes.push(code);
            }
            Waited::TimedOut => {
                say(
                    out,
                    format!(
                        "stopped: still running after the job's timeout ({} min)",
                        job.timeout / 60
                    ),
                );
                return (1, "timed out");
            }
        }
    }
    let code = combine(&codes);
    say(
        out,
        format!("{}: exit {code} ({})", job.name, result_name(code)),
    );
    (code, result_name(code))
}

/// Copy a step's output to the log and to this process's own output.
fn tee(
    from: Option<Box<dyn Read + Send>>,
    log: &Arc<Mutex<File>>,
    to_stderr: bool,
) -> Option<thread::JoinHandle<()>> {
    let mut from = from?;
    let log = Arc::clone(log);
    Some(thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok(n) = from.read(&mut buf) {
            if n == 0 {
                break;
            }
            let _ = log.lock().unwrap().write_all(&buf[..n]);
            let _ = if to_stderr {
                io::stderr().write_all(&buf[..n])
            } else {
                io::stdout().write_all(&buf[..n])
            };
        }
    }))
}

enum Waited {
    Exited(i32),
    TimedOut,
}

fn wait(child: &mut Child, deadline: i64, own_group: bool) -> Waited {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Waited::Exited(exit_code(status)),
            Ok(None) => {}
            Err(_) => return Waited::Exited(1),
        }
        if now() >= deadline {
            let pid = child.id() as i32;
            let target = if own_group { -pid } else { pid };
            signal(target, SIGTERM);
            let until = SystemTime::now() + KILL_GRACE;
            while SystemTime::now() < until {
                if let Ok(Some(_)) = child.try_wait() {
                    return Waited::TimedOut;
                }
                thread::sleep(Duration::from_millis(200));
            }
            signal(target, SIGKILL);
            let _ = child.wait();
            return Waited::TimedOut;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

/// A step killed by a signal counts as failed (1).
fn exit_code(status: std::process::ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

const SIGTERM: i32 = 15;
const SIGKILL: i32 = 9;

fn signal(pid: i32, sig: i32) {
    unsafe extern "C" {
        fn kill(pid: i32, sig: std::ffi::c_int) -> std::ffi::c_int;
    }
    // SAFETY: kill only sends a signal; a gone pid just returns an error.
    unsafe { kill(pid, sig) };
}

/// `logs/<job>/<YYYYMMDD-HHMMSS>.log` (local time; `-2`, `-3`… when taken).
fn new_log(user_data: &Path, job: &str, at: i64) -> Result<PathBuf, String> {
    let dir = state::log_dir(user_data, job);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let c = localtime::local(at);
    let stem = format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    );
    let mut path = dir.join(format!("{stem}.log"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}-{n}.log"));
        n += 1;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_error_wins_over_work_left() {
        assert_eq!(combine(&[0, 0]), 0);
        assert_eq!(combine(&[2, 0]), 2);
        assert_eq!(combine(&[0, 2]), 2);
        assert_eq!(combine(&[2, 3]), 3);
        assert_eq!(combine(&[1, 3]), 1);
        assert_eq!(combine(&[]), 0);
    }
}
