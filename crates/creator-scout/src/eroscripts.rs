//! EroScripts (read only, with the app's session cookie, paced ≥ 700 ms):
//! the `real` + `non-vr` tag feed since the cursor, and a topic's first
//! post: its video hosts, tags, likes and first image.

use crate::html;
use crate::store::{self, Post};
use serde_json::{Map, Value};
use slr_scout::http::{self, Http, NetError, Pacer, Request};
use std::time::Duration;

pub const FORUM: &str = "https://discuss.eroscripts.com";
pub const GAP: Duration = Duration::from_millis(700);
/// Free Scripts / Paid Scripts.
pub const FREE_CATEGORY: u64 = 14;
pub const PAID_CATEGORY: u64 = 15;
/// Feed pages read on the first run (no cursor yet).
pub const FIRST_RUN_PAGES: u32 = 5;
/// Feed pages read at most on later runs.
pub const MAX_PAGES: u32 = 20;

/// Hosts the EroScripts scout downloads from (forum attachments,
/// pixeldrain, mega, rule34video, SexLikeReal, pornhub via yt-dlp).
pub const DOWNLOADABLE: &[&str] = &[
    "forum",
    "pixeldrain",
    "mega",
    "rule34video",
    "slr",
    "pornhub",
];

/// Link domains → host names. The downloadable ones first; the others are
/// recorded (shown on the card) but never fetched.
const HOSTS: &[(&str, &str)] = &[
    ("pixeldrain.com", "pixeldrain"),
    ("pixeldra.in", "pixeldrain"),
    ("mega.nz", "mega"),
    ("mega.co.nz", "mega"),
    ("mega.io", "mega"),
    ("rule34video.com", "rule34video"),
    ("rule34video.party", "rule34video"),
    ("sexlikereal.com", "slr"),
    ("pornhub.com", "pornhub"),
    ("pornhub.org", "pornhub"),
    ("gofile.io", "gofile"),
    ("onlyfans.com", "onlyfans"),
    ("fansly.com", "fansly"),
    ("manyvids.com", "manyvids"),
    ("spankbang.com", "spankbang"),
    ("xvideos.com", "xvideos"),
    ("xhamster.com", "xhamster"),
    ("iwara.tv", "iwara"),
    ("mediafire.com", "mediafire"),
    ("drive.google.com", "gdrive"),
    ("dropbox.com", "dropbox"),
    ("youtube.com", "youtube"),
    ("youtu.be", "youtube"),
    ("redgifs.com", "redgifs"),
    ("eporner.com", "eporner"),
    ("clips4sale.com", "clips4sale"),
    ("faptap.net", "faptap"),
];

const VIDEO_EXTS: &[&str] = &["mp4", "m4v", "mkv", "webm", "mov"];

/// A topic as the feed lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Topic {
    pub id: u64,
    pub title: String,
    pub free: bool,
    pub tags: Vec<String>,
    pub likes: u64,
    pub created_at: Option<String>,
}

/// Tag names; Discourse gives strings or `{name}` objects.
pub fn tag_names(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_str().or_else(|| t["name"].as_str()))
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

/// A feed or topic JSON as a script post (Free or Paid Scripts, not
/// AI-generated).
pub fn topic(v: &Value) -> Option<Topic> {
    let free = match v["category_id"].as_u64()? {
        FREE_CATEGORY => true,
        PAID_CATEGORY => false,
        _ => return None,
    };
    let tags = tag_names(&v["tags"]);
    if tags.iter().any(|t| t == "ai-generated") {
        return None;
    }
    Some(Topic {
        id: v["id"].as_u64()?,
        title: v["title"].as_str()?.trim().to_string(),
        free,
        tags,
        likes: v["like_count"].as_u64().unwrap_or(0),
        created_at: v["created_at"].as_str().map(str::to_string),
    })
}

/// The new topics of a feed walk, newest id, and whether pages were left.
#[derive(Debug, Default, PartialEq)]
pub struct Listing {
    pub topics: Vec<Topic>,
    pub newest: Option<u64>,
    pub more_left: bool,
}

