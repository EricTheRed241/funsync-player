//! The Content creators tab (`/creators`): Instagram and Twitch creators who
//! also have EroScripts posts or FapTap videos, read from
//! `content-creators.json` in userData (written elsewhere; read-only here).
//! Unknown fields are ignored and missing ones default.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

pub const FILE: &str = "content-creators.json";

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Creators {
    pub version: u32,
    pub updated_at: Option<String>,
    pub creators: Vec<Creator>,
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Creator {
    pub name: String,
    pub score: Option<f64>,
    pub instagram: Option<Instagram>,
    pub twitch: Option<Twitch>,
    pub eroscripts: Vec<Post>,
    pub faptap: Vec<FapTap>,
}

/// `followers` and `posts` are kept as given ("5M", or a number).
#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Instagram {
    pub handle: String,
    pub followers: Value,
    pub posts: Value,
    #[serde(rename = "match")]
    pub confidence: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Twitch {
    pub handle: String,
    pub followers: Value,
    pub last_live: Option<String>,
    #[serde(rename = "match")]
    pub confidence: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Post {
    pub id: u64,
    pub title: String,
    pub free: Option<bool>,
    pub hosts: Vec<String>,
    pub downloadable: bool,
    pub creampie: bool,
    pub multi_axis: bool,
    pub score: Option<f64>,
    pub in_library: bool,
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct FapTap {
    /// A string in the file (too long for a JS number); numbers are accepted too.
    pub id: Value,
    pub title: String,
    pub url: String,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "scout-report-creators-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn parses_a_full_file_ignoring_unknown_fields() {
        let d = dir("full");
        std::fs::write(
            d.join(FILE),
            r#"{"version":1,"updatedAt":"2026-11-17T21:00:00Z","extra":true,"creators":[{
              "name":"Sweetie Fox","score":6.1,"nickname":"x",
              "instagram":{"handle":"swfx_real","followers":"5M","posts":"225","match":"high"},
              "twitch":{"handle":"sweetiefoxxx","followers":9225,"lastLive":"2020-09-11T18:17:44Z","match":"medium"},
              "eroscripts":[{"id":12345,"title":"T","free":true,"hosts":["pixeldrain","mega"],"downloadable":true,
                "creampie":false,"multiAxis":true,"score":7.5,"inLibrary":false}],
              "faptap":[{"id":"2122637633308856321","title":"F","url":"https://faptap.net/v/2122637633308856321"}]}]}"#,
        )
        .unwrap();
        let c = load(&d).unwrap().unwrap();
        assert_eq!(c.updated_at.as_deref(), Some("2026-11-17T21:00:00Z"));
        let fox = &c.creators[0];
        assert_eq!(fox.score, Some(6.1));
        let ig = fox.instagram.as_ref().unwrap();
        assert_eq!(
            (ig.handle.as_str(), &ig.followers),
            ("swfx_real", &Value::from("5M"))
        );
        let tw = fox.twitch.as_ref().unwrap();
        assert_eq!(tw.confidence.as_deref(), Some("medium"));
        assert_eq!(tw.followers, Value::from(9225));
        let p = &fox.eroscripts[0];
        assert!(p.downloadable && p.multi_axis && !p.in_library);
        assert_eq!(p.hosts, ["pixeldrain", "mega"]);
        assert_eq!(fox.faptap[0].id, Value::from("2122637633308856321"));
        // Serialized back with the file's names.
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(
            v["creators"][0]["twitch"]["lastLive"],
            "2020-09-11T18:17:44Z"
        );
        assert_eq!(v["creators"][0]["instagram"]["match"], "high");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn tolerates_minimal_entries_and_nulls() {
        let d = dir("minimal");
        std::fs::write(
            d.join(FILE),
            r#"{"creators":[{"name":"A"},{"name":"B","score":null,"instagram":null,
              "twitch":{"handle":"b","lastLive":null},"eroscripts":[{"id":1}]}]}"#,
        )
        .unwrap();
        let c = load(&d).unwrap().unwrap();
        assert_eq!(c.updated_at, None);
        assert_eq!(
            c.creators[0],
            Creator {
                name: "A".into(),
                ..Default::default()
            }
        );
        let b = &c.creators[1];
        assert_eq!(b.twitch.as_ref().unwrap().last_live, None);
        assert!(!b.eroscripts[0].downloadable);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn missing_and_broken_files() {
        let d = dir("missing");
        assert_eq!(load(&d), Ok(None));
        std::fs::write(d.join(FILE), "{not json").unwrap();
        assert!(load(&d).unwrap_err().starts_with(FILE));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
