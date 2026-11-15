//! The scheduler the desktop app starts (`job-runner serve`): every 20 s it
//! starts the jobs that are due, and those asked for with `job-runner start`
//! (`requests/<job>`), as `job-runner run <job> [--due <time>]`, each in its
//! own process group, so a run carries on when the app (and with it the
//! scheduler) quits or restarts, or when whatever asked for it goes away (an
//! Orca automation's agent session). Started from the app, the runs get the
//! app's macOS permissions (Documents, the library drive).

use crate::cron;
use crate::exec;
use crate::jobs::Job;
use crate::state;
use std::collections::HashMap;
use std::io::Write;
use std::os::unix::process::{CommandExt, parent_id};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;
use video_tags::host::{self, LockError};

const TICK: Duration = Duration::from_secs(20);

/// The scheduled time to start `job` for now, if any: the latest one within
/// its catch-up window that no run has been started for, unless the job is
/// paused or running (it starts once the run ends, if still within the
/// window).
pub fn due_to_start(
    job: &Job,
    last_due: Option<i64>,
    now: i64,
    paused: bool,
    running: bool,
) -> Option<i64> {
    due_with(job, last_due, now, paused, running, cron::local)
}

fn due_with(
    job: &Job,
    last_due: Option<i64>,
    now: i64,
    paused: bool,
    running: bool,
    to_civil: impl Fn(i64) -> cron::Civil,
) -> Option<i64> {
    if paused || running {
        return None;
    }
    let due = job.schedule.prev(now, job.catch_up, to_civil)?;
    (last_due < Some(due)).then_some(due)
}

pub fn serve(user_data: &Path, jobs: &[Job], jobs_file: Option<&Path>, out: &mut dyn Write) -> i32 {
    if let Err(e) = std::fs::create_dir_all(state::dir(user_data)) {
        let _ = writeln!(out, "{e}");
        return 1;
    }
    let _lock = match host::take_lock(&state::dir(user_data).join("serve.lock")) {
        Ok(lock) => lock,
        Err(LockError::Held(h)) => {
            let _ = writeln!(out, "Another scheduler runs for this data folder ({h}).");
            return 2;
        }
        Err(LockError::Io(e)) => {
            let _ = writeln!(out, "{e}");
            return 1;
        }
    };
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            let _ = writeln!(out, "{e}");
            return 1;
        }
    };
    let names: Vec<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
    let _ = writeln!(out, "Scheduling {}.", names.join(", "));
    let parent = parent_id();
    let mut started: HashMap<String, Child> = HashMap::new();
    loop {
        // The app quit or crashed: stop scheduling (runs carry on).
        if parent_id() != parent {
            let _ = writeln!(out, "The app is gone; the scheduler stops.");
            return 0;
        }
        started.retain(|name, child| match child.try_wait() {
            Ok(None) => true,
            Ok(Some(status)) => {
                let code = status.code().unwrap_or(1);
                let _ = writeln!(out, "{name}: exit {code} ({})", exec::result_name(code));
                false
            }
            Err(_) => false,
        });
        let control = state::read_control(user_data);
        let now = exec::now();
        for job in jobs {
            let running =
                started.contains_key(&job.name) || exec::holder(user_data, &job.name).is_some();
            // `job-runner start <job>` (an Orca automation, a terminal): run it now, paused or not.
            let requested = state::take_request(user_data, &job.name);
            let due = if requested {
                if running {
                    let _ = writeln!(
                        out,
                        "{}: start requested, but it's already running",
                        job.name
                    );
                    continue;
                }
                None
            } else {
                let paused = control.is_paused(&job.name, job.paused_by_default);
                let last_due = state::read(user_data, &job.name).last_due;
                match due_to_start(job, last_due, now, paused, running) {
                    Some(due) => Some(due),
                    None => continue,
                }
            };
            let mut cmd = Command::new(&exe);
            cmd.arg("run").arg(&job.name);
            if let Some(due) = due {
                cmd.arg("--due").arg(due.to_string());
            }
            cmd.arg("--user-data").arg(user_data);
            if let Some(f) = jobs_file {
                cmd.arg("--jobs").arg(f);
            }
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0);
            match cmd.spawn() {
                Ok(child) => {
                    let why = match due {
                        Some(due) => format!("scheduled {}", cron::fmt_local(due)),
                        None => "requested".into(),
                    };
                    let _ = writeln!(out, "{}: starting ({why})", job.name);
                    started.insert(job.name.clone(), child);
                }
                Err(e) => {
                    let _ = writeln!(out, "{}: can't start: {e}", job.name);
                }
            }
        }
        let _ = out.flush();
        thread::sleep(TICK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs;

    fn job(schedule: &str, catch_up: &str) -> Job {
        let text = format!(
            r#"{{"jobs":[{{"name":"j","schedule":"{schedule}","catchUp":"{catch_up}","timeout":"1h","steps":[["true"]]}}]}}"#
        );
        jobs::parse(&text).unwrap().remove(0)
    }

    /// UTC, so the tests don't depend on the machine's zone.
    fn utc(t: i64) -> cron::Civil {
        let days = t.div_euclid(86_400);
        let rem = t.rem_euclid(86_400);
        cron::Civil {
            year: 2026,
            month: 11,
            day: (14 + days - DAY0) as u32,
            hour: (rem / 3600) as u32,
            minute: (rem % 3600 / 60) as u32,
            weekday: (days + 4).rem_euclid(7) as u32,
        }
    }

    /// 2026-11-14 00:00 UTC, in days.
    const DAY0: i64 = 1_794_614_400 / 86_400;
    const NOV14: i64 = 1_794_614_400;

    #[test]
    fn starts_once_per_scheduled_time() {
        let j = job("0 19 * * *", "20h");
        let due = NOV14 + 19 * 3600;
        assert_eq!(due_with(&j, None, due + 30, false, false, utc), Some(due));
        assert_eq!(due_with(&j, Some(due), due + 600, false, false, utc), None);
        // The next day's run.
        assert_eq!(
            due_with(&j, Some(due), due + 86_400, false, false, utc),
            Some(due + 86_400)
        );
    }

    #[test]
    fn a_run_missed_while_the_app_was_closed_starts_within_the_catch_up_window() {
        let j = job("0 19 * * *", "20h");
        let due = NOV14 + 19 * 3600;
        // Opened at 14:00 the next day: 19 h late, still caught up.
        assert_eq!(
            due_with(&j, Some(due - 86_400), due + 19 * 3600, false, false, utc),
            Some(due)
        );
        // Opened at 16:00 the next day: 21 h late, skipped; the 19:00 run comes.
        assert_eq!(
            due_with(&j, Some(due - 86_400), due + 21 * 3600, false, false, utc),
            None
        );
    }

    #[test]
    fn paused_or_running_jobs_wait() {
        let j = job("5 0 * * *", "12h");
        let due = NOV14 + 300;
        assert_eq!(due_with(&j, None, due + 60, true, false, utc), None);
        assert_eq!(due_with(&j, None, due + 60, false, true, utc), None);
        // A run that ended 3 h later: the missed time still starts.
        assert_eq!(
            due_with(&j, None, due + 3 * 3600, false, false, utc),
            Some(due)
        );
    }
}
