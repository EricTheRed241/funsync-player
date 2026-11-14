//! The job definitions (`deploy/jobs.json`, built in; `--jobs <file>`
//! replaces them).

use crate::cron::Schedule;
use serde::Deserialize;

pub const BUILT_IN: &str = include_str!("../../../deploy/jobs.json");

#[derive(Debug, Clone)]
pub struct Job {
    pub name: String,
    pub schedule_text: String,
    pub schedule: Schedule,
    /// Seconds: how late a missed run may still start.
    pub catch_up: i64,
    /// Seconds: a run still going after this is stopped.
    pub timeout: i64,
    /// Paused unless `resume` was given for it.
    pub paused_by_default: bool,
    pub steps: Vec<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawJob {
    name: String,
    schedule: String,
    catch_up: String,
    timeout: String,
    #[serde(default)]
    paused: bool,
    steps: Vec<Vec<String>>,
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
            schedule: Schedule::parse(&raw.schedule).map_err(err)?,
            schedule_text: raw.schedule,
            catch_up: duration(&raw.catch_up).map_err(err)?,
            timeout: duration(&raw.timeout).map_err(err)?,
            paused_by_default: raw.paused,
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
        let scout = jobs.iter().find(|j| j.name == "eroscripts-scout").unwrap();
        assert_eq!(
            (scout.catch_up, scout.timeout, scout.steps.len()),
            (12 * 3600, 12 * 3600, 2)
        );
        assert!(
            jobs.iter()
                .find(|j| j.name == "faptap-scout")
                .unwrap()
                .paused_by_default
        );
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
    fn rejects_duplicates_and_empty_steps() {
        let job = |name: &str, steps: &str| {
            format!(
                r#"{{"name":"{name}","schedule":"0 1 * * *","catchUp":"1h","timeout":"1h","steps":{steps}}}"#
            )
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
}
