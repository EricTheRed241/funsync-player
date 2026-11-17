//! A member's download links from a scene page (port of
//! scripts/vrbangers-lib.mjs). The page carries its data in
//! `<script id="__NUXT_DATA__">`, a Nuxt "devalue" array: every object,
//! array and string is an entry and objects point at entries by index. For a
//! member it also holds signed links,
//! `https://mov.<site>/<prefix>_<folder>/<PREFIX>_<folder>_<Q>K_<projection>.mp4?secure=<token>,<expiry>`,
//! this scene's and hundreds of others' (banners, recommendations). The
//! scene's own list is `downLoadVideoListForFront` of the object with its
//! slug; failing that, the signed links of its folder
//! (`videoSettings.videoShortName`, which can differ from the slug).
//! Trailers and MISC/ extras (the toy script, the 250p preview) are not
//! downloads.
//!
//! Entries are resolved lazily by index (never rebuilt into a tree), so
//! shared and cyclic references cost nothing.

use crate::sites::Site;
use serde_json::Value;
use slr_scout::prefs::VideoCaps;
use std::collections::HashMap;

/// A signed download link.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    /// `8K`.
    pub quality: String,
    pub k: u32,
    pub file: String,
    pub folder: String,
    /// `180x180_3dh`.
    pub projection: String,
    /// The site's size label (`17.98 GB`).
    pub size: Option<String>,
    /// From the scene's `videoFiles`.
    pub bytes: Option<u64>,
    /// When the signature runs out (epoch seconds).
    pub expires: Option<i64>,
    pub url: String,
}

/// devalue's wrapper tags whose value is their first argument (Nuxt's
/// Reactive, Ref …); Set / Map / Date and friends are not wrappers.
const NOT_WRAPPERS: &[&str] = &["Set", "Map", "Date", "BigInt", "RegExp", "null", "Object"];

/// The flat devalue array.
pub struct Flat<'a>(pub &'a [Value]);

impl<'a> Flat<'a> {
    /// The value at `index`, through wrappers; None for devalue's special
    /// negative indices (undefined, NaN …) and out-of-range ones.
    pub fn at(&self, index: &Value) -> Option<&'a Value> {
        let mut i = index.as_u64()? as usize;
        for _ in 0..32 {
            let v = self.0.get(i)?;
            match v.as_array().and_then(|a| Some((a.first()?.as_str()?, a))) {
                Some((tag, a)) if !NOT_WRAPPERS.contains(&tag) => {
                    i = a.get(1)?.as_u64()? as usize;
                }
                _ => return Some(v),
            }
        }
        None
    }

    /// An object's field, resolved.
    pub fn field(&self, obj: &'a Value, key: &str) -> Option<&'a Value> {
        self.at(obj.get(key)?)
    }

    pub fn str_field(&self, obj: &'a Value, key: &str) -> Option<&'a str> {
        self.field(obj, key)?.as_str()
    }

    /// An array's (or Set's) items, resolved.
    pub fn items(&self, arr: &'a Value) -> Vec<&'a Value> {
        let Some(a) = arr.as_array() else {
            return Vec::new();
        };
        let start = usize::from(a.first().is_some_and(Value::is_string));
        a[start..].iter().filter_map(|i| self.at(i)).collect()
    }
}

/// The scene object for `slug`: an object with that slug and
/// `videoSettings`; one with a download list first, then the fullest (the
/// page's own copy has more fields than a recommendation card).
pub fn find_scene<'a>(flat: &Flat<'a>, slug: &str) -> Option<&'a Value> {
    let rank = |o: &'a Value| {
        let listed = flat
            .field(o, "downLoadVideoListForFront")
            .is_some_and(|l| !flat.items(l).is_empty());
        usize::from(listed) * 1000 + o.as_object().map_or(0, |m| m.len())
    };
    flat.0
        .iter()
        .filter(|o| {
            o.is_object()
                && o.get("videoSettings").is_some()
                && flat.str_field(o, "slug") == Some(slug)
        })
        .max_by_key(|o| rank(o))
}

