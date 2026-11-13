//! The report: every post the scout judged (eroscripts-scout-state.json),
//! merged with the waiting list (eroscripts-scout.json), the scenes turned
//! down (eroscripts-scout-rejected.json), how many of its videos the
//! library holds (crate::library) and the deck's swipe (crate::swipes).
//! Built fresh from the files on every request; a missing file is an empty
//! section.

use crate::swipes::{self, Dir, Swipes};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

pub const STATE_FILE: &str = "eroscripts-scout-state.json";
pub const WAITING_FILE: &str = "eroscripts-scout.json";
pub const REJECTED_FILE: &str = "eroscripts-scout-rejected.json";

pub const FORUM_TOPIC: &str = "https://discuss.eroscripts.com/t/";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub last_scan_at: Option<String>,
    /// `generatedAt` of the waiting list.
    pub waiting_list_at: Option<String>,
    pub feeds: Map<String, Value>,
    /// Posts per verdict.
    pub verdicts: BTreeMap<String, usize>,
    pub waiting: usize,
    /// Posts with at least one video in the library.
    pub in_library: usize,
    /// Files that exist but couldn't be read or parsed (their section is empty).
    pub problems: Vec<String>,
    /// Newest topic first.
    pub posts: Vec<Post>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Post {
    pub id: u64,
    pub url: String,
    pub verdict: String,
    pub rules: Value,
    pub at: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub created: Option<String>,
    pub bumped: Option<String>,
    pub image: Option<String>,
    pub waiting: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub likes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hosts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vr: Option<bool>,
    /// Total size of the scene videos whose size is known, in GB (10^9 bytes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gb: Option<f64>,
    /// `{base, axes, packed?, video: {name, host, size?}}` per scene.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scenes: Option<Vec<Value>>,
    /// Entries of the rejected list for this topic (`reason`, `video`, `at`, …).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rejected_scenes: Vec<Value>,
    /// Library videos from this post (scout downloads, app downloads, sync links); 0 = not downloaded.
    pub in_library: usize,
    /// The deck's swipe: `right` (wanted) or `left` (hidden from the deck).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swipe: Option<Dir>,
}

/// Read and parse `<user_data>/<name>`: `Value::Null` when missing, or when
/// unreadable / unparsable (then also a line in `problems`).
pub fn read_json(user_data: &Path, name: &str, problems: &mut Vec<String>) -> Value {
    match std::fs::read(user_data.join(name)) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            problems.push(format!("{name}: {e}"));
            Value::Null
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Null,
        Err(e) => {
            problems.push(format!("{name}: {e}"));
            Value::Null
        }
    }
}

/// The deck's swipes; an unreadable file is a line in `problems` and no swipes.
pub fn read_swipes(user_data: &Path, problems: &mut Vec<String>) -> Swipes {
    swipes::read(user_data).unwrap_or_else(|e| {
        problems.push(e);
        Swipes::default()
    })
}

/// Read the scout's three files, `config.json`, `eroscripts-links.json` and the swipes from
/// `user_data` and build the report.
pub fn load(user_data: &Path) -> Report {
    let mut problems = Vec::new();
    let state = read_json(user_data, STATE_FILE, &mut problems);
    let waiting = read_json(user_data, WAITING_FILE, &mut problems);
    let rejected = read_json(user_data, REJECTED_FILE, &mut problems);
    let config = read_json(user_data, "config.json", &mut problems);
    let links = read_json(user_data, crate::library::LINKS_FILE, &mut problems);
    let swipes = read_swipes(user_data, &mut problems);
    let videos = crate::library::topic_videos(&config, &links, &mut problems);
    let mut report = build(&state, &waiting, &rejected);
    mark_in_library(&mut report, &videos);
    for post in &mut report.posts {
        post.swipe = swipes.dir(post.id);
    }
    report.problems = problems;
    report
}

