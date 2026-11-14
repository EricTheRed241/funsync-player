//! Hand-made scripts on EroScripts: search the forum for each performer
//! (read-only, with the app's session cookie, paced ≥ 4 s) and match script
//! posts to scenes by title.

use crate::http::{self, Http, NetError, Pacer, Request};
use crate::store::{Handmade, Scene};
use crate::text;
use serde_json::Value;
use std::io::Write;

pub const FORUM: &str = "https://discuss.eroscripts.com";
/// Free Scripts / Paid Scripts.
pub const FREE_CATEGORY: u64 = 14;
pub const PAID_CATEGORY: u64 = 15;
/// Stop a search after this many result pages.
const MAX_PAGES: u32 = 20;

/// The forum session cookie (`settings.eroscripts.session.cookie` in
/// config.json, the bare `_t` value). Never printed.
pub fn session_cookie(config: &Value) -> Option<String> {
    let settings = config.get("settings").unwrap_or(config);
    let raw = settings
        .pointer("/eroscripts/session/cookie")?
        .as_str()?
        .trim();
    let value = raw.strip_prefix("_t=").unwrap_or(raw);
    (!value.is_empty()).then(|| value.to_string())
}

/// The searches for a performer: her name as a phrase, and the name run
/// together (`"Miss Lexa"`, `misslexa`).
pub fn queries(actor: &str) -> Vec<String> {
    let mut out = vec![format!("\"{actor}\"")];
    let joined: String = text::normalise(actor).replace(' ', "");
    if !joined.is_empty() && text::normalise(actor).contains(' ') {
        out.push(joined);
    }
    out
}

/// A script post.
#[derive(Debug, Clone, PartialEq)]
pub struct Topic {
    pub id: u64,
    pub title: String,
    pub free: bool,
    pub multi_axis: bool,
}

/// Tag names; Discourse gives strings or `{name}` objects.
fn tags(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_str().or_else(|| t["name"].as_str()))
        .map(str::to_ascii_lowercase)
        .collect()
}

/// A search result topic, when it's a script post (Free or Paid Scripts)
/// that isn't tagged AI-generated.
pub fn topic(v: &Value) -> Option<Topic> {
    let free = match v["category_id"].as_u64()? {
        FREE_CATEGORY => true,
        PAID_CATEGORY => false,
        _ => return None,
    };
    let tags = tags(&v["tags"]);
    if tags.iter().any(|t| t == "ai-generated") {
        return None;
    }
    Some(Topic {
        id: v["id"].as_u64()?,
        title: v["title"].as_str()?.to_string(),
        free,
        multi_axis: tags.iter().any(|t| t == "multi-axis"),
    })
}

