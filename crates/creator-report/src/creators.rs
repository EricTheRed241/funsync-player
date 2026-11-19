//! `<userData>/creators.json`, written by creator-scout only (read-only here): the
//! creators, by slug, with their Instagram and Twitch accounts, pictures and posts.
//! Unknown fields are ignored; missing or null ones default. Cache keeps the parsed
//! file and reads it again only when its modification time or size changes.

use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

pub const FILE: &str = "creators.json";

/// A missing or `null` value as the type's default.
fn nullable<'de, D: Deserializer<'de>, T: Default + Deserialize<'de>>(d: D) -> Result<T, D::Error> {
    Option::<T>::deserialize(d).map(Option::unwrap_or_default)
}

/// An id as text: FapTap ids are strings (too long for a JS number), forum ids may be numbers.
fn id_text<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => s,
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    })
}

#[derive(Debug, Default, Deserialize, Clone, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Creators {
    pub version: u32,
    pub updated_at: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub creators: BTreeMap<String, Creator>,
}

#[derive(Debug, Default, Deserialize, Clone, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Creator {
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    #[serde(deserialize_with = "nullable")]
    pub aliases: Vec<String>,
    pub first_seen: Option<String>,
    /// The average post score; None when no post is scored.
    pub score: Option<f64>,
    pub instagram: Option<Instagram>,
    pub twitch: Option<Twitch>,
    /// Newest first; files under `creator-pictures/<slug>/`.
    #[serde(deserialize_with = "nullable")]
    pub pictures: Vec<Picture>,
    #[serde(deserialize_with = "nullable")]
    pub posts: Vec<Post>,
    /// Why the user wants her (creator-scout --profiles): `swiped`, `favourite` (a
    /// favourite artist), `pornstar` (the pornstar list); empty when only discovered.
    #[serde(deserialize_with = "nullable")]
    pub sources: Vec<String>,
    /// Her other public profiles creator-scout confirmed hers (OnlyFans, Fansly, …).
    #[serde(deserialize_with = "nullable")]
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Default, Deserialize, Clone, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Profile {
    /// onlyfans / fansly / pornhub / x / reddit / manyvids / linktree / iafd /
    /// babepedia / boobpedia / freeones / profile.
    #[serde(deserialize_with = "nullable")]
    pub site: String,
    #[serde(deserialize_with = "nullable")]
    pub url: String,
    #[serde(deserialize_with = "nullable")]
    pub handle: String,
}

/// `followers` and `posts` are kept as given ("5M", or a number).
#[derive(Debug, Default, Deserialize, Clone, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Instagram {
    #[serde(deserialize_with = "nullable")]
    pub handle: String,
    pub followers: Value,
    pub posts: Value,
    pub full_name: Option<String>,
    /// None until creator-scout has an Instagram login.
    pub last_post_at: Option<String>,
    /// Our Instagram account follows her: `following` / `requested`.
    pub followed: Option<String>,
}

#[derive(Debug, Default, Deserialize, Clone, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Twitch {
    #[serde(deserialize_with = "nullable")]
    pub handle: String,
    pub followers: Value,
    pub last_live: Option<String>,
}

#[derive(Debug, Default, Deserialize, Clone, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Picture {
    /// A file name in `creator-pictures/<slug>/`.
    #[serde(deserialize_with = "nullable")]
    pub file: String,
    /// `instagram`, `eroscripts` or `faptap`.
    #[serde(deserialize_with = "nullable")]
    pub source: String,
    /// Where the picture comes from (an Instagram post, a forum topic, a FapTap video).
    pub url: Option<String>,
}

#[derive(Debug, Default, Deserialize, Clone, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Post {
    /// `eroscripts` or `faptap`.
    #[serde(deserialize_with = "nullable")]
    pub kind: String,
    #[serde(deserialize_with = "id_text")]
    pub id: String,
    #[serde(deserialize_with = "nullable")]
    pub title: String,
    #[serde(deserialize_with = "nullable")]
    pub url: String,
    #[serde(deserialize_with = "nullable")]
    pub free: bool,
    #[serde(deserialize_with = "nullable")]
    pub hosts: Vec<String>,
    /// A free post whose video the scout can fetch, or any FapTap video.
    #[serde(deserialize_with = "nullable")]
    pub downloadable: bool,
    #[serde(deserialize_with = "nullable")]
    pub creampie: bool,
    #[serde(deserialize_with = "nullable")]
    pub multi_axis: bool,
    pub score: Option<f64>,
    pub likes: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    pub tags: Vec<String>,
    /// A file name in `creator-pictures/<slug>/`.
    pub image: Option<String>,
    pub duration_s: Option<f64>,
    #[serde(deserialize_with = "nullable")]
    pub in_library: bool,
    pub created_at: Option<String>,
}

impl Post {
    /// Its key in creator-swipes.json: `<kind>:<id>`.
    pub fn key(&self) -> String {
        format!("{}:{}", self.kind, self.id)
    }

    pub fn is_eroscripts(&self) -> bool {
        self.kind == "eroscripts"
    }

    pub fn is_faptap(&self) -> bool {
        self.kind == "faptap"
    }
}

