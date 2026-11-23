//! `npm run jobs -- <command>`: FunSync's jobs on this Mac (`deploy/jobs.json`),
//! run by a scheduler the desktop app starts: at their `schedule`, else when
//! asked with `start` (Orca automations).

pub mod exec;
pub mod jobs;
pub mod localtime;
pub mod schedule;
pub mod serve;
pub mod state;

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "\
Usage: npm run jobs -- <command> [options]
FunSync's jobs on this Mac (deploy/jobs.json), run by the scheduler the desktop
app starts, so jobs get the app's access to Documents and the library drive. A
job with a `schedule` (cron: minute hour day-of-month month day-of-week, local
time) is started by the scheduler then; the others by `start` (Orca automations).
  status               every job: its schedule and next run, running or its last run
  start <job>          have the app's scheduler run a job now (it starts within 20 s,
                       outside this terminal; exit 2 when it's already running,
                       1 when the scheduler isn't running)
  run <job>            run a job now in this terminal and wait for it (exit code: the job's)
  logs <job>           print the job's latest log
  serve                the scheduler (the desktop app starts it; one per data folder)
Options:
  --user-data <dir>    app data folder (default: $FUNSYNC_USER_DATA, else the desktop app's)
  --jobs <file>        job definitions instead of the built-in deploy/jobs.json
Exit codes: 0 done, 1 error, 2 work left or already running, 3 a login expired.
Files: <userData>/job-runner/ (state, locks, requests/, logs/<job>/).
";

struct Options {
    command: String,
    target: Option<String>,
    user_data: Option<PathBuf>,
    jobs_file: Option<PathBuf>,
    trigger: exec::Trigger,
}

fn parse(args: &[OsString]) -> Result<Option<Options>, String> {
    let mut positional: Vec<String> = Vec::new();
    let mut user_data = None;
    let mut jobs_file = None;
    let mut trigger = exec::Trigger::Terminal;
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
            // Set by the scheduler on the runs it starts.
            "--requested" => trigger = exec::Trigger::Requested,
            "--scheduled" => trigger = exec::Trigger::Scheduled,
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
        trigger,
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
        "run" => find(&opts.target).map(|job| exec::run(&user_data, job, opts.trigger, out)),
        "start" => find(&opts.target).and_then(|job| {
            if exec::holder_at(&state::dir(&user_data).join("serve.lock")).is_none() {
                return Err("The scheduler isn't running (the desktop app starts it); `run` runs a job here instead.".into());
            }
            if let Some(h) = exec::holder(&user_data, &job.name) {
                let _ = writeln!(out, "{} is already running ({h}).", job.name);
                return Ok(2);
            }
            state::write_request(&user_data, &job.name)?;
            let _ = writeln!(
                out,
                "{}: start requested; the scheduler starts it within 20 s.",
                job.name
            );
            Ok(0)
        }),
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

fn status(user_data: &Path, jobs: &[jobs::Job], out: &mut dyn Write) {
    let serving = exec::holder_at(&state::dir(user_data).join("serve.lock"));
    let _ = match serving {
        Some(h) => writeln!(out, "Scheduler: running ({h})"),
        None => writeln!(
            out,
            "Scheduler: not running (the desktop app starts it); `start` won't work"
        ),
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
    let now = exec::now();
    let _ = writeln!(
        out,
        "{:<18} {:<28} {:<34} last run",
        "job", "schedule", "now"
    );
    for job in jobs {
        let st = state::read(user_data, &job.name);
        let holder = exec::holder(user_data, &job.name);
        let now_col = match &holder {
            Some(h) => {
                let since = st
                    .last
                    .as_ref()
                    .map(|r| localtime::fmt_local(r.started))
                    .unwrap_or_default();
                format!("running since {since} ({})", h.pid)
            }
            None => "idle".into(),
        };
        let schedule = match &job.schedule {
            Some(s) => match s.next_after(now) {
                Some(next) => format!("{}, next {}", s.text(), localtime::fmt_upcoming(next, now)),
                None => format!("{}, never", s.text()),
            },
            None => "on request".into(),
        };
        let last = match &st.last {
            Some(r) => {
                let took = r
                    .ended
                    .map(|e| format!(", {}", minutes(e - r.started)))
                    .unwrap_or_default();
                let exit = r.exit.map(|c| format!(" (exit {c})")).unwrap_or_default();
                // A run whose process died (killed, power cut) never wrote its end.
                let result = if r.ended.is_none() && holder.is_none() {
                    "interrupted"
                } else {
                    r.result.as_str()
                };
                format!(
                    "{} {}{took}: {result}{exit}",
                    localtime::fmt_local(r.started),
                    r.trigger,
                )
            }
            None => "never".into(),
        };
        let _ = writeln!(
            out,
            "{:<18} {:<28} {:<34} {last}",
            job.name, schedule, now_col
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
