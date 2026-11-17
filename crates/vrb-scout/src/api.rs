//! The content API (`content.<site>/api/content/v1`, public, JSON
//! `{status: {code: 1}, data: …}`):
//!
//! - `videos?page=N&limit=100&sort=latest`: the listing, `{items, pages}`.
//!   Items carry the id, slug, title, models, poster, publish time,
//!   `videoSettings` (duration, `videoShortName`, `downloadsQuality` such as
//!   `8K_180x180_3dh`), `viewCount`, `hasToyScript` and
//!   `settingsForPlaya.toyScript.permalink` (the public toy script). No
//!   categories and no update time.
//! - `videos/<slug>`: one scene with `categories`, `galleryImages`,
//!   `videoTechBar.fps` (ids into the digest), `videoFiles` and `updatedAt`.
//! - `digest`: the site's lookup tables; `videoFps` maps fps ids to values.
//!
//! The listing has no update time, so a scene's detail is fetched again only
//! when its listing entry changes: [`fingerprint`] hashes the entry without
//! its view count.

use crate::sites::Site;
use serde_json::Value;
use slr_scout::http::{self, Http, NetError, Pacer, Request};
use std::collections::BTreeMap;
use std::io::Write;

/// A listing entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub short_name: Option<String>,
    pub has_script: bool,
    /// `settingsForPlaya.toyScript.permalink` (https).
    pub toy_script: Option<String>,
    pub models: Vec<String>,
    /// The poster's permalink as given (relative to `content.<site>`).
    pub poster: Option<String>,
    pub published_at: Option<i64>,
    pub duration_s: Option<u64>,
    /// `downloadsQuality`: `<Q>_<projection>` keys, e.g. `8K_180x180_3dh`.
    pub qualities: Vec<String>,
    pub views: u64,
    /// [`fingerprint`] of the entry.
    pub key: String,
}

