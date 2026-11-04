//! The judge: a vision-capable agent run (default `omp -p …`, `--judge-cmd`
//! to replace it) that views every contact sheet and writes `p_minor` per
//! video, and the checks its results files must pass.

use crate::sheets::{Batch, ManifestEntry};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

pub const DEFAULT_MAX_COST: f64 = 30.0;

/// The scoring rubric every judging subagent gets, word for word.
pub const RUBRIC: &str = "`p_minor` (0–1): probability that a character in sexual content is depicted as, or clearly looks like, someone under 18. Signals: childlike body proportions, small stature beside adults, a child's face with a childlike body; explicit youth framing (school setting with a childlike look, \"loli\"/\"shota\" style); a character who is canonically a minor (e.g. Sarada Uchiha). Anime art giving an adult a youthful face is NOT enough on its own: adult proportions and an adult context mean a low p. Canonically adult characters drawn as adults also mean a low p. Guide: 0.5+ = more likely than not depicts a minor, 0.2–0.5 = could look underage, below 0.2 = clearly adult.";

/// The default judge command; the prompt is appended as the last argument.
/// `--wait-async`: print mode would otherwise exit (cancelling them) as soon
/// as the main agent has spawned its background subagents.
pub fn default_command(max_cost: f64) -> String {
    format!("omp -p --wait-async --tools read,write,task --auto-approve --max-cost {max_cost}")
}

/// The judge's instructions for these batches.
pub fn prompt(work: &Path, batches: &[Batch]) -> String {
    let mut list = String::new();
    for b in batches {
        list.push_str(&format!(
            "- {} → {} ({} video(s))\n",
            b.manifest.display(),
            b.results.display(),
            b.entries.len()
        ));
    }
    format!(
        "You are screening a video library for sexual content that depicts minors. \
Work folder: {work}. Each batch file below is a JSON array of {{\"id\", \"title\", \"sheet\"}}: \
\"sheet\" is the absolute path of a JPEG contact sheet (12 frames spread over the video, 4 columns × 3 rows, \
in time order; VR videos show one eye), \"title\" is the video's path inside the library (folder and file \
names often name the characters and the franchise).

Batches (batch file → results file):
{list}
Spawn one `task` subagent per batch, all in a single parallel call. Give each subagent the rubric below \
word for word, its batch file and its results file, and these instructions:
1. Read the batch file.
2. View every sheet in it with the `read` tool, one by one. Never score a video from its title alone; \
use the title together with the sheet (a canonically underage character named in the title counts).
3. Write the results file with the `write` tool: a JSON array with exactly one object per batch entry, \
in the batch's order, with the same ids: [{{\"id\": \"<id>\", \"p_minor\": <number from 0 to 1>, \
\"reason\": \"<one short sentence: who or what you saw>\"}}]. Nothing else in the file.

Rubric:
{RUBRIC}

Don't change any other file. The subagents' results come back to you as they finish; when every subagent \
has reported and every results file exists, reply with one line: done. If a subagent failed, run that batch again.",
        work = work.display()
    )
}

/// Run the judge (`sh -c '<cmd> "$1"' age-screen-judge <prompt>`) in the
/// work folder; its output goes to this process's stderr. Ok when it exits
/// 0.
pub fn run(command: &str, prompt: &str, work: &Path) -> Result<(), String> {
    use std::os::fd::AsFd;
    let to_stderr = std::io::stderr()
        .as_fd()
        .try_clone_to_owned()
        .map_err(|e| format!("can't start the judge ({e})"))?;
    let status = Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("{command} \"$1\""))
        .arg("age-screen-judge")
        .arg(prompt)
        .current_dir(work)
        .stdin(Stdio::null())
        .stdout(Stdio::from(to_stderr))
        .status()
        .map_err(|e| format!("can't start the judge ({e})"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("the judge exited with {status}"))
    }
}

/// A judged video.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub id: String,
    pub p_minor: f64,
    pub reason: String,
}

/// A batch's results file, checked against its manifest; verdicts in
/// manifest order. Any problem rejects the whole file (its videos stay
/// unscreened).
pub fn read_results(batch: &Batch) -> Result<Vec<Verdict>, String> {
    let raw = match fs::read_to_string(&batch.results) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err("no results file".into());
        }
        Err(e) => return Err(e.to_string()),
    };
    validate(&raw, &batch.entries)
}