/// Read the feed (bumped order) from page 0 until a page holds no topic
/// newer than `cursor`, the feed ends, or `max_pages` were read.
pub fn list_new(
    http: &dyn Http,
    pacer: &mut Pacer,
    base: &str,
    cookie: &str,
    cursor: Option<u64>,
    max_pages: u32,
) -> Result<Listing, NetError> {
    let mut out = Listing::default();
    for page in 0..max_pages {
        pacer.wait();
        let req = Request::get(format!(
            "{base}/tags/intersection/real/non-vr.json?page={page}"
        ))
        .header("Cookie", format!("_t={cookie}"))
        .header("Accept", "application/json");
        let v = http::json(http, &req)?;
        let list = &v["topic_list"];
        let raw = list["topics"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut fresh = 0;
        for t in raw {
            let Some(id) = t["id"].as_u64() else { continue };
            out.newest = Some(out.newest.map_or(id, |n| n.max(id)));
            if cursor.is_some_and(|c| id <= c) {
                continue;
            }
            fresh += 1;
            if let Some(topic) = topic(t)
                && !out.topics.iter().any(|x| x.id == topic.id)
            {
                out.topics.push(topic);
            }
        }
        let more = !raw.is_empty() && list["more_topics_url"].as_str().is_some();
        if !more || (cursor.is_some() && fresh == 0) {
            return Ok(out);
        }
        if page + 1 == max_pages {
            out.more_left = true;
        }
    }
    Ok(out)
}

/// A topic's JSON (`/t/<id>.json`).
pub fn topic_json(
    http: &dyn Http,
    pacer: &mut Pacer,
    base: &str,
    cookie: &str,
    id: u64,
) -> Result<Value, NetError> {
    pacer.wait();
    let req = Request::get(format!("{base}/t/{id}.json"))
        .header("Cookie", format!("_t={cookie}"))
        .header("Accept", "application/json");
    http::json(http, &req)
}

/// The script posts (Free / Paid Scripts, not AI-generated) among the
/// topics a forum search (`/search.json?q=`) finds.
pub fn search(
    http: &dyn Http,
    pacer: &mut Pacer,
    base: &str,
    cookie: &str,
    query: &str,
) -> Result<Vec<Topic>, NetError> {
    pacer.wait();
    let req = Request::get(format!(
        "{base}/search.json?q={}",
        slr_scout::text::url_encode(query)
    ))
    .header("Cookie", format!("_t={cookie}"))
    .header("Accept", "application/json");
    let v = http::json(http, &req)?;
    Ok(v["topics"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(topic)
        .collect())
}

/// The domain of an absolute URL, lower case, without `www.`.
fn domain(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("//"))?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?.split(':').next()?.to_lowercase();
    Some(host.trim_start_matches("www.").to_string())
}

fn is_video_path(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| VIDEO_EXTS.iter().any(|v| ext.eq_ignore_ascii_case(v)))
}

/// A link's host name; `None` for links that aren't videos or video pages.
pub fn host_of(url: &str, forum_base: &str) -> Option<&'static str> {
    let forum_domain = domain(forum_base);
    let upload = url.starts_with("upload://")
        || url.starts_with("/uploads/")
        || (domain(url).is_some() && domain(url) == forum_domain && url.contains("/uploads/"));
    if upload {
        return is_video_path(url).then_some("forum");
    }
    let d = domain(url)?;
    HOSTS
        .iter()
        .find(|(h, _)| d == *h || d.ends_with(&format!(".{h}")))
        .map(|(_, name)| *name)
}

/// The video hosts a post's `cooked` HTML links (`href`, `src`,
/// `data-video-src`, `data-orig-src`), each once, in order.
pub fn hosts(cooked: &str, forum_base: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in ["a", "video", "source", "div", "iframe"] {
        for t in html::open_tags(cooked, tag) {
            for (name, value) in html::attrs(t) {
                if !matches!(
                    name.as_str(),
                    "href" | "src" | "data-video-src" | "data-orig-src"
                ) {
                    continue;
                }
                if let Some(h) = host_of(&value, forum_base)
                    && !out.iter().any(|x| x == h)
                {
                    out.push(h.to_string());
                }
            }
        }
    }
    // Keep the downloadable hosts first, then the rest, each in link order.
    out.sort_by_key(|h| !DOWNLOADABLE.contains(&h.as_str()));
    out
}

/// Public video pages a post links (Pornhub videos, rule34video videos,
/// SexLikeReal scenes), each once, at most 5: their poster frames show her.
pub fn video_pages(cooked: &str, forum_base: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in ["a", "iframe"] {
        for t in html::open_tags(cooked, tag) {
            for (name, value) in html::attrs(t) {
                if name != "href" && name != "src" {
                    continue;
                }
                let page = match host_of(&value, forum_base) {
                    Some("pornhub") => value.contains("view_video.php?viewkey="),
                    Some("rule34video") => value.contains("/video/") || value.contains("/videos/"),
                    Some("slr") => value.contains("/scenes/"),
                    _ => false,
                };
                if page && value.starts_with("https://") && !out.contains(&value) && out.len() < 5 {
                    out.push(value);
                }
            }
        }
    }
    out
}

