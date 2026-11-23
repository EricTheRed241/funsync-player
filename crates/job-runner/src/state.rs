//! Files under `<userData>/job-runner/`:
//! - `<job>.json`: the job's last run, written only by the process holding
//!   `<job>.lock` (the run).
//! - `<job>.lock`: held while the job runs (its pid).
//! - `requests/<job>`: `job-runner start <job>` asks the scheduler to run the
//!   job now; the scheduler removes it when it handles it.
//! - `logs/<job>/<YYYYMMDD-HHMMSS>.log`: one per run, the last 72 kept (three
//!   days of an hourly job).
//! - `serve.lock`: the scheduler (one per data folder).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const KEEP_LOGS: usize = 72;

pub fn dir(user_data: &Path) -> PathBuf {
    user_data.join("job-runner")
}

pub fn lock_path(user_data: &Path, job: &str) -> PathBuf {
    dir(user_data).join(format!("{job}.lock"))
}

pub fn log_dir(user_data: &Path, job: &str) -> PathBuf {
    dir(user_data).join("logs").join(job)
}

fn request_path(user_data: &Path, job: &str) -> PathBuf {
    dir(user_data).join("requests").join(job)
}

/// Ask the scheduler to start `job` on its next tick.
pub fn write_request(user_data: &Path, job: &str) -> Result<(), String> {
    let path = request_path(user_data, job);
    let parent = path.parent().expect("requests/<job> has a parent");
    fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    fs::write(&path, "").map_err(|e| format!("{}: {e}", path.display()))
}

/// Whether `job` was requested; the request is consumed.
pub fn take_request(user_data: &Path, job: &str) -> bool {
    fs::remove_file(request_path(user_data, job)).is_ok()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JobState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<Run>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// `requested` (a `start` request), `scheduled` (the job's schedule),
    /// both through the scheduler, or `terminal` (`run` in a terminal).
    pub trigger: String,
    pub started: i64,
    /// None while it runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    /// `running`, `done`, `work left`, `failed`, `login expired`,
    /// `timed out`, `library missing`.
    pub result: String,
    pub log: String,
}

pub fn read(user_data: &Path, job: &str) -> JobState {
    fs::read_to_string(dir(user_data).join(format!("{job}.json")))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn write(user_data: &Path, job: &str, state: &JobState) -> Result<(), String> {
    let text = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    write_atomic(&dir(user_data).join(format!("{job}.json")), &text)
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, format!("{text}\n")).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The newest log of a job.
pub fn latest_log(user_data: &Path, job: &str) -> Option<PathBuf> {
    logs(user_data, job).pop()
}

/// A job's logs, oldest first (their names sort by time).
fn logs(user_data: &Path, job: &str) -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = fs::read_dir(log_dir(user_data, job))
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "log"))
                .collect()
        })
        .unwrap_or_default();
    list.sort();
    list
}

/// Delete all but the newest [`KEEP_LOGS`] logs of a job.
pub fn prune_logs(user_data: &Path, job: &str) {
    let list = logs(user_data, job);
    for old in &list[..list.len().saturating_sub(KEEP_LOGS)] {
        let _ = fs::remove_file(old);
    }
}
