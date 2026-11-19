//! The scenes the site scouts found, as deck cards: SexLikeReal (`slr-scout.json`,
//! `npm run slr:scout`), VRBangers and VRConk (`vrb-scout.json`, `vrb:scout`), PMVHaven
//! (`pmvhaven-scout.json`, `pmvhaven:scout`) and FapTap (`faptap-scout.json`'s `candidates`,
//! `faptap:scout`). Each source has its own swipes file (crate::swipes); a right swipe is
//! what that scout's `--fetch` downloads.
//!
//! A scene leaves the deck once downloaded (`fetchedAt`), when the library already holds it
//! (`inLibrary`), and when a waiting EroScripts post brings the same video (Covered: the post
//! wins, its script is free). Scores, tag scores and the SLR quota come from the scouts; Rust
//! here only reads and deals.

use crate::deck::{self, Stack, TagScore};
use crate::report::{id_of, str_of, strings_of};
use crate::swipes::{self, Store};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;

/// Where a scene card comes from; also the `<source>` of `/api/scenes/<source>/swipe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Source {
    Slr,
    Vrb,
    Pmvhaven,
    Faptap,
}

impl Source {
    pub const ALL: [Source; 4] = [Source::Slr, Source::Vrb, Source::Pmvhaven, Source::Faptap];

    pub fn name(self) -> &'static str {
        match self {
            Source::Slr => "slr",
            Source::Vrb => "vrb",
            Source::Pmvhaven => "pmvhaven",
            Source::Faptap => "faptap",
        }
    }

    pub fn parse(s: &str) -> Option<Source> {
        Source::ALL.into_iter().find(|src| src.name() == s)
    }

    /// The scout's file in userData.
    pub fn file(self) -> &'static str {
        match self {
            Source::Slr => "slr-scout.json",
            Source::Vrb => "vrb-scout.json",
            Source::Pmvhaven => "pmvhaven-scout.json",
            Source::Faptap => "faptap-scout.json",
        }
    }

    pub fn swipes(self) -> Store {
        match self {
            Source::Slr => swipes::SLR,
            Source::Vrb => swipes::VRB,
            Source::Pmvhaven => swipes::PMVHAVEN,
            Source::Faptap => swipes::FAPTAP,
        }
    }

    /// The file's list of scenes: FapTap's JS scout calls them `candidates`.
    fn list(self) -> &'static str {
        match self {
            Source::Faptap => "candidates",
            _ => "scenes",
        }
    }
}

/// The scenes of `source`'s parsed file with their ids (as swipes keys); entries without a
/// usable id are skipped.
pub fn scenes(source: Source, file: &Value) -> impl Iterator<Item = (String, &Value)> {
    file.get(source.list())
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| Some((swipes::id_key(s.get("id")?)?, s)))
}

/// Whether `id` is a scene in `source`'s parsed file.
pub fn is_scene(source: Source, file: &Value, id: &str) -> bool {
    scenes(source, file).any(|(s, _)| s == id)
}

/// Downloaded already (`fetchedAt`) or in the library (`inLibrary`): never dealt, and a right
/// swipe on it isn't wanted any more.
pub fn is_done(scene: &Value) -> bool {
    str_of(scene, "fetchedAt").is_some() || flag(scene, "inLibrary")
}

/// The axes besides stroke the scene's script has (`script.axes`).
fn script_axes(s: &Value) -> Vec<String> {
    s.get("script")
        .map(|sc| strings_of(sc, "axes").unwrap_or_default())
        .unwrap_or_default()
        .into_iter()
        .filter(|a| !a.is_empty())
        .collect()
}

/// The deck tab a scene goes to: a script with axes besides stroke → Multi, else VR or Flat by
/// `vr`. FapTap scripts are always single-axis.
pub fn stack_of(source: Source, s: &Value) -> Stack {
    if source != Source::Faptap && !script_axes(s).is_empty() {
        Stack::Multi
    } else if flag(s, "vr") {
        Stack::Vr
    } else {
        Stack::Flat
    }
}

