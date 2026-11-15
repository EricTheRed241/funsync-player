//! The scheduler the desktop app starts (`job-runner serve`): every 20 s it
//! starts the jobs asked for with `job-runner start` (`requests/<job>`; the
//! Orca automations send them), as `job-runner run <job> --requested`, each in
//! its own process group, so a run carries on when the app (and with it the
//! scheduler) quits or restarts, or when whatever asked for it goes away (an
//! Orca automation's agent session). Started from the app, the runs get the
//! app's macOS permissions (Documents, the library drive).

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
    let _ = writeln!(out, "Waiting for start requests: {}.", names.join(", "));
    let parent = parent_id();
    let mut started: HashMap<String, Child> = HashMap::new();
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
        for job in jobs {
            if !state::take_request(user_data, &job.name) {
                continue;
            }
            if started.contains_key(&job.name) || exec::holder(user_data, &job.name).is_some() {
                let _ = writeln!(
                    out,
                    "{}: start requested, but it's already running",
                    job.name
                );
                continue;
            }
            let mut cmd = Command::new(&exe);
            cmd.arg("run")
                .arg(&job.name)
                .arg("--requested")
                .arg("--user-data")
                .arg(user_data);
            if let Some(f) = jobs_file {
                cmd.arg("--jobs").arg(f);
            }
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0);
            match cmd.spawn() {
                Ok(child) => {
                    let _ = writeln!(out, "{}: starting", job.name);
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