/// FNV-1a 64 of the entry's JSON without `viewCount` (keys sorted: serde_json
/// maps are ordered), as 16 hex digits. Stable across Rust versions, unlike
/// the std hasher.
pub fn fingerprint(item: &Value) -> String {
    let mut v = item.clone();
    if let Some(o) = v.as_object_mut() {
        o.remove("viewCount");
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in v.to_string().bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// An id as a string (the API gives hex strings; numbers are accepted too).
pub fn id_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn data(v: &Value, url: &str) -> Result<Value, NetError> {
    if v["status"]["code"].as_i64() != Some(1) {
        return Err(NetError::Failed(format!(
            "{}: status {}",
            url.split('?').next().unwrap_or(url),
            v["status"]["code"]
        )));
    }
    Ok(v["data"].clone())
}

/// A non-empty trimmed string.
fn text(v: &Value) -> Option<&str> {
    v.as_str().map(str::trim).filter(|t| !t.is_empty())
}

/// A listing entry; None without an id or slug.
pub fn listed(item: &Value) -> Option<Listed> {
    let s = text;
    let settings = &item["videoSettings"];
    Some(Listed {
        id: id_of(&item["id"])?,
        slug: s(&item["slug"])?.to_string(),
        title: s(&item["title"]).unwrap_or_default().to_string(),
        short_name: s(&settings["videoShortName"]).map(str::to_string),
        has_script: item["hasToyScript"].as_bool() == Some(true),
        toy_script: s(&item["settingsForPlaya"]["toyScript"]["permalink"])
            .filter(|u| u.starts_with("https://"))
            .map(str::to_string),
        models: item["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| s(&m["title"]).map(str::to_string))
            .collect(),
        poster: s(&item["poster"]["permalink"]).map(str::to_string),
        published_at: item["publishedAt"].as_i64().filter(|t| *t > 0),
        duration_s: settings["duration"].as_u64().filter(|d| *d > 0),
        qualities: settings["downloadsQuality"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|q| s(q).map(str::to_string))
            .collect(),
        views: item["viewCount"].as_u64().unwrap_or(0),
        key: fingerprint(item),
    })
}

/// A listing page: its entries and the page count.
pub fn parse_list(v: &Value, url: &str) -> Result<(Vec<Listed>, u32), NetError> {
    let d = data(v, url)?;
    let items = d["items"]
        .as_array()
        .ok_or_else(|| NetError::Failed(format!("{url}: no items")))?;
    let pages = d["pages"].as_u64().unwrap_or(1).min(1000) as u32;
    Ok((items.iter().filter_map(listed).collect(), pages))
}

/// Every listed video of a site, paced; progress on `log`.
pub fn walk(
    http: &dyn Http,
    pacer: &mut Pacer,
    site: &Site,
    log: &mut dyn Write,
) -> Result<Vec<Listed>, NetError> {
    let mut out: Vec<Listed> = Vec::new();
    let mut page = 1;
    loop {
        let url = site.list_url(page);
        let v = http::paced_json(http, pacer, &Request::get(&url))?;
        let (items, pages) = parse_list(&v, &url)?;
        let _ = writeln!(log, "  {} page {page}/{pages}", site.name);
        for item in items {
            if !out.iter().any(|o| o.id == item.id) {
                out.push(item);
            }
        }
        if page >= pages {
            return Ok(out);
        }
        page += 1;
    }
}

/// What a scene's detail adds to its listing entry.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Detail {
    pub updated_at: Option<i64>,
    /// Category names (their slug when a name is missing).
    pub categories: Vec<String>,
    /// Gallery stills (XL, else L, else the original), https, at most [`IMAGES`].
    pub images: Vec<String>,
    pub fps: Option<f64>,
    /// See-through (AR) video: `videoSettings.transparency.mode` set.
    pub passthrough: bool,
}

/// Stills kept per scene, after the thumbnail.
pub const IMAGES: usize = 4;

/// The fps lookup of a digest (`data.digest.videoFps: [{id, name}]`).
pub fn fps_table(v: &Value) -> BTreeMap<u64, f64> {
    v["data"]["digest"]["videoFps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let id = e["id"].as_u64()?;
            let fps = match &e["name"] {
                Value::Number(n) => n.as_f64()?,
                Value::String(s) => s.trim().parse().ok()?,
                _ => return None,
            };
            Some((id, fps))
        })
        .collect()
}

pub fn parse_detail(
    v: &Value,
    url: &str,
    site: &Site,
    fps: &BTreeMap<u64, f64>,
) -> Result<Detail, NetError> {
    let d = data(v, url)?;
    let item = &d["item"];
    if !item.is_object() {
        return Err(NetError::Failed(format!("{url}: no item")));
    }
    let categories = item["categories"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            c["name"]
                .as_str()
                .or(c["slug"].as_str())
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string)
        })
        .collect();
    let images = item["galleryImages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| {
            let previews = g["previews"].as_array();
            let sized = |alias: &str| {
                previews?
                    .iter()
                    .find(|p| p["sizeAlias"] == alias)
                    .and_then(|p| p["permalink"].as_str())
            };
            site.asset(
                sized("XL")
                    .or_else(|| sized("L"))
                    .or(g["permalink"].as_str())?,
            )
        })
        .take(IMAGES)
        .collect();
    // The scene's fastest listed frame rate.
    let fps = item["videoTechBar"]["fps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|id| fps.get(&id.as_u64()?).copied())
        .reduce(f64::max);
    Ok(Detail {
        updated_at: item["updatedAt"].as_i64(),
        categories,
        images,
        fps,
        passthrough: item["videoSettings"]["transparency"]["mode"]
            .as_u64()
            .is_some_and(|m| m > 0),
    })
}

/// One scene's detail, paced.
pub fn detail(
    http: &dyn Http,
    pacer: &mut Pacer,
    site: &Site,
    slug: &str,
    fps: &BTreeMap<u64, f64>,
) -> Result<Detail, NetError> {
    let url = site.detail_url(slug);
    let v = http::paced_json(http, pacer, &Request::get(&url))?;
    parse_detail(&v, &url, site, fps)
}