/// Every script post a search finds, page by page.
pub fn search(
    http: &dyn Http,
    pacer: &mut Pacer,
    cookie: &str,
    query: &str,
) -> Result<Vec<Topic>, NetError> {
    let mut out = Vec::new();
    for page in 1..=MAX_PAGES {
        pacer.wait();
        let req = Request::get(format!(
            "{FORUM}/search.json?q={}&page={page}",
            text::url_encode(query)
        ))
        .header("Cookie", format!("_t={cookie}"))
        .header("Accept", "application/json");
        let v = http::json(http, &req)?;
        let topics = v["topics"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        out.extend(topics.iter().filter_map(topic));
        if topics.is_empty() || v["grouped_search_result"]["more_full_page_results"] != true {
            break;
        }
    }
    Ok(out)
}

/// Every performer's script posts, each once.
pub fn search_all(
    http: &dyn Http,
    pacer: &mut Pacer,
    cookie: &str,
    actors: &[String],
    log: &mut dyn Write,
) -> Result<Vec<Topic>, NetError> {
    let mut all: Vec<Topic> = Vec::new();
    for actor in actors {
        for q in queries(actor) {
            let found = search(http, pacer, cookie, &q)?;
            let _ = writeln!(log, "EroScripts {q}: {} script post(s)", found.len());
            for t in found {
                if !all.iter().any(|a| a.id == t.id) {
                    all.push(t);
                }
            }
        }
    }
    Ok(all)
}

/// Whether a post is about a scene: the normalised post title contains the
/// normalised scene title as whole words. Titles of one word ("Creampie")
/// would match half the forum, so they never match.
pub fn matches(post_title: &str, scene_title: &str) -> bool {
    let scene = text::normalise(scene_title);
    text::words(&scene).count() >= 2 && text::contains_words(&text::normalise(post_title), &scene)
}

/// Add the matching posts to each scene's `handmade`.
pub fn attach(scenes: &mut [Scene], topics: &[Topic]) {
    for s in scenes {
        for t in topics.iter().filter(|t| matches(&t.title, &s.title)) {
            let h = Handmade::Eroscripts {
                topic_id: t.id,
                title: t.title.clone(),
                free: t.free,
                multi_axis: t.multi_axis,
            };
            if !s.handmade.contains(&h) {
                s.handmade.push(h);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::testing::Canned;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn reads_the_session_cookie() {
        let c = json!({"settings": {"eroscripts": {"session": {"cookie": "abc"}}}});
        assert_eq!(session_cookie(&c).as_deref(), Some("abc"));
        let c = json!({"eroscripts": {"session": {"cookie": "_t=abc"}}});
        assert_eq!(session_cookie(&c).as_deref(), Some("abc"));
        assert_eq!(session_cookie(&json!({"settings": {}})), None);
        let c = json!({"settings": {"eroscripts": {"session": {"cookie": " "}}}});
        assert_eq!(session_cookie(&c), None);
    }

    #[test]
    fn builds_queries() {
        assert_eq!(queries("Miss Lexa"), ["\"Miss Lexa\"", "misslexa"]);
        assert_eq!(queries("Lexa"), ["\"Lexa\""]);
    }

    #[test]
    fn keeps_script_posts_only() {
        let t = topic(&json!({"id": 1, "title": "A", "category_id": 15,
            "tags": [{"name": "vr"}, {"name": "multi-axis"}]}))
        .unwrap();
        assert!(!t.free && t.multi_axis);
        let t =
            topic(&json!({"id": 2, "title": "B", "category_id": 14, "tags": ["free"]})).unwrap();
        assert!(t.free && !t.multi_axis);
        assert!(topic(&json!({"id": 3, "title": "C", "category_id": 5})).is_none());
        assert!(
            topic(&json!({"id": 4, "title": "D", "category_id": 14, "tags": ["ai-generated"]}))
                .is_none()
        );
    }

    #[test]
    fn matches_titles() {
        // Verified pairs (SLR scene → EroScripts post).
        assert!(matches(
            "Miss Lexa - Latina Milf oiled up & creampied [SLR]",
            "Latina Milf oiled up & creampied"
        ));
        assert!(matches(
            "(SLR) Miss Lexa – He Woke Me Up To Creampie Me",
            "He Woke Me Up To Creampie Me"
        ));
        assert!(matches(
            "MissLexa - Please Creampie Me!",
            "Please Creampie Me"
        ));
        assert!(!matches("Please Creampie Melanie", "Please Creampie Me"));
        assert!(!matches("Miss Lexa creampie", "Creampie"));
        assert!(!matches("Anything", ""));
    }

    #[test]
    fn searches_and_attaches() {
        let mut http = Canned::default();
        let url = |q: &str, p: u32| format!("{FORUM}/search.json?q={q}&page={p}");
        http.on(
            "GET",
            &url("%22Miss%20Lexa%22", 1),
            200,
            &json!({"topics": [
                {"id": 317875, "title": "Miss Lexa - Latina Milf oiled up & creampied", "category_id": 15, "tags": []},
                {"id": 9, "title": "Miss Lexa request", "category_id": 2}],
              "grouped_search_result": {"more_full_page_results": true}})
            .to_string(),
        );
        http.on(
            "GET",
            &url("%22Miss%20Lexa%22", 2),
            200,
            &json!({"topics": [], "grouped_search_result": {"more_full_page_results": null}})
                .to_string(),
        );
        http.on(
            "GET",
            &url("misslexa", 1),
            200,
            &json!({"topics": [
                {"id": 317875, "title": "Miss Lexa - Latina Milf oiled up & creampied", "category_id": 15},
                {"id": 273166, "title": "MissLexa Please Creampie Me", "category_id": 14, "tags": ["multi-axis"]}],
              "grouped_search_result": {"more_full_page_results": false}})
            .to_string(),
        );
        let topics = search_all(
            &http,
            &mut Pacer::new(Duration::ZERO),
            "secret",
            &["Miss Lexa".into()],
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            topics.iter().map(|t| t.id).collect::<Vec<_>>(),
            [317875, 273166]
        );
        assert!(
            http.seen
                .borrow()
                .iter()
                .all(|r| r.headers.contains(&("Cookie".into(), "_t=secret".into())))
        );

        let mut scenes = vec![
            crate::slr::scene(
                &json!({"id": 85666, "label": "l", "title": "Latina Milf oiled up & creampied"}),
            )
            .unwrap(),
            crate::slr::scene(&json!({"id": 62617, "label": "p", "title": "Please Creampie Me"}))
                .unwrap(),
            crate::slr::scene(&json!({"id": 1, "label": "o", "title": "Other Scene"})).unwrap(),
        ];
        attach(&mut scenes, &topics);
        attach(&mut scenes, &topics);
        assert_eq!(
            scenes[0].handmade,
            [Handmade::Eroscripts {
                topic_id: 317875,
                title: "Miss Lexa - Latina Milf oiled up & creampied".into(),
                free: false,
                multi_axis: false
            }]
        );
        assert!(matches!(
            scenes[1].handmade[..],
            [Handmade::Eroscripts {
                topic_id: 273166,
                free: true,
                multi_axis: true,
                ..
            }]
        ));
        assert!(scenes[2].handmade.is_empty());

        http.on("GET", &url("%22Rate%22", 1), 429, "");
        assert!(matches!(
            search(&http, &mut Pacer::new(Duration::ZERO), "s", "\"Rate\""),
            Err(NetError::RateLimited(_))
        ));
    }
}
