//! SLR's public catalogue API: find a performer, list her scenes, read each
//! scene's details, and turn them into [`Scene`]s (VR, creampie, PMV,
//! compilation, hand-crafted scripts). Scores are added by `score`.

use crate::http::{self, Http, NetError, Pacer, Request};
use crate::prefs::SlrScout;
use crate::store::{Actor, Handmade, Scene};
use crate::text;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::time::{Duration, UNIX_EPOCH};

pub const API: &str = "https://api.sexlikereal.com";
pub const SITE: &str = "https://www.sexlikereal.com";
const PER_PAGE: u32 = 36;

/// A GET to the public API with the headers its web client sends (the
/// client adds a browser User-Agent).
pub fn api_request(url: String) -> Request {
    Request::get(url)
        .header("Client-Type", "web")
        .header("project", "1")
        .header("Accept", "application/json")
}

/// A GET through the pacer (rate limits respected, see [`http::paced_json`]).
fn get(http: &dyn Http, pacer: &mut Pacer, url: String) -> Result<Value, NetError> {
    http::paced_json(http, pacer, &api_request(url))
}

/// A performer to look up: the name and other spellings (`aliases` of
/// `pornstarTags.pornstars`).
#[derive(Debug, Clone, PartialEq)]
pub struct Wanted {
    pub name: String,
    pub aliases: Vec<String>,
}

/// The performer SLR knows by `w.name` or one of its aliases (exact match,
/// any case or punctuation): a search by the name, then by each alias until
/// one matches. `None` when SLR knows none of them.
pub fn find_actor(
    http: &dyn Http,
    pacer: &mut Pacer,
    w: &Wanted,
) -> Result<Option<Actor>, NetError> {
    let spellings: Vec<String> = std::iter::once(&w.name)
        .chain(&w.aliases)
        .map(|n| text::normalise(n))
        .filter(|n| !n.is_empty())
        .collect();
    for query in std::iter::once(&w.name).chain(&w.aliases) {
        let v = get(
            http,
            pacer,
            format!(
                "{API}/v3/search?query={}&tab=actors",
                text::url_encode(query)
            ),
        )?;
        let found = v["data"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|a| {
                a["name"]
                    .as_str()
                    .is_some_and(|n| spellings.contains(&text::normalise(n)))
            })
            .and_then(|a| {
                Some(Actor {
                    name: a["name"].as_str()?.to_string(),
                    id: a["id"].as_u64()?,
                    label: a["label"].as_str().unwrap_or_default().to_string(),
                    scenes: a["quantities"]["scenes"].as_u64().unwrap_or(0),
                })
            });
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}

/// A scene from a listing page.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    pub id: u64,
    pub label: String,
    /// `isFlatScreenType`, when the listing says.
    pub flat: Option<bool>,
}

/// Every scene of the performer, page by page.
pub fn list_scenes(
    http: &dyn Http,
    pacer: &mut Pacer,
    actor_id: u64,
) -> Result<Vec<Listed>, NetError> {
    let mut out = Vec::new();
    let mut page: u64 = 1;
    loop {
        let v = get(
            http,
            pacer,
            format!("{API}/v3/scenes?models={actor_id}&perPage={PER_PAGE}&page={page}"),
        )?;
        let data = v["data"].as_array().map(Vec::as_slice).unwrap_or_default();
        out.extend(data.iter().filter_map(|s| {
            Some(Listed {
                id: s["id"].as_u64()?,
                label: s["label"].as_str()?.to_string(),
                flat: s["isFlatScreenType"].as_bool(),
            })
        }));
        let pages = v["meta"]["pagination"]["totalPages"].as_u64().unwrap_or(0);
        if data.is_empty() || page >= pages {
            return Ok(out);
        }
        page += 1;
    }
}

/// One scene's details (`data` of `/v3/scenes/<label>`).
pub fn detail(http: &dyn Http, pacer: &mut Pacer, label: &str) -> Result<Value, NetError> {
    let v = get(http, pacer, format!("{API}/v3/scenes/{label}"))?;
    match v.get("data") {
        Some(d) if d.is_object() => Ok(d.clone()),
        _ => Err(NetError::Failed(format!("{label}: no scene data"))),
    }
}