/// The site's fps table, paced; empty when the digest can't be read.
pub fn digest_fps(
    http: &dyn Http,
    pacer: &mut Pacer,
    site: &Site,
) -> Result<BTreeMap<u64, f64>, NetError> {
    let v = http::paced_json(http, pacer, &Request::get(site.digest_url()))?;
    Ok(fps_table(&v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sites::by_key;
    use serde_json::json;

    pub fn item() -> Value {
        json!({
            "id": "6ad73eaf80113a863f036562", "slug": "sin-sisters-a-porn-parody",
            "title": "Sin Sisters (A Porn Parody)", "hasToyScript": true,
            "settingsForPlaya": {"toyScript": {"permalink": "https://mov.vrbangers.com/vrb_sin_sisters_a_porn_parody/MISC/VRBANGERS_sin_sisters_a_porn_parody_toy.funscript"}},
            "models": [{"title": "Melissa Stratton", "slug": "melissa-stratton"}, {"title": " "}, {"title": "Little Puck"}],
            "poster": {"permalink": "/uploads/2026/10/p.jpg"}, "publishedAt": 1792738860,
            "videoSettings": {"duration": 3115, "videoShortName": "sin_sisters_a_porn_parody",
                "downloadsQuality": ["2K_180x180_3dh", "8K_180x180_3dh"]},
            "viewCount": 2090
        })
    }

    #[test]
    fn parses_the_listing() {
        let page = json!({"status": {"code": 1}, "data": {"pages": 10, "items": [
            item(),
            {"id": 7, "slug": "no-script", "hasToyScript": false, "settingsForPlaya": [], "viewCount": 5},
            {"slug": "no-id"},
            {"id": "x"}
        ]}});
        let (items, pages) = parse_list(&page, "u").unwrap();
        assert_eq!(pages, 10);
        assert_eq!(items.len(), 2);
        let a = &items[0];
        assert_eq!(a.id, "6ad73eaf80113a863f036562");
        assert_eq!(a.title, "Sin Sisters (A Porn Parody)");
        assert_eq!(a.short_name.as_deref(), Some("sin_sisters_a_porn_parody"));
        assert!(a.has_script);
        assert!(a.toy_script.as_deref().unwrap().ends_with("_toy.funscript"));
        assert_eq!(a.models, ["Melissa Stratton", "Little Puck"]);
        assert_eq!(a.poster.as_deref(), Some("/uploads/2026/10/p.jpg"));
        assert_eq!(a.published_at, Some(1792738860));
        assert_eq!(a.duration_s, Some(3115));
        assert_eq!(a.qualities, ["2K_180x180_3dh", "8K_180x180_3dh"]);
        assert_eq!(a.views, 2090);
        let b = &items[1];
        assert_eq!(b.id, "7");
        assert!(!b.has_script && b.toy_script.is_none() && b.short_name.is_none());
        let bad = json!({"status": {"code": 422}, "data": null});
        assert!(parse_list(&bad, "u").is_err());
        assert!(parse_list(&json!({"status": {"code": 1}, "data": {}}), "u").is_err());
    }

    #[test]
    fn fingerprints_ignore_the_view_count() {
        let mut a = item();
        let before = fingerprint(&a);
        a["viewCount"] = json!(9999);
        assert_eq!(fingerprint(&a), before);
        a["title"] = json!("Renamed");
        assert_ne!(fingerprint(&a), before);
        assert_eq!(before.len(), 16);
    }

    #[test]
    fn parses_details() {
        let site = by_key("vrbangers").unwrap();
        let fps = fps_table(&json!({"data": {"digest": {"videoFps": [
            {"id": 1, "name": 60}, {"id": 2, "name": "30"}, {"id": 3, "name": "x"}]}}}));
        assert_eq!(fps, BTreeMap::from([(1, 60.0), (2, 30.0)]));
        let v = json!({"status": {"code": 1}, "data": {"item": {
            "updatedAt": 1794950074,
            "categories": [{"slug": "big-tits", "name": "Big Tits"}, {"slug": "creampie"}, {"name": ""}],
            "galleryImages": [
                {"permalink": "/u/1.jpg", "previews": [{"sizeAlias": "L", "permalink": "/u/1_L.jpg"},
                    {"sizeAlias": "XL", "permalink": "/u/1_XL.jpg"}]},
                {"permalink": "/u/2.jpg", "previews": [{"sizeAlias": "L", "permalink": "/u/2_L.jpg"}]},
                {"permalink": "/u/3.jpg"},
                {"permalink": "http://insecure/4.jpg"},
                {"permalink": "/u/5.jpg"}, {"permalink": "/u/6.jpg"}],
            "videoTechBar": {"fps": [2, 1]},
            "videoSettings": {"transparency": {"mode": 0}}
        }}});
        let d = parse_detail(&v, "u", site, &fps).unwrap();
        assert_eq!(d.updated_at, Some(1794950074));
        assert_eq!(d.categories, ["Big Tits", "creampie"]);
        assert_eq!(
            d.images,
            [
                "https://content.vrbangers.com/u/1_XL.jpg",
                "https://content.vrbangers.com/u/2_L.jpg",
                "https://content.vrbangers.com/u/3.jpg",
                "https://content.vrbangers.com/u/5.jpg"
            ]
        );
        assert_eq!(d.fps, Some(60.0));
        assert!(!d.passthrough);
        let ar = json!({"status": {"code": 1}, "data": {"item": {"videoSettings": {"transparency": {"mode": 2}}}}});
        let d = parse_detail(&ar, "u", site, &fps).unwrap();
        assert!(d.passthrough && d.fps.is_none() && d.images.is_empty());
        assert!(
            parse_detail(&json!({"status": {"code": 1}, "data": {}}), "u", site, &fps).is_err()
        );
    }
}
