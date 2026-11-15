//! `npm run jobs -- <command>`: FunSync's scheduled jobs on this Mac
//! (`deploy/jobs.json`), run by a scheduler the desktop app starts.

pub mod cron;
pub mod exec;
pub mod jobs;
pub mod serve;
pub mod state;

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "\
Usage: npm run jobs -- <command> [options]
FunSync's scheduled jobs on this Mac (deploy/jobs.json). The desktop app runs
the scheduler, so jobs get the app's access to Documents and the library drive.
  status               every job: schedule, paused, running or its last run, next run
  run <job>            run a job now in this terminal and wait for it (exit code: the job's)
  start <job>          have the app's scheduler run a job now (it starts within 20 s,
                       outside this terminal, paused or not; exit 1 when the scheduler isn't running)
  pause <job|all>      stop scheduling a job (a run in progress finishes)
  resume <job|all>     schedule it again (all: the jobs not paused by default)
  logs <job>           print the job's latest log
  serve                the scheduler (the desktop app starts it; one per data folder)
Options:
  --user-data <dir>    app data folder (default: $FUNSYNC_USER_DATA, else the desktop app's)
  --jobs <file>        job definitions instead of the built-in deploy/jobs.json
Exit codes: 0 done, 1 error, 2 work left or already running, 3 a login expired.
Files: <userData>/job-runner/ (state, locks, control.json, logs/<job>/).
";

struct Options {
    command: String,
    target: Option<String>,
    user_data: Option<PathBuf>,
    jobs_file: Option<PathBuf>,
    due: Option<i64>,
}

fn parse(args: &[OsString]) -> Result<Option<Options>, String> {
    let mut positional: Vec<String> = Vec::new();
    let mut user_data = None;
    let mut jobs_file = None;
    let mut due = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let a = a.to_string_lossy();
        let mut value = |flag: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match a.as_ref() {
            "-h" | "--help" => return Ok(None),
            "--user-data" => user_data = Some(PathBuf::from(value("--user-data")?)),
            "--jobs" => jobs_file = Some(PathBuf::from(value("--jobs")?)),
            // Set by the scheduler: the scheduled time this run is for.
            "--due" => {
                let v = value("--due")?;
                due = Some(
                    v.to_string_lossy()
                        .parse()
                        .map_err(|_| "--due takes Unix seconds".to_string())?,
                );
            }
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            s => positional.push(s.to_string()),
        }
    }
    let mut positional = positional.into_iter();
    let Some(command) = positional.next() else {
        return Ok(None);
    };
    let target = positional.next();
    if let Some(extra) = positional.next() {
        return Err(format!("unexpected argument {extra}"));
    }
    Ok(Some(Options {
        command,
        target,
        user_data,
        jobs_file,
        due,
    }))
}

pub fn run(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let opts = match parse(args) {
        Ok(Some(o)) => o,
        Ok(None) => {
            let _ = write!(out, "{USAGE}");
            return 0;
        }
        Err(e) => {
            let _ = writeln!(err, "{e}\n\n{USAGE}");
            return 1;
        }
    };
    let Some(user_data) = opts
        .user_data
        .clone()
        .or_else(video_tags::host::default_user_data)
    else {
        let _ = writeln!(err, "No data folder: pass --user-data <dir>.");
        return 1;
    };
    let text = match &opts.jobs_file {
        Some(f) => match std::fs::read_to_string(f) {
            Ok(t) => t,
            Err(e) => {
                let _ = writeln!(err, "{}: {e}", f.display());
                return 1;
            }
        },
        None => jobs::BUILT_IN.to_string(),
    };
    let jobs = match jobs::parse(&text) {
        Ok(j) => j,
        Err(e) => {
            let _ = writeln!(err, "{e}");
            return 1;
        }
    };
    let find = |name: &Option<String>| -> Result<&jobs::Job, String> {
        let names: Vec<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
        let name = name
            .as_deref()
            .ok_or_else(|| format!("name a job: {}", names.join(", ")))?;
        jobs.iter()
            .find(|j| j.name == name)
            .ok_or_else(|| format!("unknown job {name} ({})", names.join(", ")))
    };
    let result = match opts.command.as_str() {
        "status" => {
            status(&user_data, &jobs, out);
            Ok(0)
        }
        "run" => find(&opts.target).map(|job| {
            let trigger = match opts.due {
                Some(due) => exec::Trigger::Schedule(due),
                None => exec::Trigger::Manual,
            };
            exec::run(&user_data, job, trigger, out)
        }),
        "start" => find(&opts.target).and_then(|job| {
            if exec::holder_at(&state::dir(&user_data).join("serve.lock")).is_none() {
                return Err("The scheduler isn't running (the desktop app starts it); `run` runs a job here instead.".into());
            }
            if let Some(h) = exec::holder(&user_data, &job.name) {
                let _ = writeln!(out, "{} is already running ({h}).", job.name);
                return Ok(2);
            }
            state::write_request(&user_data, &job.name)?;
            let _ = writeln!(out, "{}: start requested; the scheduler starts it within 20 s.", job.name);
            Ok(0)
        }),
        "pause" | "resume" => set_paused(&user_data, &jobs, &opts, out),
        "logs" => find(&opts.target).and_then(|job| {
            let path = state::latest_log(&user_data, &job.name)
                .ok_or_else(|| format!("{} has no log yet", job.name))?;
            let text =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let _ = writeln!(out, "{}", path.display());
            let _ = out.write_all(text.as_bytes());
            Ok(0)
        }),
        "serve" => Ok(serve::serve(
            &user_data,
            &jobs,
            opts.jobs_file.as_deref(),
            out,
        )),
        other => Err(format!("unknown command {other}\n\n{USAGE}")),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            let _ = writeln!(err, "{e}");
            1
        }
    }
}

