//! The scheduler the desktop app starts (`job-runner serve`). Every 20 s it
//! starts the jobs whose `schedule` (jobs.json) came round, as `job-runner run
//! <job> --scheduled`, and the jobs asked for with `job-runner start`
//! (`requests/<job>`; jobs without a schedule are started this way, by Orca
//! automations), as `job-runner run <job> --requested`. Each run gets its own
//! process group, so it carries on when the app (and with it the scheduler)
//! quits or restarts, or when whatever asked for it goes away (an Orca
//! automation's agent session). Started from the app, the runs get the app's
//! macOS permissions (Documents, the library drive).
//!
//! A scheduled time that passes while the job runs (or while the scheduler is
//! down, up to [`LOOKBACK`] ago) gets one run when the job is free again, not
//! one per missed time.

use crate::exec;
use crate::jobs::Job;
use crate::localtime;
use crate::schedule::Schedule;
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

/// How far back a missed scheduled time still gets its run.
pub const LOOKBACK: i64 = 8 * 86_400;

/// The scheduled time a job should run for now, if any: the latest one within
/// [`LOOKBACK`] that its last run (`last_started`) started before. A job that
/// never ran waits for a time from `since` (the scheduler's start) on.
pub fn due(schedule: &Schedule, last_started: Option<i64>, since: i64, now: i64) -> Option<i64> {
    let due = schedule.last_due(now, LOOKBACK)?;
    match last_started {
        Some(started) if started >= due => None,
        None if due < since => None,
        _ => Some(due),
    }
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
    let since = exec::now();
    let scheduled: Vec<String> = jobs
        .iter()
        .filter_map(|j| Some(format!("{} ({})", j.name, j.schedule.as_ref()?.text())))
        .collect();
    let names: Vec<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
    if !scheduled.is_empty() {
        let _ = writeln!(out, "Scheduled: {}.", scheduled.join(", "));
    }
    let _ = writeln!(out, "Waiting for start requests: {}.", names.join(", "));
    let parent = parent_id();
    let mut started: HashMap<String, Child> = HashMap::new();
    // The scheduled time each job was last started for, so a run that fails
    // before it records itself isn't started again every tick.
    let mut fired: HashMap<String, i64> = HashMap::new();
    let spawn = |job: &Job, flag: &str| {
        let mut cmd = Command::new(&exe);
        cmd.arg("run")
            .arg(&job.name)
            .arg(flag)
            .arg("--user-data")
            .arg(user_data);
        if let Some(f) = jobs_file {
            cmd.arg("--jobs").arg(f);
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
    };
    loop {
        // The app quit or crashed: stop (runs carry on).
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
        let now = exec::now();
        for job in jobs {
            let busy = |started: &HashMap<String, Child>| {
                started.contains_key(&job.name) || exec::holder(user_data, &job.name).is_some()
            };
            if state::take_request(user_data, &job.name) {
                if busy(&started) {
                    let _ = writeln!(
                        out,
                        "{}: start requested, but it's already running",
                        job.name
                    );
                } else {
                    match spawn(job, "--requested") {
                        Ok(child) => {
                            let _ = writeln!(out, "{}: starting", job.name);
                            started.insert(job.name.clone(), child);
                        }
                        Err(e) => {
                            let _ = writeln!(out, "{}: can't start: {e}", job.name);
                        }
                    }
                }
            }
            let Some(schedule) = &job.schedule else {
                continue;
            };
            if busy(&started) {
                continue;
            }
            let last = state::read(user_data, &job.name).last.map(|r| r.started);
            let Some(at) = due(schedule, last, since, now) else {
                continue;
            };
            if fired.get(&job.name) == Some(&at) {
                continue;
            }
            fired.insert(job.name.clone(), at);
            let when = localtime::fmt_time(at);
            match spawn(job, "--scheduled") {
                Ok(child) => {
                    let _ = writeln!(out, "{}: scheduled run (due {when})", job.name);
                    started.insert(job.name.clone(), child);
                }
                Err(e) => {
                    let _ = writeln!(out, "{}: can't start its {when} run: {e}", job.name);
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

    #[test]
    fn a_scheduled_time_runs_once_and_waits_for_a_free_job() {
        let hourly = Schedule::parse("0 * * * *").unwrap();
        let now = 1_800_000_000;
        let slot = hourly.last_due(now, LOOKBACK).unwrap();
        let since = slot - 7200;
        // The last run started before the time: run.
        assert_eq!(due(&hourly, Some(slot - 1), since, now), Some(slot));
        // A run from before several missed times: one run, for the latest.
        assert_eq!(due(&hourly, Some(slot - 5 * 3600), since, now), Some(slot));
        // Already run for it (or started later, e.g. on request).
        assert_eq!(due(&hourly, Some(slot), since, now), None);
        assert_eq!(due(&hourly, Some(slot + 30), since, now), None);
    }

    #[test]
    fn a_job_that_never_ran_waits_for_a_time_after_the_start() {
        let hourly = Schedule::parse("0 * * * *").unwrap();
        let now = 1_800_000_000;
        let slot = hourly.last_due(now, LOOKBACK).unwrap();
        assert_eq!(due(&hourly, None, slot + 1, now), None);
        assert_eq!(due(&hourly, None, slot, now), Some(slot));
        assert_eq!(due(&hourly, None, slot - 600, now), Some(slot));
    }

    #[test]
    fn a_time_past_the_lookback_is_dropped() {
        let yearly = Schedule::parse("0 4 1 1 *").unwrap();
        let now = yearly.next_after(1_800_000_000).unwrap() + LOOKBACK + 60;
        assert_eq!(due(&yearly, Some(0), 0, now), None);
    }
}
