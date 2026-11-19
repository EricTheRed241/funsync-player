//! The Creators site's creators in the swipe deck. `creators.json` (creators:scout) lists
//! each creator's EroScripts posts and FapTap videos; `creator-swipes.json` holds the
//! Creators site's yes / no per creator and the swipes of their posts.
//!
//! A creator is yes when swiped yes, or, without a swipe, when she is a favourite artist or
//! on the pornstar list (`sources`); swiped no wins (the same rule as creator-report's
//! `deck::is_yes`). Every deck card that is a yes creator's post (a waiting EroScripts post,
//! a FapTap scene) gets her name (the Creator badge) and `swipeDeck.yesCreatorBonus` added
//! to its score. Her downloadable posts that no other card shows (not on the waiting list,
//! not in faptap-scout.json, not in the library) become creator cards (source `creators`,
//! id `<kind>-<id>`), scored by creators:scout plus the bonus. Their swipes go to the `posts`
//! of creator-swipes.json (`<kind>:<id>`: right = want, left = hide, with the creator's
//! slug), which `creators:scout --fetch` downloads; the Creators site writes only its
//! `creators`, and both write the file whole (read, change one entry, write atomically).

use crate::creator_pictures;
use crate::deck::{self, Stack};
use crate::report::{self, str_of, strings_of};
use crate::scenes::{SceneCard, SceneTag};
use crate::swipes::{self, Dir};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub const FILE: &str = "creators.json";
pub const SWIPES_FILE: &str = "creator-swipes.json";
pub const SWIPES_BACKUP: &str = "creator-swipes.json.bak";
pub const SWIPES_VERSION: u64 = 1;
/// The source of a creator card: `/api/scenes/creators/swipe`, `exclude` key `creators:<id>`.
pub const SOURCE: &str = "creators";
/// The post kinds creators.json lists, which are also their card ids' prefixes.
const KINDS: [&str; 2] = ["eroscripts", "faptap"];

const SHARED: &str = include_str!("../../../renderer/js/content-preferences.json");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Shared {
    swipe_deck: SwipeDeck,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwipeDeck {
    yes_creator_bonus: f64,
}

/// `swipeDeck.yesCreatorBonus` from the shared preferences.
pub fn yes_creator_bonus() -> Result<f64, String> {
    let shared: Shared = serde_json::from_str(SHARED)
        .map_err(|e| format!("content-preferences.json: swipeDeck.yesCreatorBonus: {e}"))?;
    Ok(shared.swipe_deck.yes_creator_bonus)
}

/// A yes creator, by slug, with the name her cards show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Who {
    pub slug: String,
    pub name: String,
}

/// creators.json and creator-swipes.json as the deck reads them (missing files: nobody).
#[derive(Debug, Default)]
pub struct Creators {
    file: Value,
    swipes: Value,
    /// `<kind>:<id>` → the yes creator whose post it is (the first by slug).
    owners: HashMap<String, Who>,
    /// Score points a yes creator's card gets.
    pub bonus: f64,
}

