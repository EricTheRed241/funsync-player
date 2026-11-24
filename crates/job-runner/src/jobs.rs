//! The job definitions (`deploy/jobs.json`, built in; `--jobs <file>`
//! replaces them). A job with a `schedule` is started by the scheduler
//! (`serve`) itself; the others by `start` (their Orca automation).

use crate::schedule::Schedule;
use serde::Deserialize;

pub const BUILT_IN: &str = include_str!("../../../deploy/jobs.json");

#[derive(Debug, Clone)]
pub struct Job {
    pub name: String,
    /// Seconds: a run still going after this is stopped.
    pub timeout: i64,
    pub steps: Vec<Vec<String>>,
    /// When `serve` starts it; None: only on `start` requests.
    pub schedule: Option<Schedule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJob {
    name: String,
    timeout: String,
    steps: Vec<Vec<String>>,
    schedule: Option<String>,
}

#[derive(Deserialize)]
struct RawFile {
    jobs: Vec<RawJob>,
}

pub fn parse(text: &str) -> Result<Vec<Job>, String> {
    let file: RawFile = serde_json::from_str(text).map_err(|e| format!("job definitions: {e}"))?;
    let mut jobs: Vec<Job> = Vec::new();
    for raw in file.jobs {
        let name = raw.name;
        let err = |e: String| format!("job {name}: {e}");
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(format!("job name \"{name}\": letters, digits and - only"));
        }
        if jobs.iter().any(|j| j.name == name) {
            return Err(err("defined twice".into()));
        }
        if raw.steps.is_empty() || raw.steps.iter().any(Vec::is_empty) {
            return Err(err("needs steps, each a command".into()));
        }
        jobs.push(Job {
            timeout: duration(&raw.timeout).map_err(err)?,
            schedule: raw
                .schedule
                .as_deref()
                .map(Schedule::parse)
                .transpose()
                .map_err(err)?,
            steps: raw.steps,
            name,
        });
    }
    Ok(jobs)
}

/// `90s`, `30m`, `12h` → seconds.
pub fn duration(text: &str) -> Result<i64, String> {
    let bad = || format!("duration \"{text}\": a number with s, m or h (e.g. 30m)");
    let unit = match text.chars().last() {
        Some('s') => 1,
        Some('m') => 60,
        Some('h') => 3600,
        _ => return Err(bad()),
    };
    let n: i64 = text[..text.len() - 1].parse().map_err(|_| bad())?;
    if n <= 0 {
        return Err(bad());
    }
    Ok(n * unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_jobs_parse() {
        let jobs = parse(BUILT_IN).unwrap();
        let downloads: Vec<&Job> = jobs
            .iter()
            .filter(|j| j.name.starts_with("downloads-"))
            .collect();
        assert_eq!(downloads.len(), 4);
        assert!(downloads.iter().all(|j| j.schedule.is_some()));
        let ero = jobs
            .iter()
            .find(|j| j.name == "downloads-eroscripts")
            .unwrap();
        assert_eq!(ero.schedule.as_ref().map(Schedule::text), Some("0 * * * *"));
        let slr = jobs.iter().find(|j| j.name == "slr-downloads").unwrap();
        assert_eq!(slr.schedule.as_ref().map(Schedule::text), Some("0 4 * * 1"));
        let thumbnails = jobs.iter().find(|j| j.name == "thumbnails").unwrap();
        assert!(thumbnails.schedule.is_none());
    }

    #[test]
    fn durations() {
        assert_eq!(duration("90s"), Ok(90));
        assert_eq!(duration("30m"), Ok(1800));
        assert_eq!(duration("12h"), Ok(43_200));
        for bad in ["12", "h", "0m", "-5m", "1.5h", "2d"] {
            assert!(duration(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn rejects_duplicates_bad_names_and_empty_steps() {
        let job = |name: &str, steps: &str| {
            format!(r#"{{"name":"{name}","timeout":"1h","steps":{steps}}}"#)
        };
        let file = |jobs: &[String]| format!(r#"{{"jobs":[{}]}}"#, jobs.join(","));
        assert!(
            parse(&file(&[
                job("a", r#"[["true"]]"#),
                job("a", r#"[["true"]]"#)
            ]))
            .is_err()
        );
        assert!(parse(&file(&[job("a", "[]")])).is_err());
        assert!(parse(&file(&[job("a", "[[]]")])).is_err());
        assert!(parse(&file(&[job("a b", r#"[["true"]]"#)])).is_err());
        assert!(parse(&file(&[job("a", r#"[["true"]]"#)])).is_ok());
    }

    #[test]
    fn a_schedule_is_optional_and_checked() {
        let file = |extra: &str| {
            format!(r#"{{"jobs":[{{"name":"a","timeout":"1h","steps":[["true"]]{extra}}}]}}"#)
        };
        assert!(parse(&file("")).unwrap()[0].schedule.is_none());
        let jobs = parse(&file(r#","schedule":"15 * * * *""#)).unwrap();
        assert_eq!(jobs[0].schedule.as_ref().unwrap().text(), "15 * * * *");
        let e = parse(&file(r#","schedule":"61 * * * *""#)).unwrap_err();
        assert!(e.contains("job a") && e.contains("minute"), "{e}");
        assert!(parse(&file(r#","when":"daily""#)).is_err());
    }
}