fn set_paused(
    user_data: &Path,
    jobs: &[jobs::Job],
    opts: &Options,
    out: &mut dyn Write,
) -> Result<i32, String> {
    let pause = opts.command == "pause";
    let target = opts.target.as_deref().ok_or("name a job, or all")?;
    let list: Vec<&jobs::Job> = if target == "all" {
        // Resuming all leaves the jobs that are paused by default alone.
        jobs.iter()
            .filter(|j| pause || !j.paused_by_default)
            .collect()
    } else {
        vec![
            jobs.iter()
                .find(|j| j.name == target)
                .ok_or_else(|| format!("unknown job {target}"))?,
        ]
    };
    let mut control = state::read_control(user_data);
    for job in list {
        control.paused.insert(job.name.clone(), pause);
        let _ = writeln!(
            out,
            "{}: {}",
            job.name,
            if pause { "paused" } else { "scheduled" }
        );
    }
    state::write_control(user_data, &control)?;
    Ok(0)
}

fn status(user_data: &Path, jobs: &[jobs::Job], out: &mut dyn Write) {
    let control = state::read_control(user_data);
    let now = exec::now();
    let serving = exec::holder_at(&state::dir(user_data).join("serve.lock"));
    let _ = match serving {
        Some(h) => writeln!(out, "Scheduler: running ({h})"),
        None => writeln!(out, "Scheduler: not running (the desktop app starts it)"),
    };
    match exec::missing_sources(user_data) {
        Ok(missing) if !missing.is_empty() => {
            let list: Vec<String> = missing.iter().map(|p| p.display().to_string()).collect();
            let _ = writeln!(out, "Library missing, jobs won't run: {}", list.join(", "));
        }
        Err(e) => {
            let _ = writeln!(out, "Library sources unreadable: {e}");
        }
        Ok(_) => {}
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{:<18} {:<13} {:<26} {:<44} next",
        "job", "schedule", "now", "last run"
    );
    for job in jobs {
        let paused = control.is_paused(&job.name, job.paused_by_default);
        let st = state::read(user_data, &job.name);
        let now_col = match exec::holder(user_data, &job.name) {
            Some(h) => {
                let since = st
                    .last
                    .as_ref()
                    .map(|r| cron::fmt_local(r.started))
                    .unwrap_or_default();
                format!("running since {since} ({})", h.pid)
            }
            None if paused => "paused".into(),
            None => "idle".into(),
        };
        let held = exec::holder(user_data, &job.name).is_some();
        let last = match &st.last {
            Some(r) => {
                let took = r
                    .ended
                    .map(|e| format!(", {}", minutes(e - r.started)))
                    .unwrap_or_default();
                let exit = r.exit.map(|c| format!(" (exit {c})")).unwrap_or_default();
                // A run whose process died (killed, power cut) never wrote its end.
                let result = if r.ended.is_none() && !held {
                    "interrupted"
                } else {
                    r.result.as_str()
                };
                format!(
                    "{} {}{took}: {result}{exit}",
                    cron::fmt_local(r.started),
                    r.trigger,
                )
            }
            None => "never".into(),
        };
        let next = if paused {
            "-".into()
        } else {
            job.schedule
                .next(now, cron::local)
                .map(cron::fmt_local)
                .unwrap_or_default()
        };
        let _ = writeln!(
            out,
            "{:<18} {:<13} {:<26} {:<44} {next}",
            job.name, job.schedule_text, now_col, last
        );
    }
}

fn minutes(secs: i64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h{:02}m", secs / 3600, secs % 3600 / 60)
    }
}