/// The file: `Ok(None)` when it doesn't exist, `Err` when it can't be read or parsed.
pub fn load(user_data: &Path) -> Result<Option<Creators>, String> {
    match std::fs::read(user_data.join(FILE)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("{FILE}: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{FILE}: {e}")),
    }
}

/// The file as last read, and what it looked like then (modification time, size).
#[derive(Debug, Default)]
pub struct Cache {
    stamp: Option<(Option<SystemTime>, u64)>,
    loaded: Option<Result<Option<Creators>, String>>,
}

impl Cache {
    /// The file, read again when its modification time or size changed since the last
    /// call (or it appeared or went away). A parse error is kept until the file changes.
    pub fn get(&mut self, user_data: &Path) -> Result<Option<&Creators>, String> {
        let stamp = match std::fs::metadata(user_data.join(FILE)) {
            Ok(m) => Some((m.modified().ok(), m.len())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("{FILE}: {e}")),
        };
        if self.loaded.is_none() || stamp != self.stamp {
            self.stamp = stamp;
            self.loaded = Some(load(user_data));
        }
        match self.loaded.as_ref() {
            Some(Ok(c)) => Ok(c.as_ref()),
            Some(Err(e)) => Err(e.clone()),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "creator-report-creators-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn parses_the_contract_shape_ignoring_unknown_fields() {
        let d = dir("full");
        std::fs::write(
            d.join(FILE),
            r#"{"version":1,"updatedAt":"2026-11-18T01:40:00Z","eroscriptsChecked":{"newestTopicId":348600},
              "creators":{"sweetiefox":{"name":"Sweetie Fox","aliases":["SweetieFox"],"score":6.1,"extra":1,"sources":["favourite","pornstar"],
                "instagram":{"handle":"swfx_real","followers":"5M","posts":"225","lastPostAt":null,"match":"high","followed":"following"},
                "profiles":[{"site":"onlyfans","url":"https://onlyfans.com/sf","handle":"sf","match":"high","checkedAt":"t"}],
                "twitch":{"handle":"sweetiefoxxx","followers":9225,"lastLive":"2020-09-11T18:17:44Z"},
                "pictures":[{"file":"ig-1.jpg","source":"instagram","url":"https://www.instagram.com/p/x/"}],
                "posts":[{"kind":"eroscripts","id":"41415","title":"T","free":true,"hosts":["pornhub"],
                  "downloadable":true,"creampie":true,"score":7.0,"likes":108,"image":"es-41415-0.jpg","durationS":675},
                  {"kind":"faptap","id":2122637633308856321,"title":"F","tags":null}]}}}"#,
        )
        .unwrap();
        let c = load(&d).unwrap().unwrap();
        assert_eq!(c.updated_at.as_deref(), Some("2026-11-18T01:40:00Z"));
        let fox = &c.creators["sweetiefox"];
        assert_eq!(fox.score, Some(6.1));
        assert_eq!(fox.sources, ["favourite", "pornstar"]);
        assert_eq!(fox.profiles[0].site, "onlyfans");
        assert_eq!(fox.profiles[0].url, "https://onlyfans.com/sf");
        assert_eq!(
            fox.instagram.as_ref().unwrap().followed.as_deref(),
            Some("following")
        );
        let ig = fox.instagram.as_ref().unwrap();
        assert_eq!(ig.followers, Value::from("5M"));
        assert_eq!(ig.last_post_at, None);
        assert_eq!(fox.twitch.as_ref().unwrap().followers, Value::from(9225));
        assert_eq!(fox.posts[0].key(), "eroscripts:41415");
        assert_eq!(fox.posts[0].duration_s, Some(675.0));
        assert_eq!(fox.posts[1].key(), "faptap:2122637633308856321");
        assert!(fox.posts[1].tags.is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn tolerates_minimal_entries_and_nulls() {
        let d = dir("minimal");
        std::fs::write(
            d.join(FILE),
            r#"{"creators":{"a":{},"b":{"name":null,"pictures":null,"posts":[{"id":1,"free":null}]}}}"#,
        )
        .unwrap();
        let c = load(&d).unwrap().unwrap();
        assert_eq!(c.creators["a"], Creator::default());
        assert_eq!(c.creators["b"].posts[0].id, "1");
        assert!(!c.creators["b"].posts[0].free);
        std::fs::write(d.join(FILE), r#"{"creators":null}"#).unwrap();
        assert!(load(&d).unwrap().unwrap().creators.is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_cache_rereads_when_the_file_changes() {
        let d = dir("cache");
        let mut cache = Cache::default();
        assert_eq!(cache.get(&d), Ok(None));
        std::fs::write(d.join(FILE), r#"{"creators":{"a":{"name":"A"}}}"#).unwrap();
        assert_eq!(cache.get(&d).unwrap().unwrap().creators["a"].name, "A");
        // Another size: read again.
        std::fs::write(d.join(FILE), r#"{"creators":{"a":{"name":"AB"}}}"#).unwrap();
        assert_eq!(cache.get(&d).unwrap().unwrap().creators["a"].name, "AB");
        std::fs::write(d.join(FILE), "{oops").unwrap();
        assert!(cache.get(&d).unwrap_err().starts_with(FILE));
        std::fs::remove_file(d.join(FILE)).unwrap();
        assert_eq!(cache.get(&d), Ok(None));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