fn names(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x["name"].as_str().map(str::to_string))
        .collect()
}

/// A category named "Creampie", or a timestamp whose name contains
/// "creampie" (any case).
pub fn is_creampie(categories: &[String], timestamps: &[String]) -> bool {
    categories
        .iter()
        .any(|c| c.eq_ignore_ascii_case("creampie"))
        || timestamps
            .iter()
            .any(|t| t.to_ascii_lowercase().contains("creampie"))
}

/// A category "PMV", or PMV as a title word.
pub fn is_pmv(categories: &[String], title: &str) -> bool {
    categories.iter().any(|c| c.eq_ignore_ascii_case("pmv"))
        || text::words(&text::normalise(title)).any(|w| w == "pmv" || w == "pmvs")
}

/// A category "Compilation", or "compilation" / "best of" in the title.
pub fn is_compilation(categories: &[String], title: &str) -> bool {
    let title = text::normalise(title);
    categories
        .iter()
        .any(|c| c.eq_ignore_ascii_case("compilation"))
        || text::words(&title).any(|w| w == "compilation" || w == "compilations")
        || text::contains_words(&title, "best of")
}

/// Studio-made passthrough (chroma key or alpha). SLR's automatic AI alpha,
/// offered on most scenes, doesn't count.
fn passthrough(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Object(_) => {
            v["chromaKey"]["enabled"].as_bool() == Some(true)
                || v["alpha"]["enabled"].as_bool() == Some(true)
        }
        _ => false,
    }
}

/// `YYYY-MM-DD` of an epoch-seconds date.
pub fn day(epoch_s: i64) -> Option<String> {
    let secs = u64::try_from(epoch_s).ok()?;
    let iso = video_tags::timefmt::iso_utc(UNIX_EPOCH + Duration::from_secs(secs));
    Some(iso[..10].to_string())
}