/// The videos waiting EroScripts posts bring (swiped, in the library or not): their scenes
/// never show on their own.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Covered {
    /// SLR scene ids (a post video's `slrSceneId`).
    pub slr: HashSet<String>,
    /// VRBangers page slugs (a post video's `slug`, else its URL's), lowercased.
    pub vrb: HashSet<String>,
}

impl Covered {
    /// From every waiting post's scene videos with host `slr` or `vrbangers`.
    pub fn of(waiting: &Value) -> Covered {
        let mut c = Covered::default();
        let videos = waiting
            .get("candidates")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|p| p.get("scenes").and_then(Value::as_array))
            .flatten()
            .filter_map(|s| s.get("video"));
        for v in videos {
            match v.get("host").and_then(Value::as_str) {
                Some("slr") => {
                    if let Some(id) = v.get("slrSceneId").and_then(id_of) {
                        c.slr.insert(id.to_string());
                    }
                }
                Some("vrbangers") => {
                    let slug =
                        str_of(v, "slug").or_else(|| str_of(v, "url").and_then(|u| url_slug(&u)));
                    if let Some(slug) = slug {
                        c.vrb.insert(slug.to_lowercase());
                    }
                }
                _ => {}
            }
        }
        c
    }

    /// Whether a waiting post brings `source`'s scene `id`.
    pub fn covers(&self, source: Source, id: &str, scene: &Value) -> bool {
        match source {
            Source::Slr => self.slr.contains(id),
            Source::Vrb => str_of(scene, "url")
                .and_then(|u| url_slug(&u))
                .is_some_and(|slug| self.vrb.contains(&slug.to_lowercase())),
            Source::Pmvhaven | Source::Faptap => false,
        }
    }
}

