//! End to end through the binary: a temp userData whose config.json names a
//! library folder, and job definitions whose steps are shell commands.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "job-runner-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// userData with a config.json whose library source is `<tmp>/library`
/// (created when `library` is true), and a jobs file.
fn setup(tmp: &Path, library: bool, jobs: &str) -> (PathBuf, PathBuf) {
    let user_data = tmp.join("userData");
    let lib = tmp.join("library");
    fs::create_dir_all(&user_data).unwrap();
    if library {
        fs::create_dir_all(&lib).unwrap();
    }
    let config = serde_json::json!({"settings": {"library": {"sources": [{"path": lib}]}}});
    fs::write(user_data.join("config.json"), config.to_string()).unwrap();
    let jobs_file = tmp.join("jobs.json");
    fs::write(&jobs_file, format!(r#"{{"jobs":[{jobs}]}}"#)).unwrap();
    (user_data, jobs_file)
}

fn job(name: &str, timeout: &str, steps: &[&str]) -> String {
    let steps: Vec<String> = steps
        .iter()
        .map(|s| serde_json::to_string(&["sh", "-c", s]).unwrap())
        .collect();
    format!(
        r#"{{"name":"{name}","timeout":"{timeout}","steps":[{}]}}"#,
        steps.join(",")
    )
}

fn cli(user_data: &Path, jobs: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_job-runner"))
        .args(args)
        .arg("--user-data")
        .arg(user_data)
        .arg("--jobs")
        .arg(jobs)
        .output()
        .unwrap()
}

fn state(user_data: &Path, job: &str) -> Value {
    serde_json::from_str(
        &fs::read_to_string(user_data.join(format!("job-runner/{job}.json"))).unwrap(),
    )
    .unwrap()
}

#[test]
fn runs_every_step_and_reports_work_left() {
    let tmp = TempDir::new();
    let marker = tmp.0.join("second-ran");
    let second = format!("echo second; touch '{}'; exit 0", marker.display());
    let (ud, jobs) = setup(
        &tmp.0,
        true,
        &job("j", "1h", &["echo first; exit 2", &second]),
    );
    let out = cli(&ud, &jobs, &["run", "j"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(marker.exists(), "a step leaving work doesn't stop the next");
    let st = state(&ud, "j");
    assert_eq!(st["last"]["result"], "work left");
    assert_eq!(st["last"]["trigger"], "terminal");
    let log = fs::read_to_string(st["last"]["log"].as_str().unwrap()).unwrap();
    assert!(log.contains("first\n") && log.contains("second\n"), "{log}");
    assert!(
        !ud.join("job-runner/j.lock").exists(),
        "the lock is released"
    );
}

#[test]
fn a_failing_step_decides_the_exit_code() {
    let tmp = TempDir::new();
    let (ud, jobs) = setup(&tmp.0, true, &job("j", "1h", &["exit 3", "exit 2"]));
    assert_eq!(cli(&ud, &jobs, &["run", "j"]).status.code(), Some(3));
    assert_eq!(state(&ud, "j")["last"]["result"], "login expired");
}

#[test]
fn never_runs_against_a_missing_library() {
    let tmp = TempDir::new();
    let marker = tmp.0.join("ran");
    let (ud, jobs) = setup(
        &tmp.0,
        false,
        &job("j", "1h", &[&format!("touch '{}'", marker.display())]),
    );
    let out = cli(&ud, &jobs, &["run", "j"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(!marker.exists());
    assert_eq!(state(&ud, "j")["last"]["result"], "library missing");
    let status = String::from_utf8_lossy(&cli(&ud, &jobs, &["status"]).stdout).into_owned();
    assert!(status.contains("Library missing"), "{status}");
}

#[test]
fn a_running_job_isnt_started_twice() {
    let tmp = TempDir::new();
    let (ud, jobs) = setup(&tmp.0, true, &job("j", "1h", &["exit 0"]));
    fs::create_dir_all(ud.join("job-runner")).unwrap();
    // Held by this (live) test process.
    fs::write(ud.join("job-runner/j.lock"), std::process::id().to_string()).unwrap();
    let out = cli(&ud, &jobs, &["run", "j"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).contains("already running"));
}

#[test]
fn a_run_past_its_timeout_is_stopped() {
    let tmp = TempDir::new();
    let marker = tmp.0.join("after");
    let steps = ["sleep 30", &format!("touch '{}'", marker.display())];
    let (ud, jobs) = setup(&tmp.0, true, &job("j", "1s", &steps));
    let t = Instant::now();
    let out = cli(&ud, &jobs, &["run", "j"]);
    assert!(
        t.elapsed() < Duration::from_secs(10),
        "took {:?}",
        t.elapsed()
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(!marker.exists(), "no step runs after a timeout");
    assert_eq!(state(&ud, "j")["last"]["result"], "timed out");
}

#[test]
fn start_has_the_scheduler_run_the_job() {
    let tmp = TempDir::new();
    let marker = tmp.0.join("ran");
    let step = format!("touch '{}'", marker.display());
    let (ud, jobs) = setup(&tmp.0, true, &job("j", "1h", &[&step]));
    // No scheduler: nothing would pick the request up.
    assert_eq!(cli(&ud, &jobs, &["start", "j"]).status.code(), Some(1));

    let mut serve = Command::new(env!("CARGO_BIN_EXE_job-runner"))
        .args(["serve", "--user-data"])
        .arg(&ud)
        .arg("--jobs")
        .arg(&jobs)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let wait_for = |what: &Path, secs: u64| {
        let until = Instant::now() + Duration::from_secs(secs);
        while !what.exists() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(100));
        }
        what.exists()
    };
    assert!(wait_for(&ud.join("job-runner/serve.lock"), 10));
    let out = cli(&ud, &jobs, &["start", "j"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let ran = wait_for(&marker, 30);
    let _ = serve.kill();
    let _ = serve.wait();
    assert!(ran, "the scheduler ran the requested job");
    assert_eq!(state(&ud, "j")["last"]["trigger"], "requested");
    assert!(
        !ud.join("job-runner/requests/j").exists(),
        "the request is used up"
    );
}