/// A scene from its details; `None` without an id or label. Not scored.
pub fn scene(d: &Value) -> Option<Scene> {
    let id = d["id"].as_u64()?;
    let label = d["label"].as_str()?.to_string();
    let title = d["title"].as_str().unwrap_or_default().to_string();
    let categories = names(&d["categories"]);
    let timestamps = names(&d["timestamps"]);
    let scripts = d["scripts"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut handmade: Vec<Handmade> = Vec::new();
    for s in scripts
        .iter()
        .filter(|s| s["scriptAI"].as_bool() == Some(false))
    {
        let author = s["author"].as_str().unwrap_or_default().to_string();
        let h = Handmade::Slr { author };
        if !handmade.contains(&h) {
            handmade.push(h);
        }
    }
    let creampie = is_creampie(&categories, &timestamps);
    Some(Scene {
        url: format!("{SITE}/scenes/{label}"),
        id,
        label,
        studio: d["studio"]["name"].as_str().map(str::to_string),
        actors: names(&d["actors"]),
        date: d["date"].as_i64().and_then(day),
        duration_s: d["fullVideoLength"].as_u64(),
        thumbnail: d["thumbnailUrl"].as_str().map(str::to_string),
        vr: d["isFlatScreenType"].as_bool() != Some(true),
        fps: d["fps"].as_number().cloned(),
        passthrough: passthrough(&d["passthrough"]),
        creampie,
        ai_script: scripts
            .iter()
            .any(|s| s["scriptAI"].as_bool() == Some(true)),
        pmv: is_pmv(&categories, &title),
        compilation: is_compilation(&categories, &title),
        favorites: d["quantity"]["favorites"].as_u64().unwrap_or(0),
        tags: crate::score::scene_tags(&categories, creampie),
        tag_scores: BTreeMap::new(),
        favourite: None,
        fetched_at: None,
        script_source: None,
        handmade,
        title,
        score: 0.0,
        score_parts: BTreeMap::new(),
    })
}

/// Whether the preferences keep a scene.
pub fn wanted(prefs: &SlrScout, s: &Scene) -> bool {
    (!prefs.vr_only || s.vr) && (!prefs.require_creampie || s.creampie)
}

/// The performers and their wanted scenes (each scene once).
#[derive(Debug)]
pub struct Catalogue {
    pub actors: Vec<Actor>,
    /// Wanted performers SLR doesn't know.
    pub unknown: Vec<String>,
    pub scenes: Vec<Scene>,
}

/// Walk the catalogue: each performer's scene list (performers SLR doesn't
/// know are skipped), then the details of every scene the listing doesn't
/// already rule out, each scene once. A scene whose details can't be read
/// is skipped with a warning; a rate limit stops the walk. Progress goes to
/// `log`.
pub fn walk(
    http: &dyn Http,
    pacer: &mut Pacer,
    wanted_performers: &[Wanted],
    prefs: &SlrScout,
    log: &mut dyn Write,
) -> Result<Catalogue, NetError> {
    let mut actors: Vec<Actor> = Vec::new();
    let mut unknown = Vec::new();
    let mut listed: Vec<Listed> = Vec::new();
    let mut seen = HashSet::new();
    for (i, w) in wanted_performers.iter().enumerate() {
        let Some(actor) = find_actor(http, pacer, w)? else {
            let _ = writeln!(
                log,
                "[{}/{}] {}: not on SLR",
                i + 1,
                wanted_performers.len(),
                w.name
            );
            unknown.push(w.name.clone());
            continue;
        };
        if actors.iter().any(|a| a.id == actor.id) {
            continue;
        }
        let scenes = list_scenes(http, pacer, actor.id)?;
        let fresh = scenes.iter().filter(|s| !seen.contains(&s.id)).count();
        let _ = writeln!(
            log,
            "[{}/{}] {}: {} scene(s) listed, {fresh} new (SLR id {})",
            i + 1,
            wanted_performers.len(),
            actor.name,
            scenes.len(),
            actor.id
        );
        for s in scenes {
            if seen.insert(s.id) {
                listed.push(s);
            }
        }
        actors.push(actor);
    }
    let to_read: Vec<&Listed> = listed
        .iter()
        .filter(|l| !(prefs.vr_only && l.flat == Some(true)))
        .collect();
    let _ = writeln!(
        log,
        "Reading {} scene(s){}...",
        to_read.len(),
        if prefs.vr_only { " (VR)" } else { "" }
    );
    let mut scenes = Vec::new();
    for (i, l) in to_read.iter().enumerate() {
        match detail(http, pacer, &l.label) {
            Ok(d) => {
                if let Some(s) = scene(&d).filter(|s| wanted(prefs, s)) {
                    scenes.push(s);
                }
            }
            Err(NetError::Failed(m)) => {
                let _ = writeln!(log, "  skipped {}: {m}", l.label);
            }
            Err(e) => return Err(e),
        }
        if (i + 1) % 50 == 0 {
            let _ = writeln!(log, "  {}/{}", i + 1, to_read.len());
        }
    }
    Ok(Catalogue {
        actors,
        unknown,
        scenes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::testing::Canned;
    use serde_json::json;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn prefs(vr_only: bool, require_creampie: bool) -> SlrScout {
        let mut p = crate::prefs::load().unwrap().slr_scout;
        p.vr_only = vr_only;
        p.require_creampie = require_creampie;
        p
    }

    fn wanted_list(list: &[(&str, &[&str])]) -> Vec<Wanted> {
        list.iter()
            .map(|(name, aliases)| Wanted {
                name: name.to_string(),
                aliases: strings(aliases),
            })
            .collect()
    }

    #[test]
    fn detects_creampie_by_category_or_timestamp() {
        assert!(is_creampie(&strings(&["3D", "Creampie"]), &[]));
        assert!(is_creampie(&[], &strings(&["Doggy Style", "Creampie"])));
        assert!(is_creampie(&[], &strings(&["Anal creampie finish"])));
        assert!(!is_creampie(
            &strings(&["Cumshot"]),
            &strings(&["Cum on tits"])
        ));
        assert!(!is_creampie(&strings(&["Creampie compilation"]), &[]));
    }

    #[test]
    fn detects_pmv_and_compilations() {
        assert!(is_pmv(&strings(&["PMV"]), "x"));
        assert!(is_pmv(&[], "Miss Lexa PMV - Hypno"));
        assert!(!is_pmv(&[], "PMVision"));
        assert!(is_compilation(
            &strings(&["Compilation"]),
            "25 Top Milf Babes"
        ));
        assert!(is_compilation(&[], "Creampie Compilation Vol. 2"));
        assert!(is_compilation(&[], "The Best of Miss Lexa"));
        assert!(!is_compilation(&[], "Best Ofelia"));
        assert!(!is_compilation(&[], "Please Creampie Me"));
    }

    fn detail_json() -> Value {
        json!({
            "id": 85666, "label": "latina-milf-oiled-up-creampied-85666",
            "title": "Latina Milf oiled up & creampied", "isFlatScreenType": false,
            "date": 1777071600, "fullVideoLength": 1629, "fps": 60,
            "passthrough": {"chromaKey": {"enabled": false}, "alpha": {"enabled": false}, "aiAlpha": {"enabled": true}},
            "thumbnailUrl": "https://cdn-vr.sexlikereal.com/x.webp",
            "quantity": {"favorites": 1123}, "studio": {"name": "Miss Lexa VR"},
            "actors": [{"name": "Miss Lexa"}],
            "scripts": [{"id": 1, "scriptAI": true, "author": "scriptai"},
                        {"id": 2, "scriptAI": false, "author": "kimera"},
                        {"id": 3, "scriptAI": false, "author": "kimera"}],
            "timestamps": [{"name": "Teasing"}, {"name": "Creampie"}],
            "categories": [{"name": "3D"}, {"name": "Big tits"}]
        })
    }

    #[test]
    fn builds_scenes_from_details() {
        let s = scene(&detail_json()).unwrap();
        assert_eq!(s.id, 85666);
        assert_eq!(
            s.url,
            "https://www.sexlikereal.com/scenes/latina-milf-oiled-up-creampied-85666"
        );
        assert_eq!(s.date.as_deref(), Some("2026-04-24"));
        assert_eq!(s.duration_s, Some(1629));
        assert_eq!(s.studio.as_deref(), Some("Miss Lexa VR"));
        assert_eq!(s.actors, ["Miss Lexa"]);
        assert!(s.vr && s.creampie && s.ai_script && !s.passthrough && !s.pmv && !s.compilation);
        assert_eq!(s.fps, Some(60.into()));
        assert_eq!(s.favorites, 1123);
        assert_eq!(
            s.handmade,
            [Handmade::Slr {
                author: "kimera".into()
            }]
        );

        let mut d = detail_json();
        d["isFlatScreenType"] = json!(true);
        d["passthrough"] = json!({"chromaKey": {"enabled": true}});
        d["categories"] = Value::Null;
        d["scripts"] = Value::Null;
        d["timestamps"] = json!([]);
        let s = scene(&d).unwrap();
        assert!(!s.vr && s.passthrough && !s.creampie && !s.ai_script);
        assert!(s.handmade.is_empty());
        assert!(scene(&json!({"title": "no id"})).is_none());
    }

    #[test]
    fn filters_by_vr_and_creampie() {
        let mut s = scene(&detail_json()).unwrap();
        assert!(wanted(&prefs(true, true), &s));
        s.creampie = false;
        assert!(!wanted(&prefs(true, true), &s));
        assert!(wanted(&prefs(true, false), &s));
        s.vr = false;
        assert!(!wanted(&prefs(true, false), &s));
        assert!(wanted(&prefs(false, false), &s));
    }

    #[test]
    fn walks_the_catalogue() {
        let mut http = Canned::default();
        http.on(
            "GET",
            &format!("{API}/v3/search?query=miss%20lexa&tab=actors"),
            200,
            r#"{"data": [{"id": 1, "name": "Miss Lexa Fan"},
                         {"id": 7954, "name": "Miss Lexa", "label": "miss-lexa", "quantities": {"scenes": 3}}]}"#,
        );
        http.on(
            "GET",
            &format!("{API}/v3/scenes?models=7954&perPage=36&page=1"),
            200,
            r#"{"data": [{"id": 1, "label": "a-1", "isFlatScreenType": false},
                         {"id": 2, "label": "flat-2", "isFlatScreenType": true}],
                "meta": {"pagination": {"totalPages": 2}}}"#,
        );
        http.on(
            "GET",
            &format!("{API}/v3/scenes?models=7954&perPage=36&page=2"),
            200,
            r#"{"data": [{"id": 3, "label": "b-3", "isFlatScreenType": false}],
                "meta": {"pagination": {"totalPages": 2}}}"#,
        );
        let mut a = detail_json();
        a["id"] = json!(1);
        a["label"] = json!("a-1");
        http.on(
            "GET",
            &format!("{API}/v3/scenes/a-1"),
            200,
            &json!({ "data": a }).to_string(),
        );
        let mut b = detail_json();
        b["id"] = json!(3);
        b["label"] = json!("b-3");
        b["timestamps"] = json!([{"name": "Cowgirl"}]);
        http.on(
            "GET",
            &format!("{API}/v3/scenes/b-3"),
            200,
            &json!({ "data": b }).to_string(),
        );
        // A second performer found by her alias, sharing scene 3 and with a
        // scene whose details fail; a third SLR doesn't know.
        http.on(
            "GET",
            &format!("{API}/v3/search?query=Nobody&tab=actors"),
            200,
            r#"{"data": [{"id": 5, "name": "Nobody Else"}]}"#,
        );
        http.on(
            "GET",
            &format!("{API}/v3/search?query=Lexi&tab=actors"),
            200,
            r#"{"data": [{"id": 9, "name": "LEXI", "label": "lexi", "quantities": {"scenes": 2}}]}"#,
        );
        http.on(
            "GET",
            &format!("{API}/v3/scenes?models=9&perPage=36&page=1"),
            200,
            r#"{"data": [{"id": 3, "label": "b-3", "isFlatScreenType": false},
                         {"id": 4, "label": "gone-4", "isFlatScreenType": false}],
                "meta": {"pagination": {"totalPages": 1}}}"#,
        );
        http.on("GET", &format!("{API}/v3/scenes/gone-4"), 500, "");
        http.on(
            "GET",
            &format!("{API}/v3/search?query=Ghost&tab=actors"),
            200,
            r#"{"data": []}"#,
        );
        let mut log = Vec::new();
        let c = walk(
            &http,
            &mut Pacer::new(Duration::ZERO),
            &wanted_list(&[("miss lexa", &[]), ("Nobody", &["Lexi"]), ("Ghost", &[])]),
            &prefs(true, false),
            &mut log,
        )
        .unwrap();
        assert_eq!(c.actors.iter().map(|a| a.id).collect::<Vec<_>>(), [7954, 9]);
        assert_eq!(c.actors[0].scenes, 3);
        assert_eq!(c.unknown, ["Ghost"]);
        assert_eq!(c.scenes.iter().map(|s| s.id).collect::<Vec<_>>(), [1, 3]);
        let log = String::from_utf8(log).unwrap();
        assert!(log.contains("[2/3] LEXI: 2 scene(s) listed, 1 new"));
        assert!(log.contains("skipped gone-4"));
        assert!(log.contains("[3/3] Ghost: not on SLR"));
        {
            let seen = http.seen.borrow();
            // The flat scene's details were never read; scene 3's once.
            assert!(!seen.iter().any(|r| r.url.ends_with("flat-2")));
            assert_eq!(seen.iter().filter(|r| r.url.ends_with("/b-3")).count(), 1);
            assert!(
                seen[0]
                    .headers
                    .contains(&("Client-Type".into(), "web".into()))
            );
            assert!(seen[0].headers.contains(&("project".into(), "1".into())));
        }
        // requireCreampie keeps scene 1 only (3 has a Cowgirl timestamp).
        let c = walk(
            &http,
            &mut Pacer::new(Duration::ZERO),
            &wanted_list(&[("miss lexa", &[])]),
            &prefs(true, true),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(c.scenes.iter().map(|s| s.id).collect::<Vec<_>>(), [1]);

        http.on("GET", &format!("{API}/v3/scenes/b-3"), 429, "");
        assert!(matches!(
            walk(
                &http,
                &mut Pacer::new(Duration::ZERO),
                &wanted_list(&[("miss lexa", &[])]),
                &prefs(true, true),
                &mut Vec::new()
            ),
            Err(NetError::RateLimited(_))
        ));
    }
}
