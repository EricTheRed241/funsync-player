//! FapTap's open JSON API (no login): new videos since the cursor
//! (`/api/videos`, newest first), and a creator's videos by name (`?q=`).

use crate::store::{self, Post};
use serde_json::{Map, Value};
use slr_scout::http::{self, Http, NetError, Pacer, Request};
use slr_scout::text;
use std::cmp::Ordering;
use std::time::Duration;

pub const SITE: &str = "https://faptap.net";
pub const GAP: Duration = Duration::from_millis(1000);
/// Videos per page: the API serves at most 42.
pub const PAGE: u32 = 42;
pub const FIRST_RUN_PAGES: u32 = 5;
pub const MAX_PAGES: u32 = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub tags: Vec<String>,
    pub performers: Vec<String>,
    pub uploader: Option<String>,
    pub likes: u64,
    pub duration_s: Option<u64>,
    /// Absolute URL.
    pub thumbnail: Option<String>,
    pub created_at: Option<String>,
}

/// Compare snowflake ids (decimal strings): longer is newer.
pub fn cmp_ids(a: &str, b: &str) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

/// FapTap's thumbnail (`thumbnail_url`, else the uploader's
/// `user_thumbnail_url`) as an absolute URL: as given when https, a
/// relative asset path under `<base>/api/assets/`.
pub fn thumbnail_url(v: &Value, base: &str) -> Option<String> {
    ["thumbnail_url", "user_thumbnail_url"]
        .iter()
        .find_map(|k| {
            let t = v[*k].as_str()?.trim();
            if t.is_empty() {
                None
            } else if t.starts_with("https://") {
                Some(t.to_string())
            } else if !t.contains("..")
                && t.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b))
            {
                Some(format!("{base}/api/assets/{}", t.trim_start_matches('/')))
            } else {
                None
            }
        })
}

