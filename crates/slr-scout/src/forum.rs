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

/// The searches for a performer, in post titles only (a scene matches a
/// post by its title, and title searches stay a page or two even for the
/// busiest performers): her name as a phrase, and the name run together
/// (`"Miss Lexa" in:title`, `misslexa in:title`, which also finds
/// `[MissLexaVR]`).
pub fn queries(actor: &str) -> Vec<String> {
    let mut out = vec![format!("\"{}\" in:title", actor.trim())];
    let joined: String = text::normalise(actor).replace(' ', "");
    if !joined.is_empty() && text::normalise(actor).contains(' ') {
        out.push(format!("{joined} in:title"));
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

/// A script file attached to a post.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub name: String,
    pub url: String,
}

/// Axis suffixes of companion scripts (`<stem>.<axis>.funscript`;
/// AXIS_SUFFIXES in electron/library-scan.js), with the TCode channel names
/// some posts use. `L0` is the stroke itself.
const AXES: &[(&str, &str)] = &[
    ("surge", "surge"),
    ("sway", "sway"),
    ("twist", "twist"),
    ("roll", "roll"),
    ("pitch", "pitch"),
    ("vib", "vib"),
    ("lube", "lube"),
    ("pump", "pump"),
    ("suction", "suction"),
    ("valve", "valve"),
    ("l1", "surge"),
    ("l2", "sway"),
    ("r0", "twist"),
    ("r1", "roll"),
    ("r2", "pitch"),
];

/// A script name's stem and axis (`None` for the stroke script); `None`
/// for a name that isn't a `.funscript`.
pub fn script_axis(name: &str) -> Option<(String, Option<&'static str>)> {
    let stem = name.len().checked_sub(".funscript".len()).and_then(|cut| {
        name.get(cut..)?
            .eq_ignore_ascii_case(".funscript")
            .then(|| &name[..cut])
    })?;
    if let Some((base, suffix)) = stem.rsplit_once('.') {
        let suffix = suffix.to_ascii_lowercase();
        if suffix == "l0" {
            return Some((base.to_string(), None));
        }
        if let Some((_, axis)) = AXES.iter().find(|(s, _)| *s == suffix) {
            return Some((base.to_string(), Some(axis)));
        }
    }
    Some((stem.to_string(), None))
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let body = &rest[1..end];
        let ch = match body {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => body
                .strip_prefix("#x")
                .or_else(|| body.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| body.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut inside = false;
    for c in s.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            c if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

/// A forum upload link as an absolute forum URL (`upload://x`, `/uploads/…`
/// or the forum's own host); `None` for other hosts.
fn upload_url(href: &str) -> Option<String> {
    if let Some(rest) = href.strip_prefix("upload://") {
        return Some(format!("{FORUM}/uploads/short-url/{rest}"));
    }
    if href.starts_with("/uploads/") {
        return Some(format!("{FORUM}{href}"));
    }
    href.starts_with(&format!("{FORUM}/uploads/"))
        .then(|| href.to_string())
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The `.funscript` uploads linked in a post's `cooked` HTML: each `<a>`
/// pointing at a forum upload, named by its link text when that names a
/// script, else by the URL's file name.
pub fn post_scripts(cooked: &str) -> Vec<Attachment> {
    let mut out: Vec<Attachment> = Vec::new();
    let mut rest = cooked;
    while let Some(at) = rest.find("<a ") {
        rest = &rest[at + 3..];
        let Some(open_end) = rest.find('>') else {
            break;
        };
        let attrs = &rest[..open_end];
        let close = rest.find("</a>").unwrap_or(rest.len());
        let text = decode_entities(strip_tags(rest.get(open_end + 1..close).unwrap_or("")).trim());
        let Some(href) = attrs
            .split("href=\"")
            .nth(1)
            .and_then(|h| h.split('"').next())
        else {
            continue;
        };
        let Some(url) = upload_url(&decode_entities(href)) else {
            continue;
        };
        let file = percent_decode(url.rsplit('/').next().unwrap_or(""));
        let name = if script_axis(&text).is_some() {
            text
        } else {
            file
        };
        if script_axis(&name).is_some() && !out.iter().any(|a| a.url == url) {
            out.push(Attachment { name, url });
        }
    }
    out
}

/// The stroke script for a scene among a post's scripts, and its axis
/// companions (same stem): the only stroke script, else the one whose name
/// shares the most words with the scene title (none shared: no pick).
pub fn pick_scripts<'a>(
    scripts: &'a [Attachment],
    scene_title: &str,
) -> Option<(&'a Attachment, Vec<(&'static str, &'a Attachment)>)> {
    let mains: Vec<(&Attachment, String)> = scripts
        .iter()
        .filter_map(|a| match script_axis(&a.name)? {
            (stem, None) => Some((a, stem)),
            _ => None,
        })
        .collect();
    let title = text::normalise(scene_title);
    let title_words: Vec<&str> = text::words(&title).collect();
    let (main, stem) = if mains.len() == 1 {
        mains.into_iter().next()?
    } else {
        let shared = |stem: &str| {
            let n = text::normalise(stem);
            text::words(&n).filter(|w| title_words.contains(w)).count()
        };
        let best = mains.iter().map(|(_, s)| shared(s)).max().unwrap_or(0);
        if best == 0 {
            return None;
        }
        mains.into_iter().find(|(_, s)| shared(s) == best)?
    };
    let companions = scripts
        .iter()
        .filter_map(|a| match script_axis(&a.name)? {
            (s, Some(axis)) if s.eq_ignore_ascii_case(&stem) => Some((axis, a)),
            _ => None,
        })
        .collect();
    Some((main, companions))
}

/// A topic's first post's scripts (`/t/<id>.json`, read only).
pub fn topic_scripts(
    http: &dyn Http,
    pacer: &mut Pacer,
    cookie: &str,
    topic_id: u64,
) -> Result<Vec<Attachment>, NetError> {
    pacer.wait();
    let req = Request::get(format!("{FORUM}/t/{topic_id}.json"))
        .header("Cookie", format!("_t={cookie}"))
        .header("Accept", "application/json");
    let v = http::json(http, &req)?;
    Ok(post_scripts(
        v["post_stream"]["posts"][0]["cooked"]
            .as_str()
            .unwrap_or_default(),
    ))
}

/// Download a forum upload into `dest`: the forum answers with a redirect
/// to its CDN, followed by hand so the session cookie only goes to the forum.
pub fn download_upload(
    http: &dyn Http,
    pacer: &mut Pacer,
    cookie: &str,
    url: &str,
    dest: &std::path::Path,
) -> Result<(), NetError> {
    pacer.wait();
    let req = Request::get(url).header("Cookie", format!("_t={cookie}"));
    let res = http.send(&req).map_err(NetError::Failed)?;
    match res.status {
        200..=299 => std::fs::write(dest, res.body.as_bytes())
            .map_err(|e| NetError::Failed(format!("{}: {e}", dest.display()))),
        300..=399 => {
            let to = res
                .header_values("location")
                .next()
                .ok_or_else(|| NetError::Failed(format!("{url}: redirect without Location")))?;
            let to = if to.starts_with('/') {
                format!("{FORUM}{to}")
            } else {
                to.to_string()
            };
            http.download(&Request::get(to), dest)
                .map_err(|e| NetError::Failed(format!("{url}: {e}")))
        }
        429 => Err(NetError::RateLimited(format!("{url}: HTTP 429"))),
        s => Err(NetError::Failed(format!("{url}: HTTP {s}"))),
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
        assert_eq!(
            queries("Miss Lexa"),
            ["\"Miss Lexa\" in:title", "misslexa in:title"]
        );
        assert_eq!(queries("Lexa"), ["\"Lexa\" in:title"]);
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
            &url("%22Miss%20Lexa%22%20in%3Atitle", 1),
            200,
            &json!({"topics": [
                {"id": 317875, "title": "Miss Lexa - Latina Milf oiled up & creampied", "category_id": 15, "tags": []},
                {"id": 9, "title": "Miss Lexa request", "category_id": 2}],
              "grouped_search_result": {"more_full_page_results": true}})
            .to_string(),
        );
        http.on(
            "GET",
            &url("%22Miss%20Lexa%22%20in%3Atitle", 2),
            200,
            &json!({"topics": [], "grouped_search_result": {"more_full_page_results": null}})
                .to_string(),
        );
        http.on(
            "GET",
            &url("misslexa%20in%3Atitle", 1),
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

    #[test]
    fn reads_script_names_and_axes() {
        assert_eq!(script_axis("A.funscript"), Some(("A".into(), None)));
        assert_eq!(
            script_axis("A b.Twist.FUNSCRIPT"),
            Some(("A b".into(), Some("twist")))
        );
        assert_eq!(
            script_axis("A.R1.funscript"),
            Some(("A".into(), Some("roll")))
        );
        assert_eq!(script_axis("A.L0.funscript"), Some(("A".into(), None)));
        assert_eq!(script_axis("A.v2.funscript"), Some(("A.v2".into(), None)));
        assert_eq!(script_axis("A.zip"), None);
        assert_eq!(script_axis("funscript"), None);
    }

    #[test]
    fn finds_post_scripts_and_picks_the_scene_s() {
        let cooked = r#"<p><a class="attachment" href="/uploads/short-url/a1.funscript">Luna &amp; Sue - Part One.funscript</a> (9 KB)
            <a href="upload://b2.funscript">x</a>
            <a class="attachment" href="/uploads/short-url/c3.funscript"><span>Luna &amp; Sue - Part One.pitch.funscript</span></a>
            <a href="/uploads/short-url/z.zip">pack.zip</a>
            <a href="https://mega.nz/file/x">Part One.funscript</a>
            <a href="https://discuss.eroscripts.com/uploads/default/original/1X/Part%20Two.funscript">Part Two</a></p>"#;
        let scripts = post_scripts(cooked);
        let names: Vec<&str> = scripts.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Luna & Sue - Part One.funscript",
                "b2.funscript",
                "Luna & Sue - Part One.pitch.funscript",
                "Part Two.funscript"
            ]
        );
        assert_eq!(
            scripts[1].url,
            format!("{FORUM}/uploads/short-url/b2.funscript")
        );
        let (main, companions) = pick_scripts(&scripts, "Part One").unwrap();
        assert_eq!(main.name, "Luna & Sue - Part One.funscript");
        assert_eq!(companions.len(), 1);
        assert_eq!(companions[0].0, "pitch");
        let (main, companions) = pick_scripts(&scripts, "Part Two").unwrap();
        assert_eq!(main.name, "Part Two.funscript");
        assert!(companions.is_empty());
        assert!(pick_scripts(&scripts, "Something Else").is_none());
        // One stroke script: it's the scene's whatever its name.
        assert_eq!(
            pick_scripts(&scripts[..1], "Other").unwrap().0.name,
            "Luna & Sue - Part One.funscript"
        );
        assert!(pick_scripts(&[], "x").is_none());
    }

    #[test]
    fn downloads_uploads_through_the_redirect() {
        let dir = std::env::temp_dir().join(format!("slr-scout-upload-{}", std::process::id()));
        let _ = fs_reset(&dir);
        let mut http = Canned::default();
        let url = format!("{FORUM}/uploads/short-url/a.funscript");
        http.with_headers(
            "GET",
            &url,
            302,
            &[("location", "https://cdn.test/a.funscript")],
            "",
        );
        http.on(
            "GET",
            "https://cdn.test/a.funscript",
            200,
            "{\"actions\":[]}",
        );
        let dest = dir.join("a.funscript");
        download_upload(
            &http,
            &mut Pacer::new(Duration::ZERO),
            "secret",
            &url,
            &dest,
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "{\"actions\":[]}");
        let seen = http.seen.borrow();
        assert!(
            seen[0]
                .headers
                .contains(&("Cookie".into(), "_t=secret".into()))
        );
        // The CDN never sees the session cookie.
        assert!(seen[1].headers.is_empty());
        drop(seen);
        http.on(
            "GET",
            &format!("{FORUM}/uploads/short-url/gone.funscript"),
            404,
            "",
        );
        assert!(
            download_upload(
                &http,
                &mut Pacer::new(Duration::ZERO),
                "s",
                &format!("{FORUM}/uploads/short-url/gone.funscript"),
                &dest
            )
            .is_err()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn fs_reset(dir: &std::path::Path) -> std::io::Result<()> {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir)
    }
}
