//! `<userData>/slr-scout.json`: the scored scenes and the account's download
//! quota, read by the `scout:report` swipe page. Written atomically (temp
//! file + rename; the previous file kept as `slr-scout.json.bak`).

use serde::{Deserialize, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "slr-scout.json";
pub const BACKUP_NAME: &str = "slr-scout.json.bak";

/// A performer as SLR knows her.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Actor {
    pub name: String,
    pub id: u64,
    pub label: String,
    /// Her scene count on SLR.
    pub scenes: u64,
}

/// A hand-made script for a scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum Handmade {
    /// A hand-crafted (not AI) script on SLR itself.
    Slr { author: String },
    /// An EroScripts post.
    #[serde(rename_all = "camelCase")]
    Eroscripts {
        topic_id: u64,
        title: String,
        free: bool,
        multi_axis: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    pub id: u64,
    pub label: String,
    pub url: String,
    pub title: String,
    pub studio: Option<String>,
    pub actors: Vec<String>,
    /// `YYYY-MM-DD` (UTC).
    pub date: Option<String>,
    pub duration_s: Option<u64>,
    pub thumbnail: Option<String>,
    pub vr: bool,
    pub fps: Option<serde_json::Number>,
    pub passthrough: bool,
    pub creampie: bool,
    pub handmade: Vec<Handmade>,
    pub ai_script: bool,
    pub pmv: bool,
    pub compilation: bool,
    pub favorites: u64,
    #[serde(serialize_with = "number")]
    pub score: f64,
    #[serde(serialize_with = "numbers")]
    pub score_parts: BTreeMap<String, f64>,
}

/// One download allowance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limit {
    pub remaining: i64,
    pub total: i64,
    /// When the oldest counted download stops counting (epoch seconds).
    pub usage_expires_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quota {
    pub checked_at: String,
    /// Hand-crafted scripts per month.
    pub scripts: Limit,
    /// AI scripts per month.
    pub ai_scripts: Limit,
    /// Videos per 7 days.
    pub videos: Limit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoutFile {
    pub generated_at: String,
    pub actors: Vec<Actor>,
    pub quota: Option<Quota>,
    pub quota_error: Option<String>,
    pub scenes: Vec<Scene>,
}

impl ScoutFile {
    /// The previous file's handmade EroScripts entries for scene `id`.
    pub fn forum_matches(&self, id: u64) -> Vec<Handmade> {
        self.scenes
            .iter()
            .find(|s| s.id == id)
            .map(|s| {
                s.handmade
                    .iter()
                    .filter(|h| matches!(h, Handmade::Eroscripts { .. }))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Whole numbers as integers (`4`, not `4.0`), the way the app writes them.
fn number<S: Serializer>(x: &f64, s: S) -> Result<S::Ok, S::Error> {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        s.serialize_i64(*x as i64)
    } else {
        s.serialize_f64(*x)
    }
}

fn numbers<S: Serializer>(map: &BTreeMap<String, f64>, s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut m = s.serialize_map(Some(map.len()))?;
    for (k, v) in map {
        struct N(f64);
        impl Serialize for N {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                number(&self.0, s)
            }
        }
        m.serialize_entry(k, &N(*v))?;
    }
    m.end()
}

pub fn file_path(user_data: &Path) -> PathBuf {
    user_data.join(FILE_NAME)
}

/// The previous file: `Ok(None)` when there is none; an error when it can't
/// be read or isn't this CLI's file.
pub fn read(path: &Path) -> Result<Option<ScoutFile>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    serde_json::from_str(&raw).map(Some).map_err(|e| {
        format!(
            "{} is not an slr-scout file ({e}); move it aside and re-run",
            path.display()
        )
    })
}

/// Write the file atomically; returns whether the previous one was backed up.
pub fn write(user_data: &Path, file: &ScoutFile) -> Result<bool, String> {
    video_tags::store::write_json_atomic(user_data, FILE_NAME, BACKUP_NAME, file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub fn scene(id: u64) -> Scene {
        Scene {
            id,
            label: format!("s-{id}"),
            url: format!("https://www.sexlikereal.com/scenes/s-{id}"),
            title: "T".into(),
            studio: Some("Miss Lexa VR".into()),
            actors: vec!["Miss Lexa".into()],
            date: Some("2026-04-24".into()),
            duration_s: Some(1629),
            thumbnail: None,
            vr: true,
            fps: Some(60.into()),
            passthrough: false,
            creampie: true,
            handmade: vec![
                Handmade::Slr {
                    author: "kimera".into(),
                },
                Handmade::Eroscripts {
                    topic_id: 317875,
                    title: "Latina".into(),
                    free: false,
                    multi_axis: false,
                },
            ],
            ai_script: true,
            pmv: false,
            compilation: false,
            favorites: 1123,
            score: 5.12,
            score_parts: BTreeMap::from([
                ("handmadeScript".into(), 4.0),
                ("popularity".into(), 1.12),
            ]),
        }
    }

    #[test]
    fn serialises_in_the_contract_shape() {
        let v = serde_json::to_value(scene(85666)).unwrap();
        assert_eq!(
            v["handmade"],
            json!([
                {"source": "slr", "author": "kimera"},
                {"source": "eroscripts", "topicId": 317875, "title": "Latina", "free": false, "multiAxis": false}
            ])
        );
        assert_eq!(v["durationS"], 1629);
        assert_eq!(v["aiScript"], true);
        assert_eq!(v["fps"], 60);
        let text = serde_json::to_string(&scene(1)).unwrap();
        assert!(text.contains(r#""scoreParts":{"handmadeScript":4,"popularity":1.12}"#));
        assert!(text.contains(r#""score":5.12"#));
        let file = ScoutFile {
            generated_at: "2026-11-13T00:00:00.000Z".into(),
            actors: vec![],
            quota: None,
            quota_error: Some("no session".into()),
            scenes: vec![scene(1)],
        };
        let v = serde_json::to_value(&file).unwrap();
        assert_eq!(v["quota"], json!(null));
        assert_eq!(v["quotaError"], "no session");
        let back: ScoutFile = serde_json::from_value(v).unwrap();
        assert_eq!(back, file);
    }

    #[test]
    fn previous_forum_matches_by_scene() {
        let file = ScoutFile {
            generated_at: String::new(),
            actors: vec![],
            quota: None,
            quota_error: None,
            scenes: vec![scene(7)],
        };
        assert_eq!(file.forum_matches(7).len(), 1);
        assert!(matches!(
            file.forum_matches(7)[0],
            Handmade::Eroscripts {
                topic_id: 317875,
                ..
            }
        ));
        assert!(file.forum_matches(8).is_empty());
    }
}
