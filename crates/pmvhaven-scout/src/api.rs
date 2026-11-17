//! PMVHaven's public API (no login): the list of videos with a funscript,
//! 100 a page (`GET /api/videos?funscriptOnly=true&limit=100&page=N` →
//! `{videos: […], pagination: {totalPages, total}}`), and one video's
//! details (`GET /api/videos/<_id>` → `{data: {…}}`), the only place that
//! names its scripts: the uploader's `funScriptUrl` and the community's
//! `userFunScripts[]`. Files (videos, scripts, stills) are public on PMVHaven's
//! S3 bucket.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SITE: &str = "https://pmvhaven.com";
pub const PAGE_SIZE: u32 = 100;

pub fn list_url(page: u32) -> String {
    format!("{SITE}/api/videos?funscriptOnly=true&limit={PAGE_SIZE}&page={page}")
}

pub fn detail_url(id: &str) -> String {
    format!("{SITE}/api/videos/{id}")
}

/// The video's page: `/video/<title slug>_<id>`, as the site links it.
pub fn page_url(id: &str, title: &str) -> String {
    let slug = slr_scout::score::slug(title);
    if slug.is_empty() {
        format!("{SITE}/video/{id}")
    } else {
        format!("{SITE}/video/{slug}_{id}")
    }
}

/// A video as the list shows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub uploader: String,
    pub original_creator: Option<String>,
    /// Performers (`starsTags`, and `featuredStar` when set).
    pub stars: Vec<String>,
    pub video_url: Option<String>,
    /// The largest thumbnail (`thumbnailSizes.lg`, else `thumbnailUrl`).
    pub thumbnail: Option<String>,
    pub duration_s: Option<u64>,
    pub width: Option<u64>,
    pub height: Option<u64>,
    /// `aspectRatio`, else width / height.
    pub aspect: Option<f64>,
    pub views: u64,
    pub likes: u64,
    pub gay: bool,
    pub trans: bool,
    pub extreme: bool,
    pub voice_over: bool,
    /// `tags` then `top5Tags`, as PMVHaven spells them, each once.
    pub tags: Vec<String>,
    /// `YYYY-MM-DD` of `releaseDate`, else `uploadDate`.
    pub date: Option<String>,
}

fn text(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// An `https` URL, else nothing (the deck shows only those).
fn https(v: &Value) -> Option<String> {
    text(v).filter(|u| u.starts_with("https://"))
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(text)
        .collect()
}

/// A list item; `None` without an id or title.
pub fn video(v: &Value) -> Option<Video> {
    let id = text(&v["_id"])?;
    let title = text(&v["title"])?;
    let mut stars = strings(&v["starsTags"]);
    if let Some(f) = text(&v["featuredStar"])
        && !stars.iter().any(|s| s.eq_ignore_ascii_case(&f))
    {
        stars.insert(0, f);
    }
    let mut tags: Vec<String> = Vec::new();
    for t in strings(&v["tags"])
        .into_iter()
        .chain(strings(&v["top5Tags"]))
    {
        if !tags.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
            tags.push(t);
        }
    }
    let width = v["width"].as_u64().filter(|w| *w > 0);
    let height = v["height"].as_u64().filter(|h| *h > 0);
    let aspect = v["aspectRatio"]
        .as_f64()
        .filter(|a| *a > 0.0)
        .or_else(|| Some(width? as f64 / height? as f64));
    let date = text(&v["releaseDate"])
        .or_else(|| text(&v["uploadDate"]))
        .filter(|d| d.len() >= 10 && d.is_char_boundary(10))
        .map(|d| d[..10].to_string());
    Some(Video {
        id,
        title,
        uploader: text(&v["uploader"])
            .or_else(|| text(&v["uploaderUsername"]))
            .unwrap_or_default(),
        original_creator: text(&v["originalCreator"]),
        stars,
        video_url: https(&v["videoUrl"]),
        thumbnail: https(&v["thumbnailSizes"]["lg"]["url"]).or_else(|| https(&v["thumbnailUrl"])),
        duration_s: v["durationSeconds"].as_u64().filter(|d| *d > 0),
        width,
        height,
        aspect,
        views: v["views"].as_u64().unwrap_or(0),
        likes: v["likes"].as_u64().unwrap_or(0),
        gay: v["hasGayContent"].as_bool() == Some(true),
        trans: v["hasTransContent"].as_bool() == Some(true),
        extreme: v["hasExtremeContent"].as_bool() == Some(true),
        voice_over: v["hasVoiceOver"].as_bool() == Some(true),
        tags,
        date,
    })
}

