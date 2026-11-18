//! `--import-seed`: the research snapshot (`content-creators.json`:
//! `{version, updatedAt, creators: [{name, score, instagram, twitch,
//! eroscripts: [...], faptap: [...]}]}`) merged into creators.json. Nothing
//! existing is dropped: a known creator gains the snapshot's missing posts
//! and accounts.

use crate::store::{self, Creator, CreatorsFile, Instagram, Post, Twitch};
use serde_json::{Map, Value};

#[derive(Debug, Default, PartialEq)]
pub struct Imported {
    pub creators_added: usize,
    pub creators_merged: usize,
    pub posts_added: usize,
    pub skipped: Vec<String>,
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(text)
        .collect()
}

fn instagram(v: &Value, at: Option<&str>) -> Option<Instagram> {
    let handle = text(&v["handle"])?;
    Some(Instagram {
        handle: handle.trim_start_matches('@').to_string(),
        followers: text(&v["followers"]),
        posts: text(&v["posts"]),
        full_name: text(&v["fullName"]),
        bio: text(&v["bio"]),
        last_post_at: text(&v["lastPostAt"]),
        match_: text(&v["match"]),
        checked_at: at.map(str::to_string),
        followed: None,
        extra: Map::new(),
    })
}

fn twitch(v: &Value, at: Option<&str>) -> Option<Twitch> {
    let handle = text(&v["handle"])?;
    Some(Twitch {
        handle,
        followers: v["followers"].as_u64(),
        last_live: text(&v["lastLive"]),
        match_: text(&v["match"]),
        checked_at: at.map(str::to_string),
        extra: Map::new(),
    })
}

fn eroscripts_post(v: &Value) -> Option<Post> {
    let id = text(&v["id"])?;
    Some(Post {
        kind: "eroscripts".into(),
        url: format!("{}/t/{id}", store::FORUM_SITE),
        id,
        title: text(&v["title"]).unwrap_or_default(),
        free: v["free"].as_bool().unwrap_or(false),
        hosts: strings(&v["hosts"]),
        downloadable: v["downloadable"].as_bool().unwrap_or(false),
        creampie: v["creampie"].as_bool().unwrap_or(false),
        multi_axis: v["multiAxis"].as_bool().unwrap_or(false),
        score: v["score"].as_f64(),
        likes: v["likes"].as_u64().unwrap_or(0),
        tags: strings(&v["tags"]),
        image: None,
        duration_s: v["duration"].as_u64(),
        in_library: v["inLibrary"].as_bool().unwrap_or(false),
        created_at: None,
        video_pages: Vec::new(),
        extra: Map::new(),
    })
}

fn faptap_post(v: &Value) -> Option<Post> {
    let id = text(&v["id"])?;
    let title = text(&v["title"]).unwrap_or_default();
    Some(Post {
        kind: "faptap".into(),
        url: text(&v["url"]).unwrap_or_else(|| format!("{}/v/{id}", store::FAPTAP_SITE)),
        id,
        creampie: title.to_lowercase().contains("creampie"),
        title,
        free: true,
        hosts: vec!["faptap".into()],
        downloadable: true,
        multi_axis: false,
        score: None,
        likes: 0,
        tags: Vec::new(),
        image: None,
        duration_s: v["duration"].as_u64(),
        in_library: false,
        created_at: None,
        video_pages: Vec::new(),
        extra: Map::new(),
    })
}