pub fn validate(raw: &str, manifest: &[ManifestEntry]) -> Result<Vec<Verdict>, String> {
    let v: Value = serde_json::from_str(raw).map_err(|e| format!("not JSON ({e})"))?;
    let list = v.as_array().ok_or("not a JSON array")?;
    let mut by_id: HashMap<String, Verdict> = HashMap::new();
    for (i, e) in list.iter().enumerate() {
        let id = match e.get("id") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            _ => return Err(format!("entry {} has no id", i + 1)),
        };
        if !manifest.iter().any(|m| m.id == id) {
            return Err(format!("id {id} is not in the batch"));
        }
        let p = e
            .get("p_minor")
            .and_then(Value::as_f64)
            .filter(|p| (0.0..=1.0).contains(p))
            .ok_or_else(|| format!("id {id}: p_minor missing or outside 0–1"))?;
        let reason = e
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if by_id
            .insert(
                id.clone(),
                Verdict {
                    id: id.clone(),
                    p_minor: p,
                    reason,
                },
            )
            .is_some()
        {
            return Err(format!("id {id} appears twice"));
        }
    }
    let missing: Vec<&str> = manifest
        .iter()
        .filter(|m| !by_id.contains_key(&m.id))
        .map(|m| m.id.as_str())
        .collect();
    if !missing.is_empty() {
        return Err(format!("no result for {}", missing.join(", ")));
    }
    Ok(manifest
        .iter()
        .filter_map(|m| by_id.remove(&m.id))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(ids: &[&str]) -> Vec<ManifestEntry> {
        ids.iter()
            .map(|id| ManifestEntry {
                id: id.to_string(),
                title: format!("t/{id}.mp4"),
                sheet: format!("/w/{id}.jpg"),
            })
            .collect()
    }

    #[test]
    fn accepts_a_complete_file_in_manifest_order() {
        let m = manifest(&["a", "b"]);
        let got = validate(
            r#"[{"id": "b", "p_minor": 1, "reason": " x "}, {"id": "a", "p_minor": 0}]"#,
            &m,
        )
        .unwrap();
        assert_eq!(
            got,
            vec![
                Verdict {
                    id: "a".into(),
                    p_minor: 0.0,
                    reason: String::new()
                },
                Verdict {
                    id: "b".into(),
                    p_minor: 1.0,
                    reason: "x".into()
                },
            ]
        );
    }

    #[test]
    fn rejects_missing_extra_bad_ids_and_bad_scores() {
        let m = manifest(&["a", "b"]);
        let cases = [
            (r#"[{"id": "a", "p_minor": 0.1}]"#, "no result for b"),
            (
                r#"[{"id": "a", "p_minor": 0.1}, {"id": "b", "p_minor": 0.1}, {"id": "c", "p_minor": 0.1}]"#,
                "id c is not in the batch",
            ),
            (
                r#"[{"id": "a", "p_minor": 0.1}, {"id": "a", "p_minor": 0.2}]"#,
                "id a appears twice",
            ),
            (
                r#"[{"p_minor": 0.1}, {"id": "b", "p_minor": 0.1}]"#,
                "entry 1 has no id",
            ),
            (
                r#"[{"id": "a", "p_minor": 1.2}, {"id": "b", "p_minor": 0.1}]"#,
                "id a: p_minor missing or outside 0–1",
            ),
            (
                r#"[{"id": "a", "p_minor": "0.5"}, {"id": "b", "p_minor": 0.1}]"#,
                "id a: p_minor missing or outside 0–1",
            ),
            (r#"{"a": 0.1}"#, "not a JSON array"),
        ];
        for (raw, want) in cases {
            assert_eq!(validate(raw, &m).unwrap_err(), want, "{raw}");
        }
        assert!(validate("[", &m).unwrap_err().starts_with("not JSON"));
    }

    #[test]
    fn a_missing_results_file_is_rejected() {
        let b = Batch {
            name: "01".into(),
            manifest: "/nonexistent/batches/01.json".into(),
            results: "/nonexistent/results/01.json".into(),
            entries: manifest(&["a"]),
        };
        assert_eq!(read_results(&b).unwrap_err(), "no results file");
    }

    #[test]
    fn the_prompt_names_every_batch_and_carries_the_rubric() {
        let b = Batch {
            name: "01".into(),
            manifest: "/w/batches/01.json".into(),
            results: "/w/results/01.json".into(),
            entries: manifest(&["a"]),
        };
        let p = prompt(Path::new("/w"), &[b]);
        assert!(p.contains("/w/batches/01.json → /w/results/01.json (1 video(s))"));
        assert!(p.contains(RUBRIC));
        assert_eq!(
            default_command(12.5),
            "omp -p --wait-async --tools read,write,task --auto-approve --max-cost 12.5"
        );
    }
}