/// Set each post's library video count and the report's total.
pub fn mark_in_library(report: &mut Report, videos: &HashMap<u64, usize>) {
    for post in &mut report.posts {
        post.in_library = videos.get(&post.id).copied().unwrap_or(0);
    }
    report.in_library = report.posts.iter().filter(|p| p.in_library > 0).count();
}

pub(crate) fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn strings_of(v: &Value, key: &str) -> Option<Vec<String>> {
    v.get(key).and_then(Value::as_array).map(|a| {
        a.iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    })
}

/// A topic id from a number or a numeric string.
pub(crate) fn id_of(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// The scene fields the page shows.
fn scene_summary(scene: &Value) -> Value {
    let mut out = Map::new();
    out.insert(
        "base".into(),
        scene.get("base").cloned().unwrap_or(Value::Null),
    );
    out.insert(
        "axes".into(),
        scene
            .get("axes")
            .cloned()
            .unwrap_or_else(|| Value::Array(vec![])),
    );
    if let Some(packed) = scene.get("packed").filter(|p| !p.is_null()) {
        out.insert("packed".into(), packed.clone());
    }
    let video = scene.get("video").unwrap_or(&Value::Null);
    let mut v = Map::new();
    v.insert(
        "name".into(),
        video.get("name").cloned().unwrap_or(Value::Null),
    );
    v.insert(
        "host".into(),
        video.get("host").cloned().unwrap_or(Value::Null),
    );
    if let Some(size) = video.get("size").filter(|s| s.is_number()) {
        v.insert("size".into(), size.clone());
    }
    out.insert("video".into(), Value::Object(v));
    Value::Object(out)
}

/// Total size of the scenes' videos whose size is known, in GB (10^9 bytes, 2 decimals).
pub(crate) fn total_gb(scenes: &[Value]) -> Option<f64> {
    let sizes: Vec<f64> = scenes
        .iter()
        .filter_map(|s| s.pointer("/video/size").and_then(Value::as_f64))
        .collect();
    if sizes.is_empty() {
        return None;
    }
    let gb = sizes.iter().sum::<f64>() / 1e9;
    Some((gb * 100.0).round() / 100.0)
}

/// Merge the parsed files (`Value::Null` for a missing one) into the report.
pub fn build(state: &Value, waiting: &Value, rejected: &Value) -> Report {
    let candidates: HashMap<u64, &Value> = waiting
        .get("candidates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| Some((id_of(c.get("id")?)?, c)))
        .collect();

    // Rejected scenes per topic, without the fields the post already shows.
    let mut rejected_by_topic: HashMap<u64, Vec<Value>> = HashMap::new();
    let mut rejected_titles: HashMap<u64, String> = HashMap::new();
    for entry in rejected
        .get("entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = entry.get("topicId").and_then(id_of) else {
            continue;
        };
        if let Some(title) = str_of(entry, "title") {
            rejected_titles.entry(id).or_insert(title);
        }
        let mut e = entry.as_object().cloned().unwrap_or_default();
        for k in ["topicId", "title", "url"] {
            e.remove(k);
        }
        rejected_by_topic
            .entry(id)
            .or_default()
            .push(Value::Object(e));
    }

    let empty = Map::new();
    let topics = state
        .get("topics")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let mut records: BTreeMap<u64, &Value> = topics
        .iter()
        .filter_map(|(k, v)| Some((k.parse().ok()?, v)))
        .collect();
    // A waiting post the state doesn't know (shouldn't happen) still shows.
    static NO_RECORD: Value = Value::Null;
    for id in candidates.keys() {
        records.entry(*id).or_insert(&NO_RECORD);
    }

    let mut verdicts = BTreeMap::new();
    let mut posts = Vec::with_capacity(records.len());
    for (id, rec) in records.into_iter().rev() {
        let cand = candidates.get(&id).copied();
        let verdict = rec
            .get("verdict")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| {
                if cand.is_some() {
                    "candidate".into()
                } else {
                    "unknown".into()
                }
            });
        *verdicts.entry(verdict.clone()).or_insert(0) += 1;
        let from_cand = |key: &str| cand.and_then(|c| str_of(c, key));
        let scenes: Option<Vec<Value>> = cand
            .and_then(|c| c.get("scenes"))
            .and_then(Value::as_array)
            .map(|a| a.iter().map(scene_summary).collect());
        posts.push(Post {
            id,
            url: format!("{FORUM_TOPIC}{id}"),
            verdict,
            rules: rec.get("rules").cloned().unwrap_or(Value::Null),
            at: str_of(rec, "at"),
            title: str_of(rec, "title")
                .or_else(|| from_cand("title"))
                .or_else(|| rejected_titles.remove(&id)),
            author: str_of(rec, "author").or_else(|| from_cand("author")),
            created: str_of(rec, "created"),
            bumped: str_of(rec, "bumped"),
            image: str_of(rec, "image"),
            waiting: cand.is_some(),
            score: cand.and_then(|c| c.get("score")).and_then(Value::as_f64),
            axis_type: from_cand("axisType"),
            tags: cand.and_then(|c| strings_of(c, "tags")),
            likes: cand.and_then(|c| c.get("likes")).and_then(Value::as_i64),
            hosts: cand.and_then(|c| strings_of(c, "hosts")),
            vr: cand.and_then(|c| c.get("vr")).and_then(Value::as_bool),
            gb: scenes.as_deref().and_then(total_gb),
            scenes,
            rejected_scenes: rejected_by_topic.remove(&id).unwrap_or_default(),
            in_library: 0,
            swipe: None,
        });
    }

    Report {
        last_scan_at: str_of(state, "lastScanAt"),
        waiting_list_at: str_of(waiting, "generatedAt"),
        feeds: state
            .get("feeds")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default(),
        verdicts,
        waiting: candidates.len(),
        in_library: 0,
        problems: Vec::new(),
        posts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state() -> Value {
        json!({
            "version": 1,
            "lastScanAt": "2026-11-12T15:51:07.650Z",
            "feeds": {
                "multi-axis": {"backfillPage": 65, "exhausted": false, "doneSince": "2025-01-01", "reached": "2026-10-23T08:56:26.621Z"},
                "vam": {"backfillPage": 2, "exhausted": true}
            },
            "topics": {
                "2095": {"verdict": "excluded", "rules": 8, "at": "2026-11-04T14:33:24.681Z"},
                "300": {"verdict": "candidate", "rules": 12, "at": "2026-11-10T00:00:00Z",
                        "title": "From state", "author": "alice", "created": "2026-01-02T00:00:00Z",
                        "bumped": "2026-03-04T00:00:00Z", "image": "https://eroscripts-discourse.eroscripts.com/a.jpg"},
                "400": {"verdict": "candidate", "rules": 12, "at": "2026-11-11T00:00:00Z"},
                "500": {"verdict": "short-and-sad", "rules": 13, "at": "2026-11-11T00:00:00Z"},
                "not-a-number": {"verdict": "excluded"}
            },
            "pending": {}
        })
    }

    fn waiting() -> Value {
        json!({
            "generatedAt": "2026-11-12T15:51:07.948Z",
            "candidates": [
                {"id": 300, "title": "From list", "author": "bob", "url": "https://discuss.eroscripts.com/t/300",
                 "tags": ["vr", "pov"], "likes": 9, "vr": true, "axisType": "multi", "hosts": ["pixeldrain"],
                 "score": 4.5, "scenes": [
                    {"base": "A", "axes": ["twist"], "packed": true, "scripts": [{"name": "A.funscript"}],
                     "video": {"name": "A.mp4", "host": "pixeldrain", "size": 1_500_000_000u64, "url": "https://x"}},
                    {"base": "B", "axes": [], "video": {"name": "B.mp4", "host": "MEGA"}}
                 ]},
                {"id": 400, "title": "Only in list", "author": "carol", "tags": [], "score": -1.0,
                 "axisType": "single", "scenes": [{"base": "C", "axes": [], "video": {"name": "C.mp4", "host": "MEGA"}}]}
            ]
        })
    }

    fn rejected() -> Value {
        json!({
            "version": 1,
            "entries": [
                {"key": "500|x.mp4", "topicId": 500, "video": "x.mp4", "title": "Rejected title",
                 "url": "https://discuss.eroscripts.com/t/500", "reason": "short", "lengthMs": 1000, "at": "2026-11-01T00:00:00Z"},
                {"key": "300|y.mp4", "topicId": 300, "video": "y.mp4", "title": "ignored", "reason": "duplicate", "at": "2026-11-01T00:00:00Z"},
                {"key": "faptap|1", "faptapId": 1, "reason": "low-rating", "source": "faptap"}
            ]
        })
    }

    fn post(r: &Report, id: u64) -> &Post {
        r.posts.iter().find(|p| p.id == id).unwrap()
    }

    #[test]
    fn merges_state_waiting_list_and_rejected() {
        let r = build(&state(), &waiting(), &rejected());
        assert_eq!(
            r.posts.iter().map(|p| p.id).collect::<Vec<_>>(),
            [2095, 500, 400, 300]
        );
        assert_eq!(r.last_scan_at.as_deref(), Some("2026-11-12T15:51:07.650Z"));
        assert_eq!(
            r.waiting_list_at.as_deref(),
            Some("2026-11-12T15:51:07.948Z")
        );
        assert_eq!(r.waiting, 2);
        assert_eq!(r.feeds["vam"]["exhausted"], json!(true));

        // The state's own details win over the waiting list's.
        let p = post(&r, 300);
        assert_eq!(p.title.as_deref(), Some("From state"));
        assert_eq!(p.author.as_deref(), Some("alice"));
        assert_eq!(p.created.as_deref(), Some("2026-01-02T00:00:00Z"));
        assert_eq!(p.bumped.as_deref(), Some("2026-03-04T00:00:00Z"));
        assert!(p.image.as_deref().unwrap().starts_with("https://"));
        assert!(p.waiting);
        assert_eq!(p.score, Some(4.5));
        assert_eq!(p.axis_type.as_deref(), Some("multi"));
        assert_eq!(
            p.tags.as_deref(),
            Some(&["vr".to_string(), "pov".into()][..])
        );
        assert_eq!(p.likes, Some(9));
        assert_eq!(p.vr, Some(true));
        assert_eq!(p.hosts.as_deref(), Some(&["pixeldrain".to_string()][..]));
        assert_eq!(p.gb, Some(1.5));
        let scenes = p.scenes.as_ref().unwrap();
        assert_eq!(
            scenes[0],
            json!({"base": "A", "axes": ["twist"], "packed": true,
                   "video": {"name": "A.mp4", "host": "pixeldrain", "size": 1_500_000_000u64}})
        );
        assert_eq!(
            scenes[1],
            json!({"base": "B", "axes": [], "video": {"name": "B.mp4", "host": "MEGA"}})
        );
        assert_eq!(
            p.rejected_scenes,
            [
                json!({"key": "300|y.mp4", "video": "y.mp4", "reason": "duplicate", "at": "2026-11-01T00:00:00Z"})
            ]
        );

        // The waiting list fills in what the state lacks; no sizes, no gb.
        let p = post(&r, 400);
        assert_eq!(p.title.as_deref(), Some("Only in list"));
        assert_eq!(p.author.as_deref(), Some("carol"));
        assert_eq!(p.gb, None);
        assert!(p.waiting);
    }

    #[test]
    fn id_only_records_and_rejected_titles() {
        let r = build(&state(), &waiting(), &rejected());
        let p = post(&r, 2095);
        assert_eq!(p.url, "https://discuss.eroscripts.com/t/2095");
        assert_eq!(p.verdict, "excluded");
        assert_eq!(p.rules, json!(8));
        assert_eq!(p.at.as_deref(), Some("2026-11-04T14:33:24.681Z"));
        assert_eq!(
            (&p.title, &p.author, &p.created, &p.bumped, &p.image),
            (&None, &None, &None, &None, &None)
        );
        assert!(!p.waiting && p.score.is_none() && p.scenes.is_none());
        let v = serde_json::to_value(p).unwrap();
        assert_eq!(v["title"], Value::Null);
        assert_eq!(v["image"], Value::Null);
        assert!(v.get("score").is_none() && v.get("rejectedScenes").is_none());

        // A rejected scene's post title names a post the state knows by id only.
        let p = post(&r, 500);
        assert_eq!(p.title.as_deref(), Some("Rejected title"));
        assert_eq!(p.rejected_scenes[0]["reason"], "short");
        assert_eq!(p.rejected_scenes[0]["lengthMs"], 1000);
    }

    #[test]
    fn counts_verdicts_including_unknown_strings() {
        let r = build(&state(), &waiting(), &rejected());
        assert_eq!(
            r.verdicts,
            BTreeMap::from([
                ("candidate".to_string(), 2),
                ("excluded".to_string(), 1),
                ("short-and-sad".to_string(), 1),
            ])
        );
        assert_eq!(post(&r, 500).verdict, "short-and-sad");
    }

    #[test]
    fn waiting_post_missing_from_state_still_shows() {
        let r = build(&Value::Null, &waiting(), &Value::Null);
        assert_eq!(r.posts.len(), 2);
        assert_eq!(post(&r, 300).verdict, "candidate");
        assert_eq!(post(&r, 300).title.as_deref(), Some("From list"));
        assert_eq!(post(&r, 300).rules, Value::Null);
    }

    #[test]
    fn missing_files_are_empty_sections() {
        let dir = std::env::temp_dir().join(format!("scout-report-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let r = load(&dir);
        assert!(r.posts.is_empty() && r.feeds.is_empty() && r.verdicts.is_empty());
        assert_eq!(
            (r.last_scan_at, r.waiting_list_at, r.waiting),
            (None, None, 0)
        );
        assert!(r.problems.is_empty());

        // A broken file is reported and its section left empty.
        std::fs::write(dir.join(STATE_FILE), serde_json::to_vec(&state()).unwrap()).unwrap();
        std::fs::write(dir.join(WAITING_FILE), b"{not json").unwrap();
        let r = load(&dir);
        assert_eq!(r.posts.len(), 4);
        assert_eq!(r.waiting, 0);
        assert!(!post(&r, 300).waiting);
        assert_eq!(r.problems.len(), 1);
        assert!(r.problems[0].starts_with(WAITING_FILE));

        // Posts carry their swipe; a broken swipes file is a problem, not an error.
        std::fs::write(
            dir.join(swipes::FILE),
            br#"{"version":1,"swipes":{"300":{"dir":"right","at":"x"},"2095":{"dir":"left","at":"y"}}}"#,
        )
        .unwrap();
        let r = load(&dir);
        assert_eq!(post(&r, 300).swipe, Some(Dir::Right));
        assert_eq!(post(&r, 2095).swipe, Some(Dir::Left));
        assert_eq!(post(&r, 400).swipe, None);
        let v = serde_json::to_value(post(&r, 300)).unwrap();
        assert_eq!(v["swipe"], "right");
        assert!(
            serde_json::to_value(post(&r, 400))
                .unwrap()
                .get("swipe")
                .is_none()
        );
        std::fs::write(dir.join(swipes::FILE), b"[]").unwrap();
        let r = load(&dir);
        assert_eq!(post(&r, 300).swipe, None);
        assert!(r.problems.iter().any(|p| p.starts_with(swipes::FILE)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