/// A signed link of `site` → its parts; None for anything else.
pub fn parse_link(site: &Site, url: &str) -> Option<Link> {
    let rest = url.strip_prefix(&format!("{}/{}_", site.mov, site.folder_prefix))?;
    let (folder, rest) = rest.split_once('/')?;
    let (file, query) = rest.split_once('?')?;
    let token = query.strip_prefix("secure=")?;
    let token = token.split(['&', '#']).next()?;
    let middle = file
        .strip_prefix(&format!("{}_{folder}_", site.file_prefix))?
        .strip_suffix(".mp4")?;
    let (q, projection) = middle.split_once('_')?;
    let k: u32 = q.strip_suffix(['K', 'k'])?.parse().ok()?;
    let ok_projection = !projection.is_empty()
        && projection
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_');
    if folder.is_empty() || folder.contains('?') || !ok_projection || token.is_empty() {
        return None;
    }
    Some(Link {
        quality: format!("{k}K"),
        k,
        file: file.to_string(),
        folder: folder.to_string(),
        projection: projection.to_ascii_lowercase(),
        size: None,
        bytes: None,
        expires: token
            .rsplit(',')
            .next()
            .and_then(|e| e.parse::<i64>().ok())
            .filter(|e| *e > 0),
        url: url.to_string(),
    })
}

/// A trailer or a MISC/ preview (`…_250p.mp4`), not a download.
fn is_extra(l: &Link) -> bool {
    let f = l.file.to_ascii_lowercase();
    f.contains("_trailer_")
        || f.strip_suffix("p.mp4").is_some_and(|s| {
            s.rsplit('_')
                .next()
                .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        })
}

/// The folder holding `slug`'s videos among `folders`: the scene's own
/// short name, else the slug's folder (`one-last-bang` → `one_last_bang`),
/// else the shortest folder starting with it (`anal_virginity` →
/// `anal_virginity_remastered`).
pub fn scene_folder<'a>(folders: &[&'a str], slug: &str, short: Option<&str>) -> Option<&'a str> {
    if let Some(s) = short
        && let Some(f) = folders.iter().find(|f| **f == s)
    {
        return Some(f);
    }
    let key = slug.to_lowercase().replace('-', "_");
    if let Some(f) = folders.iter().find(|f| **f == key) {
        return Some(f);
    }
    let prefix = format!("{key}_");
    folders
        .iter()
        .filter(|f| f.starts_with(&prefix))
        .min_by_key(|f| f.len())
        .copied()
}

/// This scene's downloads in a `#__NUXT_DATA__` array, one per file,
/// highest quality first.
pub fn scene_links(site: &Site, data: &Value, slug: &str, short: Option<&str>) -> Vec<Link> {
    let Some(raw) = data.as_array() else {
        return Vec::new();
    };
    let flat = Flat(raw);
    let scene = find_scene(&flat, slug);
    let mut picked: Vec<Link> = scene
        .and_then(|o| flat.field(o, "downLoadVideoListForFront"))
        .map(|l| flat.items(l))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| {
            let mut link = parse_link(site, flat.str_field(e, "url")?)?;
            link.size = flat.str_field(e, "size").map(str::to_string);
            Some(link)
        })
        .filter(|l| !is_extra(l))
        .collect();
    if picked.is_empty() {
        let all: Vec<Link> = raw
            .iter()
            .filter_map(Value::as_str)
            .filter(|s| s.contains(".mp4?secure="))
            .filter_map(|s| parse_link(site, s))
            .filter(|l| !is_extra(l))
            .collect();
        let folders: Vec<&str> = all.iter().map(|l| l.folder.as_str()).collect();
        let own = short.or_else(|| {
            scene
                .and_then(|o| flat.field(o, "videoSettings"))
                .and_then(|v| flat.str_field(v, "videoShortName"))
        });
        if let Some(folder) = scene_folder(&folders, slug, own).map(str::to_string) {
            picked = all.into_iter().filter(|l| l.folder == folder).collect();
        }
    }
    let bytes: HashMap<String, u64> = scene
        .and_then(|o| flat.field(o, "videoFiles"))
        .map(|f| flat.items(f))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|f| {
            Some((
                flat.str_field(f, "qualityKey")?.to_string(),
                flat.field(f, "fileSize")?.as_u64()?,
            ))
        })
        .collect();
    let mut out: Vec<Link> = Vec::new();
    for mut l in picked {
        if out.iter().any(|o| o.file == l.file) {
            continue;
        }
        l.bytes = bytes
            .get(&format!("{}_{}", l.quality, l.projection))
            .copied();
        out.push(l);
    }
    out.sort_by(|a, b| b.k.cmp(&a.k).then(a.projection.cmp(&b.projection)));
    out
}

/// The highest K label a VR video may have: the cap's height over 540
/// (4320 → 8K; the sites' K is nominal width / 960, 2:1 stereo).
pub fn max_k(caps: &VideoCaps) -> u32 {
    (caps.vr_max_height / 540) as u32
}

