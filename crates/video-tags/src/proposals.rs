//! The proposals file a tagging job writes:
//!
//! ```json
//! { "by": "tagger-name", "videos": { "/abs/video.mp4": { "tags": ["pov"], "by": "…" } } }
//! [ { "path": "/abs/video.mp4", "tags": ["pov"], "by": "…" } ]
//! ```
//!
//! `by` (optional, per entry or for the whole object) is stored with the
//! entry; other fields are ignored.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    /// The path as proposed (validated later).
    pub path: String,
    pub tags: Vec<String>,
    pub by: Option<String>,
    /// Why the entry itself is malformed, if it is.
    pub problem: Option<String>,
}

pub fn parse(raw: &str) -> Result<Vec<Proposal>, String> {
    let v: Value = serde_json::from_str(raw).map_err(|e| format!("not valid JSON: {e}"))?;
    let by_all = v.get("by").and_then(Value::as_str);
    match &v {
        Value::Object(o) => {
            let videos = o
                .get("videos")
                .and_then(Value::as_object)
                .ok_or("expected {\"videos\": {\"<path>\": {\"tags\": [...]}}} or an array of {\"path\", \"tags\"}")?;
            Ok(videos
                .iter()
                .map(|(path, entry)| proposal(path.clone(), entry, by_all))
                .collect())
        }
        Value::Array(items) => Ok(items
            .iter()
            .enumerate()
            .map(
                |(i, entry)| match entry.get("path").and_then(Value::as_str) {
                    Some(path) => proposal(path.to_string(), entry, None),
                    None => Proposal {
                        path: format!("<entry {}>", i + 1),
                        tags: Vec::new(),
                        by: None,
                        problem: Some("no \"path\" string".into()),
                    },
                },
            )
            .collect()),
        _ => Err("expected an object with \"videos\" or an array".into()),
    }
}

fn proposal(path: String, entry: &Value, by_all: Option<&str>) -> Proposal {
    let by = entry
        .get("by")
        .and_then(Value::as_str)
        .or(by_all)
        .map(str::to_string);
    let mut problem = None;
    let mut tags = Vec::new();
    match entry.get("tags").and_then(Value::as_array) {
        None => problem = Some("no \"tags\" array".to_string()),
        Some(list) => {
            for t in list {
                match t.as_str() {
                    Some(s) => tags.push(s.to_string()),
                    None => problem = Some(format!("tag {t} is not a string")),
                }
            }
        }
    }
    Proposal {
        path,
        tags,
        by,
        problem,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_object_form_with_by() {
        let p = parse(r#"{"by":"job","videos":{"/a.mp4":{"tags":["pov"],"score":0.9},"/b.mp4":{"tags":[],"by":"me"}}}"#).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(
            p[0],
            Proposal {
                path: "/a.mp4".into(),
                tags: vec!["pov".into()],
                by: Some("job".into()),
                problem: None
            }
        );
        assert_eq!(p[1].by.as_deref(), Some("me"));
    }

    #[test]
    fn reads_the_array_form_and_flags_bad_entries() {
        let p = parse(r#"[{"path":"/a.mp4","tags":["vr"]},{"tags":["pov"]},{"path":"/c.mp4","tags":[1]},{"path":"/d.mp4"}]"#).unwrap();
        assert_eq!(p[0].tags, vec!["vr".to_string()]);
        assert_eq!(p[1].problem.as_deref(), Some("no \"path\" string"));
        assert_eq!(p[2].problem.as_deref(), Some("tag 1 is not a string"));
        assert_eq!(p[3].problem.as_deref(), Some("no \"tags\" array"));
    }

    #[test]
    fn rejects_other_shapes() {
        assert!(parse("{}").is_err());
        assert!(parse("3").is_err());
        assert!(parse("{nope").is_err());
    }
}
