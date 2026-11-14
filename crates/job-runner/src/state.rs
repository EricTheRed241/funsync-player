//! Files under `<userData>/job-runner/`:
//! - `<job>.json`: the job's last run and the last scheduled time it handled,
//!   written only by the process holding `<job>.lock` (the run).
//! - `<job>.lock`: held while the job runs (`<pid>@<host>`).
//! - `control.json`: `{"paused": {"<job>": true|false}}`, written by
//!   pause / resume; a job not listed follows its `paused` default.
//! - `logs/<job>/<YYYYMMDD-HHMMSS>.log`: one per run, the last 20 kept.
//! - `serve.lock`: the scheduler (one per data folder).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const KEEP_LOGS: usize = 20;

pub fn dir(user_data: &Path) -> PathBuf {
    user_data.join("job-runner")
}

pub fn lock_path(user_data: &Path, job: &str) -> PathBuf {
    dir(user_data).join(format!("{job}.lock"))
}

pub fn log_dir(user_data: &Path, job: &str) -> PathBuf {
    dir(user_data).join("logs").join(job)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobState {
    /// The last scheduled time (Unix seconds) a run was started for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_due: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<Run>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    /// `schedule` or `manual`.
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

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Control {
    #[serde(default)]
    pub paused: BTreeMap<String, bool>,
}

pub fn read_control(user_data: &Path) -> Control {
    fs::read_to_string(dir(user_data).join("control.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn write_control(user_data: &Path, control: &Control) -> Result<(), String> {
    let text = serde_json::to_string_pretty(control).map_err(|e| e.to_string())?;
    write_atomic(&dir(user_data).join("control.json"), &text)
}

impl Control {
    pub fn is_paused(&self, job: &str, by_default: bool) -> bool {
        self.paused.get(job).copied().unwrap_or(by_default)
    }
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