/// The download to take: the highest quality within the VR caps (≤
/// [`max_k`], the scene's fps ≤ `vrMaxFps` when known), stereo 180 before
/// mono 360. None when nothing fits.
pub fn choose<'a>(links: &'a [Link], fps: Option<f64>, caps: &VideoCaps) -> Option<&'a Link> {
    if fps.is_some_and(|f| f > caps.vr_max_fps) {
        return None;
    }
    let mono = |l: &Link| l.projection.contains("mono");
    links
        .iter()
        .filter(|l| l.k <= max_k(caps))
        .min_by(|a, b| mono(a).cmp(&mono(b)).then(b.k.cmp(&a.k)))
}

/// The `#__NUXT_DATA__` array of a page's HTML; None when absent or broken.
pub fn nuxt_data_of(html: &str) -> Option<Value> {
    let at = html.find("id=\"__NUXT_DATA__\"")?;
    let open = at + html[at..].find('>')? + 1;
    let close = open + html[open..].find("</script>")?;
    serde_json::from_str(&html[open..close]).ok()
}

/// A signed link without its token, for logs.
pub fn redact(url: &str) -> String {
    match url.split_once("?secure=") {
        Some((head, rest)) => {
            let tail = rest.find([',', '&']).map_or("", |i| &rest[i..]);
            format!("{head}?secure=<redacted>{tail}")
        }
        None => url.to_string(),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::sites::by_key;
    use serde_json::json;

    pub fn link(folder: &str, q: &str, proj: &str) -> String {
        format!(
            "https://mov.vrbangers.com/vrb_{folder}/VRBANGERS_{folder}_{q}_{proj}.mp4?secure=TOKEN123,1800000000"
        )
    }

    /// A page payload: the scene (slug `wet-adventures`, folder
    /// `wet_adventures_remastered`) with a download list, a recommendation
    /// card of it, and another scene's links.
    pub fn payload(with_list: bool) -> Value {
        let mut flat = vec![
            json!(["Reactive", 1]),            // 0
            json!({"scenes": 2, "other": 15}), // 1
            json!([3, 9]),                     // 2
            json!({"slug": 4, "videoSettings": 5, "downLoadVideoListForFront": 7, "videoFiles": 20, "title": 8}), // 3
            json!("wet-adventures"),                // 4
            json!({"videoShortName": 6}),           // 5
            json!("wet_adventures_remastered"),     // 6
            json!(["ShallowReactive", 21]),         // 7
            json!("Wet Adventures"),                // 8
            json!({"slug": 4, "videoSettings": 5}), // 9 (a card)
            json!({"url": 11, "size": 12}),         // 10
            json!(link("wet_adventures_remastered", "8K", "180x180_3dh")), // 11
            json!("17.98 GB"),                      // 12
            json!({"url": 14}),                     // 13
            json!(link("wet_adventures_remastered", "6K", "180x180_3dh")), // 14
            json!([16, 17, 18]),                    // 15
            json!(link("other_scene", "8K", "180x180_3dh")), // 16
            json!(link("wet_adventures_remastered", "4K", "180x180_3dh")), // 17
            json!(
                "https://mov.vrbangers.com/vrb_wet_adventures_remastered/MISC/VRBANGERS_wet_adventures_remastered_250p.mp4?secure=x,1"
            ), // 18
            json!({"url": 23}),                     // 19
            json!([22]),                            // 20
            json!([10, 13, 19, 13]),                // 21
            json!({"qualityKey": 24, "fileSize": 25}), // 22
            json!(link("wet_adventures_remastered", "trailer", "x")), // 23 (not a link)
            json!("8K_180x180_3dh"),                // 24
            json!(19587917314u64),                  // 25
        ];
        if !with_list {
            flat[21] = json!([]);
        }
        Value::Array(flat)
    }

    #[test]
    fn parses_signed_links() {
        let site = by_key("vrbangers").unwrap();
        let l = parse_link(site, &link("one_last_bang", "8K", "180x180_3dh")).unwrap();
        assert_eq!(l.folder, "one_last_bang");
        assert_eq!(l.file, "VRBANGERS_one_last_bang_8K_180x180_3dh.mp4");
        assert_eq!((l.k, l.quality.as_str()), (8, "8K"));
        assert_eq!(l.projection, "180x180_3dh");
        assert_eq!(l.expires, Some(1_800_000_000));
        // Another folder in the file name, no token, another site, no mp4.
        for bad in [
            "https://mov.vrbangers.com/vrb_a/VRBANGERS_b_8K_180x180_3dh.mp4?secure=t,1",
            "https://mov.vrbangers.com/vrb_a/VRBANGERS_a_8K_180x180_3dh.mp4",
            "https://mov.vrconk.com/vrconk_a/VRCONK_a_8K_180x180_3dh.mp4?secure=t,1",
            "https://mov.vrbangers.com/vrb_a/MISC/VRBANGERS_a_toy.funscript?secure=t,1",
            "https://mov.vrbangers.com/vrb_a/VRBANGERS_a_HQ_180x180_3dh.mp4?secure=t,1",
        ] {
            assert!(parse_link(site, bad).is_none(), "{bad}");
        }
        let conk = by_key("vrconk").unwrap();
        assert!(
            parse_link(
                conk,
                "https://mov.vrconk.com/vrconk_a_b/VRCONK_a_b_6K_180x180_3dh.mp4?secure=t,1"
            )
            .is_some()
        );
    }

    #[test]
    fn picks_the_scenes_own_links() {
        let site = by_key("vrbangers").unwrap();
        let links = scene_links(site, &payload(true), "wet-adventures", None);
        let files: Vec<&str> = links.iter().map(|l| l.quality.as_str()).collect();
        assert_eq!(files, ["8K", "6K"], "listed ones only, deduplicated");
        assert_eq!(links[0].size.as_deref(), Some("17.98 GB"));
        assert_eq!(links[0].bytes, Some(19587917314));
        assert_eq!(links[1].bytes, None);
        // No list: the scene folder's links among all strings.
        let links = scene_links(site, &payload(false), "wet-adventures", None);
        let files: Vec<&str> = links.iter().map(|l| l.quality.as_str()).collect();
        assert_eq!(files, ["8K", "6K", "4K"]);
        assert!(
            links
                .iter()
                .all(|l| l.folder == "wet_adventures_remastered")
        );
        // An unknown slug: nothing.
        assert!(scene_links(site, &payload(true), "nope", None).is_empty());
        assert!(scene_links(site, &json!({}), "wet-adventures", None).is_empty());
    }

    #[test]
    fn finds_the_scene_folder() {
        let f = [
            "anal_virginity_remastered",
            "anal_virginity_2",
            "one_last_bang",
        ];
        assert_eq!(
            scene_folder(&f, "one-last-bang", None),
            Some("one_last_bang")
        );
        assert_eq!(
            scene_folder(&f, "anal-virginity", None),
            Some("anal_virginity_2")
        );
        assert_eq!(
            scene_folder(&f, "x", Some("anal_virginity_remastered")),
            Some("anal_virginity_remastered")
        );
        assert_eq!(scene_folder(&f, "nothing", None), None);
    }

    fn l(k: u32, proj: &str) -> Link {
        Link {
            quality: format!("{k}K"),
            k,
            file: format!("f{k}{proj}"),
            folder: "f".into(),
            projection: proj.into(),
            size: None,
            bytes: None,
            expires: None,
            url: String::new(),
        }
    }

    #[test]
    fn chooses_within_the_caps() {
        let caps = slr_scout::prefs::load().unwrap().video_caps;
        assert_eq!(max_k(&caps), 8);
        let links = [
            l(10, "180x180_3dh"),
            l(8, "360_mono"),
            l(8, "180x180_3dh"),
            l(6, "180x180_3dh"),
        ];
        assert_eq!(choose(&links, Some(60.0), &caps).unwrap().k, 8);
        assert_eq!(
            choose(&links, None, &caps).unwrap().projection,
            "180x180_3dh"
        );
        // Mono only when nothing stereo fits.
        let mono = [l(8, "360_mono"), l(12, "180x180_3dh")];
        assert_eq!(choose(&mono, None, &caps).unwrap().projection, "360_mono");
        assert!(choose(&links, Some(caps.vr_max_fps + 30.0), &caps).is_none());
        assert!(choose(&[l(12, "180x180_3dh")], None, &caps).is_none());
    }

    #[test]
    fn reads_the_page_data_and_redacts() {
        let html = r#"<html><script type="application/json" data-nuxt-data="nuxt-app" id="__NUXT_DATA__">[1,"x"]</script></html>"#;
        assert_eq!(nuxt_data_of(html), Some(json!([1, "x"])));
        assert_eq!(nuxt_data_of("<html></html>"), None);
        assert_eq!(
            nuxt_data_of(r#"<script id="__NUXT_DATA__">[oops</script>"#),
            None
        );
        assert_eq!(
            redact(&link("a", "8K", "180x180_3dh")),
            "https://mov.vrbangers.com/vrb_a/VRBANGERS_a_8K_180x180_3dh.mp4?secure=<redacted>,1800000000"
        );
        assert_eq!(redact("https://x/y"), "https://x/y");
    }
}
