//! What the library already has (posts marked `inLibrary`), and how posts
//! are scored: the EroScripts deck's `candidateScore` (slr-scout's port),
//! with the user's tag scores and favourites from config.json.

use crate::store::{Creator, CreatorsFile, Post};
use serde_json::Value;
use slr_scout::library::{self as lib, Source, UserConfig};
use slr_scout::prefs::Prefs;
use slr_scout::score::{Candidate, Scorer};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub const LINKS_FILE: &str = "eroscripts-links.json";
const SIDECAR_DEPTH: u32 = 4;

/// EroScripts topics and FapTap videos the library holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LibraryIds {
    pub topics: HashSet<u64>,
    pub faptap: HashSet<String>,
}

impl LibraryIds {
    /// Topic ids in `eroscripts-links.json` (`links.<path>.topicId`), and
    /// the `eroscripts.json` (`topicId`) and `faptap.json` (`faptapId`)
    /// sidecars in the enabled sources, four levels down (dot names, so
    /// AppleDouble `._*` files, skipped).
    pub fn scan(user_data: &Path, sources: &[Source]) -> LibraryIds {
        let mut ids = LibraryIds::default();
        if let Some(links) = fs::read_to_string(user_data.join(LINKS_FILE))
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        {
            for link in links["links"].as_object().into_iter().flatten() {
                if let Some(id) = topic_id(&link.1["topicId"]) {
                    ids.topics.insert(id);
                }
            }
        }
        for s in lib::enabled(sources) {
            ids.walk(&s.path, 0);
        }
        ids
    }

    fn walk(&mut self, dir: &Path, depth: u32) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let Ok(name) = e.file_name().into_string() else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = e.file_type() else { continue };
            if kind.is_dir() {
                if depth < SIDECAR_DEPTH {
                    self.walk(&e.path(), depth + 1);
                }
                continue;
            }
            if name != "eroscripts.json" && name != "faptap.json" {
                continue;
            }
            let Some(v) = fs::read_to_string(e.path())
                .ok()
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            else {
                continue;
            };
            if name == "eroscripts.json" {
                if let Some(id) = topic_id(&v["topicId"]) {
                    self.topics.insert(id);
                }
            } else if let Some(id) = id_text(&v["faptapId"]) {
                self.faptap.insert(id);
            }
        }
    }

    pub fn has(&self, post: &Post) -> bool {
        match post.kind.as_str() {
            "eroscripts" => post.id.parse().is_ok_and(|id| self.topics.contains(&id)),
            "faptap" => self.faptap.contains(&post.id),
            _ => false,
        }
    }

    /// Set every post's `inLibrary` (a post once in the library stays so);
    /// returns how many changed.
    pub fn mark(&self, file: &mut CreatorsFile) -> usize {
        file.creators
            .values_mut()
            .map(|c| self.mark_creator(c))
            .sum()
    }

    /// [`mark`](Self::mark) for one creator.
    pub fn mark_creator(&self, c: &mut Creator) -> usize {
        let mut n = 0;
        for p in &mut c.posts {
            if !p.in_library && self.has(p) {
                p.in_library = true;
                n += 1;
            }
        }
        n
    }
}

fn topic_id(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

fn id_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The deck's score for posts.
pub struct Scoring {
    scorer: Scorer,
    likes_per: f64,
}

impl Scoring {
    pub fn new(prefs: &Prefs, user: &UserConfig) -> Scoring {
        Scoring {
            scorer: Scorer::new(prefs, user, lib::creators(&user.sources)),
            likes_per: prefs.ranking.likes_per,
        }
    }

    /// `candidateScore` of a post by `creator` (flat video: never `vr`),
    /// two decimals (never `-0`).
    pub fn score(&self, post: &Post, creator: &str) -> f64 {
        let favourite = self.scorer.favourite_in([creator]).is_some();
        self.scorer.candidate_score(
            &Candidate {
                tags: &post.tags,
                title: &post.title,
                vr: false,
                artist_in_library: self.scorer.in_library(creator),
                favourite,
                multi_axis: post.multi_axis,
                likes: post.likes as f64,
            },
            self.likes_per,
        ) + 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn finds_links_and_sidecars_but_not_appledouble() {
        let dir = std::env::temp_dir().join(format!("creator-scout-lib-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let src = dir.join("lib");
        fs::create_dir_all(src.join("EroScripts/A/scene")).unwrap();
        fs::create_dir_all(src.join("FapTap/B/scene")).unwrap();
        fs::write(
            src.join("EroScripts/A/scene/eroscripts.json"),
            r#"{"topicId": 41415}"#,
        )
        .unwrap();
        fs::write(
            src.join("EroScripts/A/scene/._eroscripts.json"),
            r#"{"topicId": 7}"#,
        )
        .unwrap();
        fs::write(
            src.join("FapTap/B/scene/faptap.json"),
            r#"{"faptapId": "2122637633308856321"}"#,
        )
        .unwrap();
        fs::write(
            dir.join(LINKS_FILE),
            r#"{"version":1,"links":{"/v.mp4":{"topicId":331821}},"review":{"/w.mp4":{"topicId":5}}}"#,
        )
        .unwrap();
        let sources = vec![Source {
            name: None,
            path: PathBuf::from(&src),
            enabled: true,
        }];
        let ids = LibraryIds::scan(&dir, &sources);
        let mut topics: Vec<u64> = ids.topics.iter().copied().collect();
        topics.sort();
        assert_eq!(topics, [41415, 331821]);
        assert!(ids.faptap.contains("2122637633308856321"));
        let post = |kind: &str, id: &str| Post {
            kind: kind.into(),
            id: id.into(),
            ..Post::default()
        };
        assert!(ids.has(&post("eroscripts", "41415")));
        assert!(!ids.has(&post("eroscripts", "7")));
        assert!(ids.has(&post("faptap", "2122637633308856321")));
        let _ = fs::remove_dir_all(&dir);
    }
}