/// Merge the snapshot `raw` into `file`; `now` is the import time (a new
/// creator's `firstSeen` when the snapshot has no date).
pub fn import(raw: &str, file: &mut CreatorsFile, now: &str) -> Result<Imported, String> {
    let v: Value = serde_json::from_str(raw).map_err(|e| format!("not JSON ({e})"))?;
    let list = v["creators"]
        .as_array()
        .ok_or("no \"creators\" list: not a content-creators.json snapshot")?;
    let at = text(&v["updatedAt"]);
    let at = at.as_deref();
    let mut out = Imported::default();
    for entry in list {
        let Some(name) = text(&entry["name"]) else {
            out.skipped.push("(no name)".into());
            continue;
        };
        let posts: Vec<Post> = entry["eroscripts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(eroscripts_post)
            .chain(
                entry["faptap"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(faptap_post),
            )
            .collect();
        let key = match file.find(&name) {
            Some(key) => {
                out.creators_merged += 1;
                key
            }
            None => {
                let key = store::slug(&name);
                if key.is_empty() {
                    out.skipped.push(name);
                    continue;
                }
                file.creators.insert(
                    key.clone(),
                    Creator {
                        name: name.clone(),
                        first_seen: Some(at.unwrap_or(now).to_string()),
                        ..Creator::default()
                    },
                );
                out.creators_added += 1;
                key
            }
        };
        let c = file.creators.get_mut(&key).ok_or("creator vanished")?;
        if c.instagram.is_none() {
            c.instagram = instagram(&entry["instagram"], at);
        }
        if c.twitch.is_none() {
            c.twitch = twitch(&entry["twitch"], at);
        }
        for p in posts {
            if c.post(&p.kind, &p.id).is_none() {
                c.posts.push(p);
                out.posts_added += 1;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &str = r#"{"version":1,"updatedAt":"2026-11-17T21:47:54Z","creators":[
      {"name":"HannahOwO","score":7.0,
       "instagram":{"handle":"itshannahowo","followers":"668K","posts":"39","match":"high"},"twitch":null,
       "eroscripts":[{"id":260228,"title":"[OnlyFans] HannahOwO - Blowjob","free":true,"hosts":["pixeldrain","mega"],
                      "downloadable":true,"creampie":false,"multiAxis":true,"score":7,"inLibrary":true}],
       "faptap":[{"id":"2047467413527597057","title":"Hannah Owo Riding","url":"https://faptap.net/v/2047467413527597057","duration":300}]},
      {"name":"Lucy Cat","score":5,"instagram":{"handle":"lucycat","followers":"1M","posts":"10","match":"high"},
       "twitch":{"handle":"lucycat","followers":3716,"lastLive":"2020-05-05T18:32:48Z","match":"high"},
       "eroscripts":[],"faptap":[]},
      {"name":"!!!"}
    ]}"#;

    #[test]
    fn imports_and_merges_without_dropping() {
        let mut f = CreatorsFile::empty();
        let r = import(SEED, &mut f, "NOW").unwrap();
        assert_eq!(r.creators_added, 2);
        assert_eq!(r.posts_added, 2);
        assert_eq!(r.skipped, ["!!!"]);
        let h = &f.creators["hannahowo"];
        assert_eq!(h.first_seen.as_deref(), Some("2026-11-17T21:47:54Z"));
        let ig = h.instagram.as_ref().unwrap();
        assert_eq!(
            (ig.handle.as_str(), ig.followers.as_deref()),
            ("itshannahowo", Some("668K"))
        );
        assert_eq!(ig.last_post_at, None);
        let es = h.post("eroscripts", "260228").unwrap();
        assert_eq!(es.url, "https://discuss.eroscripts.com/t/260228");
        assert!(es.in_library && es.multi_axis && es.downloadable);
        assert_eq!(es.score, Some(7.0));
        let ft = h.post("faptap", "2047467413527597057").unwrap();
        assert_eq!(ft.hosts, ["faptap"]);
        assert_eq!(ft.duration_s, Some(300));
        let t = f.creators["lucycat"].twitch.as_ref().unwrap();
        assert_eq!((t.handle.as_str(), t.followers), ("lucycat", Some(3716)));

        // A second import (or a newer snapshot) keeps what is there.
        f.creators.get_mut("hannahowo").unwrap().posts[0].title = "kept".into();
        f.creators.get_mut("lucycat").unwrap().aliases = vec!["LucyCatX".into()];
        let r = import(SEED, &mut f, "NOW").unwrap();
        assert_eq!(
            (r.creators_added, r.creators_merged, r.posts_added),
            (0, 2, 0)
        );
        assert_eq!(f.creators["hannahowo"].posts[0].title, "kept");
        assert_eq!(f.creators.len(), 2);
        assert!(import("{}", &mut f, "NOW").is_err());
    }
}