/// The first picture in a post (emoji, avatars and site icons skipped), as
/// an absolute URL.
pub fn first_image(cooked: &str, forum_base: &str) -> Option<String> {
    html::open_tags(cooked, "img").into_iter().find_map(|t| {
        let class = html::attr(t, "class").unwrap_or_default();
        if ["emoji", "avatar", "site-icon"]
            .iter()
            .any(|c| class.contains(c))
        {
            return None;
        }
        let src = html::attr(t, "src")?;
        if src.contains("/images/emoji/") || src.contains("avatar") || src.starts_with("data:") {
            return None;
        }
        absolute(&src, forum_base)
    })
}

fn absolute(src: &str, base: &str) -> Option<String> {
    if src.starts_with("https://") || src.starts_with("http://") {
        Some(src.to_string())
    } else if let Some(rest) = src.strip_prefix("//") {
        Some(format!("https://{rest}"))
    } else if src.starts_with('/') {
        Some(format!("{}{src}", base.trim_end_matches('/')))
    } else {
        None
    }
}

/// A post from a topic as the feed listed it, before its details are read.
pub fn listed_post(t: &Topic) -> Post {
    Post {
        kind: "eroscripts".into(),
        id: t.id.to_string(),
        title: t.title.clone(),
        url: format!("{}/t/{}", store::FORUM_SITE, t.id),
        free: t.free,
        hosts: Vec::new(),
        downloadable: false,
        creampie: is_creampie(&t.tags, &t.title),
        multi_axis: t.tags.iter().any(|x| x == "multi-axis"),
        score: None,
        likes: t.likes,
        tags: t.tags.clone(),
        image: None,
        duration_s: None,
        in_library: false,
        created_at: t.created_at.clone(),
        video_pages: Vec::new(),
        extra: Map::new(),
    }
}

fn is_creampie(tags: &[String], title: &str) -> bool {
    tags.iter().any(|t| t == "creampie") || title.to_lowercase().contains("creampie")
}

/// A post from its topic JSON: the feed fields refreshed, the hosts and
/// whether it can be fetched; the first image's URL beside it.
pub fn detailed_post(v: &Value, forum_base: &str) -> Option<(Post, Option<String>)> {
    let t = topic(v)?;
    let cooked = v["post_stream"]["posts"][0]["cooked"]
        .as_str()
        .unwrap_or_default();
    let mut post = listed_post(&t);
    post.hosts = hosts(cooked, forum_base);
    post.video_pages = video_pages(cooked, forum_base);
    post.downloadable = post.free
        && post
            .hosts
            .iter()
            .any(|h| DOWNLOADABLE.contains(&h.as_str()));
    Some((post, first_image(cooked, forum_base)))
}