/// One list page.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub videos: Vec<Video>,
    pub total_pages: u32,
    pub total: u64,
}

/// A list answer (`videos`, or `data` in the older shape).
pub fn page(v: &Value) -> Result<Page, String> {
    if v["success"].as_bool() == Some(false) {
        return Err("PMVHaven answered success: false".into());
    }
    let list = v["videos"]
        .as_array()
        .or_else(|| v["data"].as_array())
        .ok_or("PMVHaven's list has no videos array")?;
    let p = &v["pagination"];
    Ok(Page {
        videos: list.iter().filter_map(video).collect(),
        total_pages: p["totalPages"].as_u64().unwrap_or(1) as u32,
        total: p["total"].as_u64().unwrap_or(list.len() as u64),
    })
}

/// A community script (`userFunScripts[]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityScript {
    pub username: String,
    pub url: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub likes: u64,
    #[serde(default)]
    pub uploaded: Option<String>,
}

/// What the scout keeps of a video's details (cached across runs).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    #[serde(default)]
    pub description: String,
    /// The uploader's own script (`funScriptUrl`).
    #[serde(default)]
    pub uploader_script: Option<String>,
    #[serde(default)]
    pub community: Vec<CommunityScript>,
    /// Timeline stills (`timelineThumbnails`), in playing order.
    #[serde(default)]
    pub stills: Vec<String>,
    #[serde(default)]
    pub file_size: Option<u64>,
}

/// A detail answer's `data`.
pub fn detail(v: &Value) -> Result<Detail, String> {
    let d = &v["data"];
    if !d.is_object() {
        return Err("PMVHaven's video detail has no data".into());
    }
    let community = d["userFunScripts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            Some(CommunityScript {
                username: text(&s["username"]).unwrap_or_default(),
                url: https(&s["scriptUrl"])?,
                description: text(&s["description"]).unwrap_or_default(),
                likes: s["likes"].as_u64().unwrap_or(0),
                uploaded: text(&s["uploadDate"]),
            })
        })
        .collect();
    let mut stills: Vec<(f64, String)> = d["timelineThumbnails"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| Some((t["captureTime"].as_f64().unwrap_or(0.0), https(&t["url"])?)))
        .collect();
    stills.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut urls: Vec<String> = Vec::new();
    for (_, u) in stills {
        if !urls.contains(&u) {
            urls.push(u);
        }
    }
    Ok(Detail {
        description: text(&d["description"]).unwrap_or_default(),
        uploader_script: https(&d["funScriptUrl"]),
        community,
        stills: urls,
        file_size: d["fileSize"].as_u64().filter(|s| *s > 0),
    })
}

