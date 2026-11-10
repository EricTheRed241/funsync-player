//! The pornstar list and threshold, from the shared part of the content
//! preferences (`pornstarTags` in `renderer/js/content-preferences.json`).
//! Embedded at compile time, so `cargo run` rebuilds when the file changes;
//! no list lives in Rust.

use crate::text;
use serde::Deserialize;
use std::collections::HashSet;

const SHARED: &str = include_str!("../../../renderer/js/content-preferences.json");

/// One listed performer.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Listed {
    pub name: String,
    /// The tag; the slug of `name` when absent.
    #[serde(default)]
    pub tag: Option<String>,
    /// Other spellings of the name.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Scene titles that identify her without naming her.
    #[serde(default)]
    pub scenes: Vec<String>,
}

impl Listed {
    pub fn tag(&self) -> String {
        self.tag.clone().unwrap_or_else(|| text::slug(&self.name))
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PornstarTags {
    /// A tag is applied once this many scene folders show the performer.
    pub min_scenes: usize,
    pub pornstars: Vec<Listed>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Shared {
    pornstar_tags: PornstarTags,
}

/// The embedded preferences.
pub fn load() -> Result<PornstarTags, String> {
    parse(SHARED)
}

pub fn parse(raw: &str) -> Result<PornstarTags, String> {
    let prefs = serde_json::from_str::<Shared>(raw)
        .map_err(|e| format!("renderer/js/content-preferences.json: {e}"))?
        .pornstar_tags;
    let bad = |what: String| Err(format!("content-preferences.json: pornstarTags: {what}"));
    if prefs.min_scenes == 0 {
        return bad("minScenes must be 1 or more".into());
    }
    let mut tags = HashSet::new();
    for p in &prefs.pornstars {
        let tag = p.tag();
        if tag.is_empty() || text::slug(&tag) != tag {
            return bad(format!(
                "\"{}\" needs a tag of lower-case words joined by '-' (got \"{tag}\")",
                p.name
            ));
        }
        if !tags.insert(tag.clone()) {
            return bad(format!("tag \"{tag}\" is listed twice"));
        }
    }
    Ok(prefs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_list_loads() {
        let prefs = load().unwrap();
        assert_eq!(prefs.min_scenes, 2);
        let alice = prefs
            .pornstars
            .iter()
            .find(|p| p.name == "MissAliceNZ")
            .unwrap();
        assert_eq!(alice.tag(), "alicenz");
        let candy = &prefs.pornstars[0];
        assert_eq!(candy.tag(), "candy-love");
    }

    #[test]
    fn rejects_bad_values() {
        let wrap = |inner: &str| format!("{{\"pornstarTags\": {inner}}}");
        assert!(parse(&wrap(r#"{"minScenes": 0, "pornstars": []}"#)).is_err());
        assert!(
            parse(&wrap(
                r#"{"minScenes": 2, "pornstars": [{"name": "A B"}, {"name": "a-b"}]}"#
            ))
            .is_err()
        );
        assert!(
            parse(&wrap(
                r#"{"minScenes": 2, "pornstars": [{"name": "A", "tag": "Not A Slug"}]}"#
            ))
            .is_err()
        );
        assert!(parse(&wrap(r#"{"minScenes": 2, "pornstars": [{"name": "!!"}]}"#)).is_err());
    }
}