/// Fetch a post image: forum-hosted ones with the cookie and a redirect
/// followed by hand (the cookie only goes to the forum), others plainly.
pub fn download_image(
    http: &dyn Http,
    pacer: &mut Pacer,
    base: &str,
    cookie: &str,
    url: &str,
    dest: &std::path::Path,
) -> Result<(), String> {
    if domain(url) != domain(base) {
        return http.download(&Request::get(url), dest);
    }
    pacer.wait();
    let res = http.send(&Request::get(url).header("Cookie", format!("_t={cookie}")))?;
    match res.status {
        200..=299 => std::fs::write(dest, res.body.as_bytes())
            .map_err(|e| format!("{}: {e}", dest.display())),
        300..=399 => {
            let to = res
                .header_values("location")
                .next()
                .ok_or("redirect without Location")?;
            let to = absolute(to, base).ok_or("bad redirect")?;
            http.download(&Request::get(to), dest)
        }
        s => Err(format!("HTTP {s}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Canned;
    use serde_json::json;
    use std::time::Duration;

    const COOKED: &str = r#"<p>Video: <a href="https://pixeldrain.com/u/abc">pd</a>
        <a href="https://mega.nz/file/x#y">mega</a> and <a href="https://www.onlyfans.com/sweetie">OF</a>
        <a href="https://discuss.eroscripts.com/t/other/1">other topic</a></p>
        <div class="video-placeholder-container" data-video-src="/uploads/short-url/abc.mp4"></div>
        <a class="attachment" href="/uploads/short-url/script.funscript">script</a>
        <img src="/images/emoji/twitter/smile.png" class="emoji">
        <img src="https://cdn.x/user_avatar/a.png" class="avatar">
        <a class="lightbox" href="https://cdn.x/original/1.jpeg"><img src="//cdn.x/optimized/1_690x388.jpeg" alt="Cover"></a>
        <a href="https://de.pornhub.com/view_video.php?viewkey=1">ph</a>"#;

    #[test]
    fn finds_hosts_downloadable_first() {
        assert_eq!(
            hosts(COOKED, FORUM),
            ["pixeldrain", "mega", "pornhub", "forum", "onlyfans"]
        );
        assert_eq!(
            hosts("<a href=\"https://fansly.com/x\">f</a>", FORUM),
            ["fansly"]
        );
        assert_eq!(host_of("upload://abc.MP4", FORUM), Some("forum"));
        assert_eq!(
            host_of("https://discuss.eroscripts.com/uploads/a.zip", FORUM),
            None
        );
        assert_eq!(
            host_of("https://www.sexlikereal.com/scenes/x", FORUM),
            Some("slr")
        );
        assert_eq!(host_of("https://imgur.com/x", FORUM), None);
    }

    #[test]
    fn first_image_skips_emoji_and_avatars() {
        assert_eq!(
            first_image(COOKED, FORUM).as_deref(),
            Some("https://cdn.x/optimized/1_690x388.jpeg")
        );
        assert_eq!(
            first_image(r#"<img src="/uploads/default/1.png">"#, FORUM).as_deref(),
            Some("https://discuss.eroscripts.com/uploads/default/1.png")
        );
        assert_eq!(first_image("<p>none</p>", FORUM), None);
    }

    #[test]
    fn detailed_posts() {
        let v = json!({"id": 41415, "title": " Sweetie Fox - Creampie ", "category_id": 14,
            "tags": [{"name": "Multi-Axis"}, "real"], "like_count": 108, "created_at": "2026-11-01T00:00:00Z",
            "post_stream": {"posts": [{"cooked": COOKED}]}});
        let (p, image) = detailed_post(&v, FORUM).unwrap();
        assert_eq!(p.url, "https://discuss.eroscripts.com/t/41415");
        assert_eq!(p.title, "Sweetie Fox - Creampie");
        assert!(p.free && p.downloadable && p.creampie && p.multi_axis);
        assert_eq!(p.likes, 108);
        assert_eq!(p.tags, ["multi-axis", "real"]);
        assert_eq!(
            p.video_pages,
            ["https://de.pornhub.com/view_video.php?viewkey=1"]
        );
        assert!(image.is_some());
        let mut paid = v.clone();
        paid["category_id"] = json!(15);
        let (p, _) = detailed_post(&paid, FORUM).unwrap();
        assert!(!p.free && !p.downloadable);
        let mut other = v.clone();
        other["category_id"] = json!(2);
        assert!(detailed_post(&other, FORUM).is_none());
        let mut ai = v;
        ai["tags"] = json!(["ai-generated"]);
        assert!(detailed_post(&ai, FORUM).is_none());
    }

    fn page(topics: Value, more: bool) -> String {
        let mut list = json!({"topics": topics});
        if more {
            list["more_topics_url"] = json!("/tags/intersection/real/non-vr?page=1");
        }
        json!({"topic_list": list}).to_string()
    }

    fn t(id: u64, cat: u64) -> Value {
        json!({"id": id, "title": format!("T{id}"), "category_id": cat, "tags": ["real", "non-vr"]})
    }

    #[test]
    fn feed_stops_at_the_cursor_and_the_page_cap() {
        let url = |p: u32| format!("{FORUM}/tags/intersection/real/non-vr.json?page={p}");
        let mut net = Canned::default();
        net.on(
            "GET",
            &url(0),
            200,
            &page(json!([t(30, 14), t(29, 5), t(10, 14)]), true),
        );
        net.on(
            "GET",
            &url(1),
            200,
            &page(json!([t(28, 15), t(9, 14)]), true),
        );
        net.on(
            "GET",
            &url(2),
            200,
            &page(json!([t(8, 14), t(7, 14)]), true),
        );
        let mut pacer = Pacer::new(Duration::ZERO);
        let l = list_new(&net, &mut pacer, FORUM, "c", Some(20), 20).unwrap();
        let ids: Vec<u64> = l.topics.iter().map(|t| t.id).collect();
        assert_eq!(ids, [30, 28]);
        assert_eq!(l.newest, Some(30));
        assert!(!l.more_left);
        assert!(
            net.seen.borrow()[0]
                .headers
                .contains(&("Cookie".into(), "_t=c".into()))
        );
        // No cursor: everything up to the page cap, and more is left.
        let l = list_new(&net, &mut pacer, FORUM, "c", None, 2).unwrap();
        assert_eq!(l.topics.len(), 4);
        assert!(l.more_left);
    }
}