/// The last path segment of a page URL (`https://vrbangers.com/video/wet-and-wild/` →
/// `wet-and-wild`), without query or fragment.
fn url_slug(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let path = path.split_once("://").map_or(path, |(_, rest)| rest);
    let (_, path) = path.split_once('/')?;
    path.split('/')
        .rfind(|seg| !seg.is_empty())
        .map(str::to_string)
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn https(u: Option<String>) -> Option<String> {
    u.filter(|u| u.starts_with("https://"))
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneCard {
    /// `slr`, `vrb`, `pmvhaven`, `faptap` or `creators` (crate::creators): where the swipe
    /// goes (`/api/scenes/<source>/swipe`).
    pub source: &'static str,
    /// The site the scene is on (`vrbangers` or `vrconk` for vrb; else the source): the badge.
    pub site: String,
    /// The site's own id, always a string (SLR's numbers too).
    pub id: String,
    /// The scene's page (https only), or null.
    pub url: Option<String>,
    pub title: Option<String>,
    /// The studio, uploader or FapTap author: shown where a post shows its author.
    pub author: Option<String>,
    pub actors: Vec<String>,
    /// The https thumbnail, then the scene's other https stills, then (SLR) its preview frames
    /// on disk (`/frames/slr/…`), at most deck::MAX_IMAGES.
    pub images: Vec<String>,
    pub score: Option<f64>,
    pub score_parts: Value,
    pub vr: bool,
    pub creampie: bool,
    pub runtime_ms: Option<u64>,
    /// Release (SLR, VRBangers) or upload date.
    pub date: Option<String>,
    pub fps: Option<f64>,
    pub passthrough: bool,
    pub pmv: bool,
    pub compilation: bool,
    /// The popularity number the score used (`likes`; SLR's `favorites`).
    pub likes: Option<i64>,
    /// The scene's tags with the user's tag scores when the scout gave them (`tagScores`; then
    /// highest first, ties by name), else in the file's order, unscored.
    pub tags: Vec<SceneTag>,
    /// The best few positive-scored tags (`deck::top_tags`).
    pub top_tags: Vec<TagScore>,
    /// A favourite performer or studio in the scene.
    pub favourite: Option<String>,
    /// Whether a script comes with a download (SLR: `scriptSource`; FapTap: always).
    pub script: bool,
    /// Axes the script has besides stroke (`script.axes`; non-empty = the Multi tab).
    pub axes: Vec<String>,
    /// `{source: "slr", author}` or `{source: "eroscripts", topicId, title, free, multiAxis}`.
    pub handmade: Vec<Value>,
    /// SLR: where a download takes the script from: `eroscripts`, `slr-handcrafted`, `slr-ai`.
    pub script_source: Option<String>,
    pub ai_script: bool,
    /// FapTap: the video's projection (`MONO_FLAT`, …) and the stream height `--fetch` takes.
    pub projection: Option<String>,
    pub quality: Option<u64>,
    /// The yes creator (crate::creators) the scene belongs to: the card's Creator badge; its
    /// score includes the creator bonus (`scoreParts` "yes creator").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    /// A creator post's script is multi-axis (axes unknown; `axes` stays empty).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub multi_axis: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SceneTag {
    pub tag: String,
    pub score: Option<f64>,
}

/// The scene's tags (`tags`) scored by `tagScores` when the scout gave them, else unscored.
fn scene_tags(s: &Value) -> (Vec<SceneTag>, Vec<TagScore>) {
    if s.get("tagScores").is_some_and(Value::is_object) {
        let scored = deck::post_tags(s);
        let top = deck::top_tags(&scored);
        let tags = scored
            .into_iter()
            .map(|t| SceneTag {
                tag: t.tag,
                score: Some(t.score),
            })
            .collect();
        return (tags, top);
    }
    let mut seen = HashSet::new();
    let tags = strings_of(s, "tags")
        .unwrap_or_default()
        .into_iter()
        .filter(|t| !t.is_empty() && seen.insert(t.clone()))
        .map(|tag| SceneTag { tag, score: None })
        .collect();
    (tags, Vec::new())
}

/// The scene's https pictures: `thumbnail` (FapTap's older lists: `thumbnailUrl`), then
/// `images`, each once, at most deck::MAX_IMAGES.
fn scene_images(s: &Value) -> Vec<String> {
    let thumb = https(str_of(s, "thumbnail").or_else(|| str_of(s, "thumbnailUrl")));
    let rest = strings_of(s, "images").unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for url in thumb.into_iter().chain(rest) {
        if out.len() == deck::MAX_IMAGES {
            break;
        }
        if url.starts_with("https://") && !out.contains(&url) {
            out.push(url);
        }
    }
    out
}

/// The scene's page: its https `url`, else what the site's own pattern gives (SLR: the label
/// or id; FapTap: `/v/<id>`).
fn scene_url(source: Source, id: &str, s: &Value) -> Option<String> {
    https(str_of(s, "url")).or_else(|| match source {
        Source::Slr => Some(match str_of(s, "label") {
            Some(l) => format!("https://www.sexlikereal.com/scenes/{l}"),
            None => format!("https://www.sexlikereal.com/scenes/{id}"),
        }),
        Source::Faptap => Some(format!("https://faptap.net/v/{id}")),
        Source::Vrb | Source::Pmvhaven => None,
    })
}

/// The scene as a card.
pub fn card(source: Source, id: String, s: &Value) -> SceneCard {
    let (tags, top_tags) = scene_tags(s);
    let num = |key: &str| s.get(key).and_then(Value::as_f64);
    let seconds = num("durationS").or_else(|| num("duration"));
    let script_source = str_of(s, "scriptSource");
    SceneCard {
        source: source.name(),
        site: str_of(s, "site").unwrap_or_else(|| source.name().to_string()),
        url: scene_url(source, &id, s),
        id,
        title: str_of(s, "title"),
        author: str_of(s, "studio").or_else(|| str_of(s, "author")),
        actors: strings_of(s, "actors").unwrap_or_default(),
        images: scene_images(s),
        score: num("score"),
        score_parts: s.get("scoreParts").cloned().unwrap_or(Value::Null),
        vr: flag(s, "vr"),
        creampie: flag(s, "creampie"),
        runtime_ms: seconds
            .filter(|n| n.is_finite() && *n > 0.0)
            .map(|n| (n * 1000.0).round() as u64),
        date: str_of(s, "date"),
        fps: num("fps"),
        passthrough: flag(s, "passthrough"),
        pmv: flag(s, "pmv"),
        compilation: flag(s, "compilation"),
        likes: s
            .get("likes")
            .or_else(|| s.get("favorites"))
            .and_then(Value::as_i64),
        tags,
        top_tags,
        favourite: str_of(s, "favourite"),
        script: match source {
            Source::Slr => script_source.is_some(),
            Source::Faptap => true,
            Source::Vrb | Source::Pmvhaven => s.get("script").is_some_and(Value::is_object),
        },
        axes: if source == Source::Faptap {
            Vec::new()
        } else {
            script_axes(s)
        },
        handmade: s
            .get("handmade")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        script_source,
        ai_script: flag(s, "aiScript"),
        projection: str_of(s, "projection"),
        quality: s.get("quality").and_then(Value::as_u64),
        creator: None,
        multi_axis: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sources_and_their_files() {
        for src in Source::ALL {
            assert_eq!(Source::parse(src.name()), Some(src));
            assert!(src.file().starts_with(src.name()));
            assert!(src.swipes().file.starts_with(src.name()));
        }
        assert_eq!(Source::parse("eroscripts"), None);
        assert_eq!(Source::parse("SLR"), None);
        assert_eq!(Source::Faptap.file(), "faptap-scout.json");
        assert_eq!(Source::Vrb.swipes().file, "vrb-scout-swipes.json");
    }

    #[test]
    fn tabs_by_script_axes_then_vr() {
        let multi = json!({"vr": true, "script": {"url": "https://x/a.funscript", "axes": ["twist", "roll"]}});
        let vr = json!({"vr": true, "script": {"axes": []}});
        let flat = json!({"vr": false, "script": null});
        for src in [Source::Slr, Source::Vrb, Source::Pmvhaven] {
            assert_eq!(stack_of(src, &multi), Stack::Multi, "{src:?}");
            assert_eq!(stack_of(src, &vr), Stack::Vr, "{src:?}");
            assert_eq!(stack_of(src, &flat), Stack::Flat, "{src:?}");
            assert_eq!(stack_of(src, &json!({"vr": true})), Stack::Vr);
        }
        // FapTap is always single-axis, whatever its list says.
        assert_eq!(stack_of(Source::Faptap, &multi), Stack::Vr);
        assert_eq!(stack_of(Source::Faptap, &json!({"vr": false})), Stack::Flat);
        // An axes list of empty names counts as none.
        assert_eq!(
            stack_of(Source::Vrb, &json!({"script": {"axes": [""]}})),
            Stack::Flat
        );
    }

    #[test]
    fn waiting_posts_cover_their_slr_and_vrbangers_videos() {
        let waiting = json!({"candidates": [
            {"id": 1, "scenes": [
                {"video": {"host": "slr", "slrSceneId": 66249}},
                {"video": {"host": "slr", "slrSceneId": "51207"}},
                {"video": {"host": "vrbangers", "slug": "Full-Body-Massage", "url": "https://vrbangers.com/video/other/"}}]},
            {"id": 2, "scenes": [
                {"video": {"host": "vrbangers", "url": "https://vrbangers.com/video/and-the-award/?x=1"}},
                {"video": {"host": "MEGA", "slug": "mega-slug"}}]},
            {"id": 3}
        ]});
        let c = Covered::of(&waiting);
        assert_eq!(c.slr, HashSet::from(["66249".into(), "51207".into()]));
        assert_eq!(
            c.vrb,
            HashSet::from(["full-body-massage".into(), "and-the-award".into()])
        );
        assert!(c.covers(Source::Slr, "66249", &Value::Null));
        assert!(!c.covers(Source::Slr, "1", &Value::Null));
        let vrb = |url: &str| json!({"url": url});
        assert!(c.covers(
            Source::Vrb,
            "x",
            &vrb("https://vrbangers.com/video/full-body-massage/")
        ));
        assert!(c.covers(
            Source::Vrb,
            "x",
            &vrb("https://vrbangers.com/video/and-the-award")
        ));
        assert!(!c.covers(
            Source::Vrb,
            "x",
            &vrb("https://vrbangers.com/video/other-one/")
        ));
        assert!(!c.covers(Source::Vrb, "x", &json!({})));
        // A VRBangers slug never covers another source's scene; nor a MEGA video's slug.
        assert!(!c.covers(
            Source::Pmvhaven,
            "x",
            &vrb("https://pmvhaven.com/video/full-body-massage")
        ));
        assert!(!c.vrb.contains("mega-slug"));
        assert_eq!(Covered::of(&Value::Null), Covered::default());
    }

    #[test]
    fn url_slugs() {
        assert_eq!(
            url_slug("https://vrbangers.com/video/wet-and-wild-in-malibu/").as_deref(),
            Some("wet-and-wild-in-malibu")
        );
        assert_eq!(url_slug("https://x.com/a/b?c=/d#e").as_deref(), Some("b"));
        assert_eq!(url_slug("https://x.com/"), None);
        assert_eq!(url_slug("https://x.com"), None);
    }

    #[test]
    fn done_scenes() {
        assert!(is_done(&json!({"fetchedAt": "2026-11-17T00:00:00Z"})));
        assert!(is_done(&json!({"inLibrary": true})));
        assert!(!is_done(&json!({"fetchedAt": null, "inLibrary": false})));
        assert!(!is_done(&json!({"fetchedAt": ""})));
    }

    #[test]
    fn a_contract_scene_as_a_card() {
        let s = json!({
            "id": "6accfc2cff316f0c39086f52", "site": "vrconk",
            "url": "https://vrconk.com/video/wet-and-wild/", "title": "Wet & Wild",
            "studio": "VRConk", "actors": ["Angel Youngs"], "date": "2026-10-14", "durationS": 4041,
            "thumbnail": "https://c/t.jpg",
            "images": ["https://c/1.jpg", "http://c/2.jpg", "https://c/t.jpg", "https://c/3.jpg",
                       "https://c/4.jpg", "https://c/5.jpg", "https://c/6.jpg"],
            "vr": true, "fps": 60, "passthrough": true, "creampie": true, "pmv": false,
            "compilation": false, "likes": 3233, "tags": ["asian", "creampie"],
            "tagScores": {"creampie": 2}, "favourite": "Angel Youngs",
            "script": {"url": "https://mov/x.funscript", "axes": ["twist"]}, "handmade": [],
            "inLibrary": false, "fetchedAt": null, "score": 7.5,
            "scoreParts": {"vr": 3, "tag:creampie": 2}
        });
        let c =
            serde_json::to_value(card(Source::Vrb, "6accfc2cff316f0c39086f52".into(), &s)).unwrap();
        assert_eq!(
            c,
            json!({
                "source": "vrb", "site": "vrconk", "id": "6accfc2cff316f0c39086f52",
                "url": "https://vrconk.com/video/wet-and-wild/", "title": "Wet & Wild",
                "author": "VRConk", "actors": ["Angel Youngs"],
                "images": ["https://c/t.jpg", "https://c/1.jpg", "https://c/3.jpg", "https://c/4.jpg", "https://c/5.jpg"],
                "score": 7.5, "scoreParts": {"vr": 3, "tag:creampie": 2}, "vr": true, "creampie": true,
                "runtimeMs": 4_041_000, "date": "2026-10-14", "fps": 60.0, "passthrough": true,
                "pmv": false, "compilation": false, "likes": 3233,
                "tags": [{"tag": "creampie", "score": 2.0}, {"tag": "asian", "score": 0.0}],
                "topTags": [{"tag": "creampie", "score": 2.0}], "favourite": "Angel Youngs",
                "script": true, "axes": ["twist"], "handmade": [], "scriptSource": null,
                "aiScript": false, "projection": null, "quality": null
            })
        );
        // No script, no https page: neither shows.
        let c = card(
            Source::Pmvhaven,
            "p1".into(),
            &json!({"url": "http://x", "script": null}),
        );
        assert_eq!(
            (c.url, c.script, c.site.as_str()),
            (None, false, "pmvhaven")
        );
    }

    #[test]
    fn faptap_candidates_map_to_cards() {
        // The JS scout's toCandidate shape (+ quality, thumbnail).
        let cand = json!({
            "id": "1957860091331874817", "title": "Pov ride", "url": "https://faptap.net/v/1957860091331874817",
            "author": "scripter", "tags": ["pov", "riding"], "tagScores": {"riding": 2}, "likes": 41,
            "duration": 312.4, "vr": false, "projection": "MONO_FLAT", "music": false, "focus": ["pov"],
            "artistInLibrary": null, "live": false, "favourite": null, "picked": false,
            "score": 4.5, "quality": 1080, "thumbnail": "https://faptap.net/api/assets/t.jpg"
        });
        let c = card(Source::Faptap, "1957860091331874817".into(), &cand);
        assert_eq!(c.source, "faptap");
        assert_eq!(c.site, "faptap");
        assert_eq!(
            c.url.as_deref(),
            Some("https://faptap.net/v/1957860091331874817")
        );
        assert_eq!(c.author.as_deref(), Some("scripter"));
        assert_eq!(c.runtime_ms, Some(312_400));
        assert_eq!(
            (c.likes, c.score, c.quality),
            (Some(41), Some(4.5), Some(1080))
        );
        assert_eq!(c.projection.as_deref(), Some("MONO_FLAT"));
        assert_eq!(c.images, ["https://faptap.net/api/assets/t.jpg"]);
        assert!(c.script && c.axes.is_empty() && !c.vr);
        assert_eq!(c.top_tags[0].tag, "riding");
        assert_eq!(stack_of(Source::Faptap, &cand), Stack::Flat);
        // Older lists: thumbnailUrl; no url → the id's page.
        let c = card(
            Source::Faptap,
            "7".into(),
            &json!({"thumbnailUrl": "https://faptap.net/t.jpg", "vr": true}),
        );
        assert_eq!(c.images, ["https://faptap.net/t.jpg"]);
        assert_eq!(c.url.as_deref(), Some("https://faptap.net/v/7"));
        // The file's list is `candidates`, ids strings or numbers.
        let file = json!({"candidates": [cand, {"id": 12, "title": "n"}, {"id": "a:b"}, {"title": "no id"}],
                          "scenes": [{"id": "not-read"}]});
        let ids: Vec<String> = scenes(Source::Faptap, &file).map(|(id, _)| id).collect();
        assert_eq!(ids, ["1957860091331874817", "12"]);
        assert!(is_scene(Source::Faptap, &file, "12"));
        assert!(!is_scene(Source::Faptap, &file, "not-read"));
        assert!(!is_scene(Source::Vrb, &file, "12"));
    }

    #[test]
    fn slr_scenes_keep_their_fields() {
        let s = json!({"id": 66249, "label": "he-woke-me-up-66249", "studio": "Miss Lexa VR",
            "favorites": 2016, "scriptSource": "slr-ai", "aiScript": true, "tags": ["milf", "", "milf", "pov"],
            "handmade": [{"source": "eroscripts", "topicId": 282260}], "vr": true});
        let c = card(Source::Slr, "66249".into(), &s);
        assert_eq!(
            c.url.as_deref(),
            Some("https://www.sexlikereal.com/scenes/he-woke-me-up-66249")
        );
        assert_eq!((c.likes, c.script, c.ai_script), (Some(2016), true, true));
        assert_eq!(c.script_source.as_deref(), Some("slr-ai"));
        assert_eq!(c.handmade[0]["topicId"], 282260);
        // No tagScores: unscored, in the file's order, deduped; nothing on top.
        assert_eq!(
            serde_json::to_value(&c.tags).unwrap(),
            json!([{"tag": "milf", "score": null}, {"tag": "pov", "score": null}])
        );
        assert!(c.top_tags.is_empty());
        let c = card(Source::Slr, "5".into(), &json!({"url": "http://x/1"}));
        assert_eq!(
            c.url.as_deref(),
            Some("https://www.sexlikereal.com/scenes/5")
        );
        assert!(!c.script);
    }
}