/// A post's key in creator-swipes.json, `<kind>:<id>`; ids may be numbers in creators.json.
fn post_key_of(p: &Value) -> Option<String> {
    let kind = p.get("kind").and_then(Value::as_str)?;
    let id = match p.get("id")? {
        Value::String(s) if !s.is_empty() => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    KINDS.contains(&kind).then(|| format!("{kind}:{id}"))
}

/// A creator card's id (`<kind>-<id>`) as its post key (`<kind>:<id>`).
pub fn post_key(card_id: &str) -> Option<String> {
    let (kind, id) = card_id.split_once('-')?;
    (KINDS.contains(&kind) && !id.is_empty()).then(|| format!("{kind}:{id}"))
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

impl Creators {
    /// Read both files and the bonus; problems go to `problems`. A swipes file of another
    /// version is a problem and read as no swipes.
    pub fn read(user_data: &Path, problems: &mut Vec<String>) -> Creators {
        let file = report::read_json(user_data, FILE, problems);
        let mut swipes = report::read_json(user_data, SWIPES_FILE, problems);
        if !swipes.is_null()
            && swipes.get("version").and_then(Value::as_u64) != Some(SWIPES_VERSION)
        {
            problems.push(format!("{SWIPES_FILE}: not version {SWIPES_VERSION}"));
            swipes = Value::Null;
        }
        let bonus = yes_creator_bonus().unwrap_or_else(|e| {
            problems.push(e);
            0.0
        });
        Creators::new(file, swipes, bonus)
    }

    pub fn new(file: Value, swipes: Value, bonus: f64) -> Creators {
        let mut c = Creators {
            file,
            swipes,
            owners: HashMap::new(),
            bonus,
        };
        let mut owners = HashMap::new();
        for (who, cr) in c.yes() {
            for key in posts(cr).filter_map(post_key_of) {
                owners.entry(key).or_insert_with(|| who.clone());
            }
        }
        c.owners = owners;
        c
    }

    /// The yes creators, by slug.
    fn yes(&self) -> impl Iterator<Item = (Who, &Value)> {
        self.file
            .get("creators")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter(|(slug, c)| self.is_yes(slug, c))
            .map(|(slug, c)| {
                let name = str_of(c, "name").filter(|n| !n.is_empty());
                let who = Who {
                    slug: slug.clone(),
                    name: name.unwrap_or_else(|| slug.clone()),
                };
                (who, c)
            })
    }

    fn is_yes(&self, slug: &str, c: &Value) -> bool {
        match self.swipes.pointer(&format!(
            "/creators/{}",
            slug.replace('~', "~0").replace('/', "~1")
        )) {
            Some(s) => s.get("choice").and_then(Value::as_str) == Some("yes"),
            None => strings_of(c, "sources")
                .unwrap_or_default()
                .iter()
                .any(|s| s == "favourite" || s == "pornstar"),
        }
    }

    /// The yes creator whose post `key` (`<kind>:<id>`) is.
    pub fn owner(&self, key: &str) -> Option<&Who> {
        self.owners.get(key)
    }

    fn post_choice(&self, key: &str) -> Option<&str> {
        self.swipes
            .get("posts")
            .and_then(|p| p.get(key))
            .and_then(|s| s.get("choice"))
            .and_then(Value::as_str)
    }

    /// Mark a deck card that is a yes creator's post: her name, and the bonus on its score.
    pub fn tag(&self, card: &mut deck::DeckCard) {
        let key = match card {
            deck::DeckCard::Post(p) => format!("eroscripts:{}", p.id),
            deck::DeckCard::Scene(s) if s.source == "faptap" => format!("faptap:{}", s.id),
            deck::DeckCard::Scene(_) => return,
        };
        let Some(who) = self.owners.get(&key) else {
            return;
        };
        match card {
            deck::DeckCard::Post(p) => {
                p.creator = Some(who.name.clone());
                p.score = p.score.map(|s| s + self.bonus);
            }
            deck::DeckCard::Scene(s) => {
                s.creator = Some(who.name.clone());
                s.score = s.score.map(|v| v + self.bonus);
                if !s.score_parts.is_object() {
                    s.score_parts = json!({});
                }
                s.score_parts["yes creator"] = json!(self.bonus);
            }
        }
    }

    /// The creator cards of `stack`: the yes creators' downloadable posts not in the library
    /// (`in_library`: the library's videos per topic id), not `covered` by another card
    /// (`<kind>:<id>` of every waiting post and FapTap scene) and not swiped; and how many of
    /// those posts of `stack` are swiped want.
    pub fn cards(
        &self,
        stack: Stack,
        covered: &HashSet<String>,
        in_library: &HashMap<u64, usize>,
    ) -> (Vec<SceneCard>, usize) {
        let mut seen = HashSet::new();
        let mut cards = Vec::new();
        let mut wanted = 0;
        for (who, c) in self.yes() {
            for p in posts(c) {
                let Some(key) = post_key_of(p) else { continue };
                let (kind, id) = key.split_once(':').unwrap_or_default();
                let card_id = format!("{kind}-{id}");
                let vr = strings_of(p, "tags")
                    .unwrap_or_default()
                    .iter()
                    .any(|t| t.eq_ignore_ascii_case("vr"));
                if !flag(p, "downloadable")
                    || flag(p, "inLibrary")
                    || !swipes::is_id(&card_id)
                    || covered.contains(&key)
                    || (kind == "eroscripts"
                        && id
                            .parse()
                            .is_ok_and(|n: u64| in_library.get(&n).is_some_and(|&v| v > 0)))
                    || Stack::of(flag(p, "multiAxis"), vr) != stack
                    || !seen.insert(key.clone())
                {
                    continue;
                }
                match self.post_choice(&key) {
                    Some("want") => wanted += 1,
                    Some(_) => {}
                    None => cards.push(self.card(&who, c, p, kind, card_id, vr)),
                }
            }
        }
        (cards, wanted)
    }

    /// A creator card for post `p` (of `kind`) of creator `c`: the post's picture, then hers.
    fn card(&self, who: &Who, c: &Value, p: &Value, kind: &str, id: String, vr: bool) -> SceneCard {
        let slug = who.slug.as_str();
        let post_score = p.get("score").and_then(Value::as_f64);
        let mut images: Vec<String> = str_of(p, "image")
            .and_then(|f| creator_pictures::url(slug, &f))
            .into_iter()
            .collect();
        // Her pictures, Instagram's first (as on the Creators site).
        let pics: Vec<&Value> = c
            .get("pictures")
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        let (ig, rest): (Vec<&Value>, Vec<&Value>) = pics
            .into_iter()
            .partition(|x| x.get("source").and_then(Value::as_str) == Some("instagram"));
        for pic in ig.into_iter().chain(rest) {
            if images.len() == deck::MAX_IMAGES {
                break;
            }
            if let Some(url) = str_of(pic, "file").and_then(|f| creator_pictures::url(slug, &f))
                && !images.contains(&url)
            {
                images.push(url);
            }
        }
        SceneCard {
            source: SOURCE,
            site: kind.to_string(),
            id,
            url: str_of(p, "url").filter(|u| u.starts_with("https://")),
            title: str_of(p, "title"),
            author: Some(who.name.clone()),
            images,
            score: post_score.map(|s| s + self.bonus),
            score_parts: match post_score {
                Some(s) => json!({"post": s, "yes creator": self.bonus}),
                None => Value::Null,
            },
            vr,
            creampie: flag(p, "creampie"),
            runtime_ms: p
                .get("durationS")
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && *n > 0.0)
                .map(|n| (n * 1000.0).round() as u64),
            date: str_of(p, "createdAt"),
            likes: p.get("likes").and_then(Value::as_i64),
            tags: strings_of(p, "tags")
                .unwrap_or_default()
                .into_iter()
                .filter(|t| !t.is_empty())
                .map(|tag| SceneTag { tag, score: None })
                .collect(),
            script: true,
            creator: Some(who.name.clone()),
            multi_axis: flag(p, "multiAxis"),
            ..SceneCard::default()
        }
    }
}

fn posts(c: &Value) -> impl Iterator<Item = &Value> {
    c.get("posts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// creator-swipes.json to change: missing → empty; unreadable, unparsable or another version
/// → an error naming it (never written over).
fn read_for_write(user_data: &Path) -> Result<Value, String> {
    let bytes = match std::fs::read(user_data.join(SWIPES_FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(json!({"version": SWIPES_VERSION, "creators": {}, "posts": {}}));
        }
        Err(e) => return Err(format!("{SWIPES_FILE}: {e}")),
    };
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| format!("{SWIPES_FILE}: {e}"))?;
    if v.get("version").and_then(Value::as_u64) != Some(SWIPES_VERSION) {
        return Err(format!("{SWIPES_FILE}: not version {SWIPES_VERSION}"));
    }
    Ok(v)
}

fn write(user_data: &Path, v: &Value) -> Result<(), String> {
    video_tags::store::write_json_atomic(user_data, SWIPES_FILE, SWIPES_BACKUP, v).map(|_| ())
}

/// Record post `key` of creator `slug`: right = want, left = hide. Returns the entry's `at`.
pub fn set_post(
    user_data: &Path,
    key: &str,
    slug: &str,
    dir: Dir,
    at: String,
) -> Result<String, String> {
    let mut v = read_for_write(user_data)?;
    let choice = match dir {
        Dir::Right => "want",
        Dir::Left => "hide",
    };
    if !v.get("posts").is_some_and(Value::is_object) {
        v["posts"] = json!({});
    }
    v["posts"][key] = json!({"choice": choice, "at": at, "creator": slug});
    write(user_data, &v)?;
    Ok(at)
}

/// Forget post `key`'s swipe; whether there was one (the file is written only then).
pub fn undo_post(user_data: &Path, key: &str) -> Result<bool, String> {
    let mut v = read_for_write(user_data)?;
    let removed = v
        .get_mut("posts")
        .and_then(Value::as_object_mut)
        .is_some_and(|p| p.remove(key).is_some());
    if removed {
        write(user_data, &v)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "scout-report-creators-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn on_disk(dir: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(dir.join(SWIPES_FILE)).unwrap()).unwrap()
    }

    #[test]
    fn yes_is_a_yes_swipe_else_a_favourite_or_pornstar_and_no_wins() {
        let c = Creators::new(
            json!({"creators": {
                "a": {"name": "A", "posts": [{"kind": "eroscripts", "id": 1}]},
                "b": {"sources": ["favourite"], "posts": [{"kind": "faptap", "id": "9"}, {"kind": "eroscripts", "id": 1}]},
                "c": {"name": "C", "sources": ["pornstar"], "posts": [{"kind": "eroscripts", "id": 2}]},
                "d": {"name": "D", "sources": ["swiped"], "posts": [{"kind": "eroscripts", "id": 3}]}
            }}),
            json!({"version": 1, "creators": {"a": {"choice": "yes"}, "c": {"choice": "no"}}}),
            4.0,
        );
        let who = |k: &str| c.owner(k).map(|w| (w.slug.as_str(), w.name.as_str()));
        // Two yes creators list topic 1: the first by slug owns it; b has no name, so her slug.
        assert_eq!(who("eroscripts:1"), Some(("a", "A")));
        assert_eq!(who("faptap:9"), Some(("b", "b")));
        assert_eq!(who("eroscripts:2"), None);
        assert_eq!(who("eroscripts:3"), None);
    }

    #[test]
    fn card_ids_map_to_post_keys() {
        assert_eq!(
            post_key("eroscripts-41415").as_deref(),
            Some("eroscripts:41415")
        );
        assert_eq!(
            post_key("faptap-2122637633308856321").as_deref(),
            Some("faptap:2122637633308856321")
        );
        for bad in ["slr-1", "eroscripts-", "eroscripts", "-1"] {
            assert_eq!(post_key(bad), None, "{bad}");
        }
    }

    #[test]
    fn post_swipes_keep_the_creator_sites_entries() {
        let dir = temp("swipes");
        // No file yet: a new one in the Creators site's shape.
        set_post(&dir, "faptap:7", "bea", Dir::Left, "t0".into()).unwrap();
        assert_eq!(
            on_disk(&dir),
            json!({"version": 1, "creators": {}, "posts": {"faptap:7": {"choice": "hide", "at": "t0", "creator": "bea"}}})
        );
        let theirs =
            json!({"version": 1, "creators": {"ann": {"choice": "yes", "at": "t"}}, "posts": {}});
        std::fs::write(dir.join(SWIPES_FILE), theirs.to_string()).unwrap();
        set_post(&dir, "eroscripts:1", "ann", Dir::Right, "t1".into()).unwrap();
        let v = on_disk(&dir);
        assert_eq!(v["creators"], theirs["creators"]);
        assert_eq!(
            v["posts"]["eroscripts:1"],
            json!({"choice": "want", "at": "t1", "creator": "ann"})
        );
        assert!(dir.join(SWIPES_BACKUP).exists());
        assert!(undo_post(&dir, "eroscripts:1").unwrap());
        assert!(!undo_post(&dir, "eroscripts:1").unwrap());
        assert_eq!(on_disk(&dir)["posts"], json!({}));
        // Another version is never written over.
        std::fs::write(dir.join(SWIPES_FILE), r#"{"version": 2}"#).unwrap();
        assert!(set_post(&dir, "eroscripts:1", "ann", Dir::Right, "t2".into()).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join(SWIPES_FILE)).unwrap(),
            r#"{"version": 2}"#
        );
        let mut problems = Vec::new();
        Creators::read(&dir, &mut problems);
        assert_eq!(problems, [format!("{SWIPES_FILE}: not version 1")]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