pub fn video(v: &Value, base: &str) -> Option<Video> {
    let id = match &v["id"] {
        Value::String(s) if !s.is_empty() => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let names = |key: &str| -> Vec<String> {
        v[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_str().or_else(|| t["name"].as_str()))
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    };
    Some(Video {
        title: v["name"].as_str().unwrap_or_default().trim().to_string(),
        tags: names("tags")
            .into_iter()
            .map(|t| t.to_lowercase())
            .collect(),
        performers: names("performers"),
        uploader: v["user"]["username"].as_str().map(str::to_string),
        likes: v["rating"]["likes"].as_u64().unwrap_or(0),
        duration_s: v["duration"].as_f64().map(|d| d.round() as u64),
        thumbnail: thumbnail_url(v, base),
        created_at: v["created_at"].as_str().map(str::to_string),
        id,
    })
}

fn get(
    http: &dyn Http,
    pacer: &mut Pacer,
    base: &str,
    query: &str,
) -> Result<(Vec<Video>, u64), NetError> {
    pacer.wait();
    let req =
        Request::get(format!("{base}/api/videos?{query}")).header("Accept", "application/json");
    let v = http::json(http, &req)?;
    let data = &v["data"];
    let videos = data["videos"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| video(x, base))
        .collect();
    Ok((videos, data["total"].as_u64().unwrap_or(0)))
}

#[derive(Debug, Default, PartialEq)]
pub struct Listing {
    pub videos: Vec<Video>,
    pub newest: Option<String>,
    pub more_left: bool,
}

/// New videos (newest first) until one at or below `cursor`, the end, or
/// `max_pages`.
pub fn list_new(
    http: &dyn Http,
    pacer: &mut Pacer,
    base: &str,
    cursor: Option<&str>,
    max_pages: u32,
) -> Result<Listing, NetError> {
    let mut out = Listing::default();
    for page in 1..=max_pages {
        let (videos, total) = get(
            http,
            pacer,
            base,
            &format!("limit={PAGE}&page={page}&order_by=created_at.desc"),
        )?;
        let mut reached = false;
        for v in videos.iter() {
            if out
                .newest
                .as_deref()
                .is_none_or(|n| cmp_ids(&v.id, n) == Ordering::Greater)
            {
                out.newest = Some(v.id.clone());
            }
            if cursor.is_some_and(|c| cmp_ids(&v.id, c) != Ordering::Greater) {
                reached = true;
                continue;
            }
            if !out.videos.iter().any(|x| x.id == v.id) {
                out.videos.push(v.clone());
            }
        }
        let more = !videos.is_empty() && u64::from(page * PAGE) < total;
        if reached || !more {
            return Ok(out);
        }
        if page == max_pages {
            out.more_left = true;
        }
    }
    Ok(out)
}

/// The first page of a name search.
pub fn search(
    http: &dyn Http,
    pacer: &mut Pacer,
    base: &str,
    name: &str,
) -> Result<Vec<Video>, NetError> {
    get(
        http,
        pacer,
        base,
        &format!("limit={PAGE}&page=1&q={}", text::url_encode(name)),
    )
    .map(|(v, _)| v)
}

/// Whether a name (its words run together) is a run of whole words of
/// `haystack`: "HannahOwO" occurs in "Hannah Owo riding", "Lucy Cat" in
/// "lucycat solo", but "Cat" not in "Catwalk". Names of under 3 letters
/// never match.
pub fn occurs(name: &str, haystack: &str) -> bool {
    let want = text::normalise(name).replace(' ', "");
    if want.len() < 3 {
        return false;
    }
    let hay = text::normalise(haystack);
    let words: Vec<&str> = text::words(&hay).collect();
    for i in 0..words.len() {
        let mut run = String::new();
        for w in &words[i..] {
            run.push_str(w);
            if run == want {
                return true;
            }
            if run.len() >= want.len() {
                break;
            }
        }
    }
    false
}

/// Whether a video is by one of `names`: in its title or performers.
pub fn by_creator(v: &Video, names: &[String]) -> bool {
    names
        .iter()
        .any(|n| occurs(n, &v.title) || v.performers.iter().any(|p| occurs(n, p)))
}

pub fn post(v: &Video) -> Post {
    Post {
        kind: "faptap".into(),
        id: v.id.clone(),
        title: v.title.clone(),
        url: format!("{}/v/{}", store::FAPTAP_SITE, v.id),
        free: true,
        hosts: vec!["faptap".into()],
        downloadable: true,
        creampie: v.tags.iter().any(|t| t == "creampie")
            || v.title.to_lowercase().contains("creampie"),
        multi_axis: false,
        score: None,
        likes: v.likes,
        tags: v.tags.clone(),
        image: None,
        duration_s: v.duration_s,
        in_library: false,
        created_at: v.created_at.clone(),
        video_pages: Vec::new(),
        extra: Map::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Canned;
    use serde_json::json;

    fn v(id: &str, title: &str) -> Value {
        json!({"id": id, "name": title, "duration": 300.4, "rating": {"likes": 3},
               "tags": [{"name": "Creampie"}], "user": {"username": "up"},
               "user_thumbnail_url": "user_thumbnails/1.webp", "created_at": "2026-11-17T10:00:00Z"})
    }

    #[test]
    fn parses_videos_and_thumbnails() {
        let x = video(&v("21", "Hannah Owo Riding"), SITE).unwrap();
        assert_eq!(x.duration_s, Some(300));
        assert_eq!(x.likes, 3);
        assert_eq!(x.tags, ["creampie"]);
        assert_eq!(
            x.thumbnail.as_deref(),
            Some("https://faptap.net/api/assets/user_thumbnails/1.webp")
        );
        let p = post(&x);
        assert!(p.creampie && p.downloadable && p.free);
        assert_eq!(p.url, "https://faptap.net/v/21");
        let bunny = json!({"id": 1, "thumbnail_url": "https://vz.b-cdn.net/x/thumbnail.jpg"});
        assert_eq!(
            thumbnail_url(&bunny, SITE).as_deref(),
            Some("https://vz.b-cdn.net/x/thumbnail.jpg")
        );
        assert_eq!(thumbnail_url(&json!({"thumbnail_url": "../x"}), SITE), None);
    }

    #[test]
    fn names_occur_as_whole_words() {
        assert!(occurs("HannahOwO", "Hannah Owo Riding and Grinding"));
        assert!(occurs("Lucy Cat", "lucycat solo"));
        assert!(occurs("Sweetie Fox", "[SweetieFox] Morning"));
        assert!(!occurs("Cat", "Catwalk show"));
        assert!(!occurs("Al", "Al Bundy"));
        let x = Video {
            performers: vec!["Mila Koi".into()],
            ..video(&v("1", "Morning"), SITE).unwrap()
        };
        assert!(by_creator(&x, &["Mila Koi".into()]));
        assert!(!by_creator(&x, &["Other".into()]));
    }

    #[test]
    fn ids_compare_as_numbers() {
        assert_eq!(cmp_ids("10", "9"), Ordering::Greater);
        assert_eq!(
            cmp_ids("2122637633308856321", "2122637633308856320"),
            Ordering::Greater
        );
        assert_eq!(cmp_ids("5", "5"), Ordering::Equal);
    }

    #[test]
    fn feed_stops_at_the_cursor() {
        let url = |p: u32| format!("{SITE}/api/videos?limit=42&page={p}&order_by=created_at.desc");
        let mut net = Canned::default();
        net.on(
            "GET",
            &url(1),
            200,
            &json!({"data": {"total": 500, "videos": [v("30", "a"), v("29", "b")]}}).to_string(),
        );
        net.on(
            "GET",
            &url(2),
            200,
            &json!({"data": {"total": 500, "videos": [v("28", "c"), v("20", "d")]}}).to_string(),
        );
        let mut pacer = Pacer::new(Duration::ZERO);
        let l = list_new(&net, &mut pacer, SITE, Some("20"), 20).unwrap();
        let ids: Vec<&str> = l.videos.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["30", "29", "28"]);
        assert_eq!(l.newest.as_deref(), Some("30"));
        let l = list_new(&net, &mut pacer, SITE, None, 1).unwrap();
        assert_eq!(l.videos.len(), 2);
        assert!(l.more_left);
    }
}