/// At most `n` stills spread over the video (first, last and evenly between).
pub fn spread(stills: &[String], n: usize) -> Vec<String> {
    if stills.len() <= n {
        return stills.to_vec();
    }
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![stills[stills.len() / 2].clone()];
    }
    (0..n)
        .map(|i| stills[i * (stills.len() - 1) / (n - 1)].clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub fn item() -> Value {
        json!({
            "_id": "6afc22aff41606376b321559", "title": "TRY NOT TO CUM CHALLENGE 20 - VERSION 2.0",
            "uploader": "Cowgirlfan101", "uploaderUsername": "Cowgirlfan101",
            "videoUrl": "https://pmvhavencloud.s3.example/videos/a.mp4",
            "thumbnailUrl": "https://pmvhavencloud.s3.example/t/thumb_lg.webp",
            "thumbnailSizes": {"lg": {"url": "https://pmvhavencloud.s3.example/t/lg.webp"},
                               "md": {"url": "https://pmvhavencloud.s3.example/t/md.webp"}},
            "views": 930, "durationSeconds": 644, "aspectRatio": 1.7778, "featuredStar": "",
            "originalCreator": "", "width": 1920, "height": 1080, "likes": 22,
            "hasGayContent": false, "hasTransContent": false, "hasExtremeContent": false,
            "hasVoiceOver": true, "releaseDate": "2026-11-17T16:14:13.522Z",
            "tags": ["Challenge", "Pmv，music", "Blowjob"], "top5Tags": ["Challenge", "Cum"],
            "starsTags": ["Dainty wilder", "Elly clutch"]
        })
    }

    #[test]
    fn parses_list_items() {
        let v = video(&item()).unwrap();
        assert_eq!(v.id, "6afc22aff41606376b321559");
        assert_eq!(v.uploader, "Cowgirlfan101");
        assert_eq!(v.original_creator, None);
        assert_eq!(v.stars, ["Dainty wilder", "Elly clutch"]);
        assert_eq!(
            v.thumbnail.as_deref(),
            Some("https://pmvhavencloud.s3.example/t/lg.webp")
        );
        assert_eq!(v.tags, ["Challenge", "Pmv，music", "Blowjob", "Cum"]);
        assert_eq!(v.date.as_deref(), Some("2026-11-17"));
        assert_eq!((v.duration_s, v.likes, v.views), (Some(644), 22, 930));
        assert!(v.voice_over && !v.gay);
        // No aspect ratio: from the frame size; a featured star leads.
        let mut i = item();
        i["aspectRatio"] = Value::Null;
        i["width"] = 3840.into();
        i["height"] = 1920.into();
        i["featuredStar"] = "Riley Reid".into();
        i["videoUrl"] = "http://insecure/a.mp4".into();
        let v = video(&i).unwrap();
        assert_eq!(v.aspect, Some(2.0));
        assert_eq!(v.stars[0], "Riley Reid");
        assert_eq!(v.video_url, None);
        assert!(video(&json!({"_id": "x"})).is_none());
    }

    #[test]
    fn parses_pages() {
        let p = page(&json!({"success": true, "videos": [item(), {"bad": 1}],
            "pagination": {"page": 1, "total": 1258, "totalPages": 13}}))
        .unwrap();
        assert_eq!((p.videos.len(), p.total_pages, p.total), (1, 13, 1258));
        let p = page(&json!({"success": true, "data": [item()]})).unwrap();
        assert_eq!((p.videos.len(), p.total_pages), (1, 1));
        assert!(page(&json!({"success": false})).is_err());
        assert!(page(&json!({})).is_err());
    }

    #[test]
    fn parses_details() {
        let d = detail(&json!({"data": {
            "description": "re-edit",
            "funScriptUrl": "https://s3/funscripts/1_a_x.funscript",
            "fileSize": 423925450,
            "userFunScripts": [
                {"username": "Goonretto", "scriptUrl": "https://s3/f/v/u_1_A.funscript",
                 "description": "Quick Audio Based FunScript", "likes": 2, "uploadDate": "2026-08-05T18:35:07.422Z"},
                {"username": "x", "scriptUrl": "http://insecure"}
            ],
            "timelineThumbnails": [
                {"url": "https://s3/t/2.webp", "captureTime": 120.5},
                {"url": "https://s3/t/1.webp", "captureTime": 58.6},
                {"url": "https://s3/t/1.webp", "captureTime": 60.0}
            ]}}))
        .unwrap();
        assert_eq!(
            d.uploader_script.as_deref(),
            Some("https://s3/funscripts/1_a_x.funscript")
        );
        assert_eq!(d.community.len(), 1);
        assert_eq!(d.community[0].likes, 2);
        assert_eq!(d.stills, ["https://s3/t/1.webp", "https://s3/t/2.webp"]);
        assert_eq!(d.file_size, Some(423925450));
        assert!(detail(&json!({"success": false})).is_err());
    }

    #[test]
    fn spreads_stills_and_names_pages() {
        let s: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        assert_eq!(spread(&s, 4), ["0", "3", "6", "9"]);
        assert_eq!(spread(&s[..3], 4), ["0", "1", "2"]);
        assert_eq!(spread(&s, 1), ["5"]);
        assert_eq!(
            page_url("6afc", "TRY NOT TO CUM 2.0"),
            "https://pmvhaven.com/video/try-not-to-cum-2-0_6afc"
        );
        assert_eq!(page_url("6afc", "!!"), "https://pmvhaven.com/video/6afc");
    }
}
