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
//!
//! Every dealt card also names the creators in it (`people`, any creator in creators.json,
//! yes or not; at most MAX_PEOPLE): the post is hers (its creator card's, then the yes
//! creator's, then any creator's listing it), a scene's performer is her (name or alias,
//! folded), or her name runs in the title as whole words (one-word names of MIN_TITLE_WORD
//! letters or more only). Each comes with her best picture on disk (best_picture), which joins
//! the card's images; a creator without one is left out.

use crate::creator_pictures;
use crate::deck::{self, Stack};
use crate::duplicates::fold;
use crate::report::{self, str_of, strings_of};
use crate::scenes::{SceneCard, SceneTag};
use crate::swipes::{self, Dir};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::cmp::Reverse;
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
/// The Instagram profile photo creator-scout saves (its `store::INSTAGRAM_PROFILE_PICTURE`).
const INSTAGRAM_PROFILE: &str = "ig-profile.jpg";
/// Creators named per card.
pub const MAX_PEOPLE: usize = 2;
/// A one-word name counts from a card's title only with this many letters or more: "Pupu" in
/// a title can be anything, "Agapilord" can't.
const MIN_TITLE_WORD: usize = 6;
/// Shortest name (folded, words run together) matched at all.
const MIN_NAME: usize = 3;

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

/// A creator in a card (`people`): her name and her best picture (best_picture).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub name: String,
    pub slug: String,
    /// The picture's served URL, `/pictures/<slug>/<file>`.
    pub picture: String,
    /// Where it comes from: `instagram`, `profile`, `onlyfans`, `x`, ….
    pub picture_source: String,
    /// The page it comes from (https only).
    pub picture_url: Option<String>,
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
    /// Every creator, yes or not, by slug: who a card can name.
    everyone: Vec<Who>,
    /// `<kind>:<id>` → every creator listing that post (`everyone` indexes, by slug).
    listed: HashMap<String, Vec<usize>>,
    /// A name or alias, its folded words run together → each creator with it, and whether
    /// her name there is a single word.
    names: HashMap<String, Vec<(usize, bool)>>,
    /// The longest `names` key, in bytes: a title's runs of words stop there.
    longest: usize,
}

/// The words of `s` folded (crate::duplicates::fold: lower case, accents dropped): letters
/// and digits only.
fn words(s: &str) -> Vec<String> {
    fold(s)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// How a picture shows her, best kind first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    /// One of her Instagram posts.
    Timeline,
    /// A profile-type picture: `profile`, `onlyfans`, `fansly`, `x`, `reddit`, `pornhub`, `web`, ….
    Profile,
    /// Her Instagram profile photo (INSTAGRAM_PROFILE).
    InstagramProfile,
    /// A post's picture (`eroscripts`, `faptap`): a scene, not her.
    Post,
}

fn kind(p: &Value) -> Kind {
    match p.get("source").and_then(Value::as_str) {
        Some("instagram") if p.get("file").and_then(Value::as_str) == Some(INSTAGRAM_PROFILE) => {
            Kind::InstagramProfile
        }
        Some("instagram") => Kind::Timeline,
        Some("eroscripts" | "faptap") => Kind::Post,
        _ => Kind::Profile,
    }
}

/// Creator `c`'s pictures best first: her Instagram posts by most `likes`, then newest
/// (`takenAt`); then profile-type pictures, her Instagram profile photo, and post pictures
/// last, each in the file's order.
fn ranked(c: &Value) -> Vec<&Value> {
    let rank = |p: &&Value| {
        let k = kind(p);
        let timeline = |key: &str| p.get(key).filter(|_| k == Kind::Timeline);
        (
            k,
            Reverse(timeline("likes").and_then(Value::as_u64)),
            Reverse(
                timeline("takenAt")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            ),
        )
    };
    let mut pics: Vec<&Value> = c
        .get("pictures")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    pics.sort_by_cached_key(rank);
    pics
}

/// Creator `slug`'s best picture (`c` her creators.json entry) with its served URL: the first
/// of `ranked` that is her (not a post's picture) and on disk under
/// `<userData>/creator-pictures/<slug>/`; None without one.
pub fn best_picture<'a>(user_data: &Path, slug: &str, c: &'a Value) -> Option<(&'a Value, String)> {
    let dir = user_data.join(creator_pictures::DIR).join(slug);
    ranked(c)
        .into_iter()
        .filter(|p| kind(p) != Kind::Post)
        .find_map(|p| {
            let file = p.get("file").and_then(Value::as_str)?;
            let url = creator_pictures::url(slug, file)?;
            dir.join(file).is_file().then_some((p, url))
        })
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
            bonus,
            ..Creators::default()
        };
        let mut owners = HashMap::new();
        for (who, cr) in c.yes() {
            for key in posts(cr).filter_map(post_key_of) {
                owners.entry(key).or_insert_with(|| who.clone());
            }
        }
        let (mut everyone, mut listed, mut names) = (Vec::new(), HashMap::new(), HashMap::new());
        let mut longest = 0;
        for (i, (who, cr)) in c.all().enumerate() {
            for key in posts(cr).filter_map(post_key_of) {
                let list: &mut Vec<usize> = listed.entry(key).or_default();
                if list.last() != Some(&i) {
                    list.push(i);
                }
            }
            let aliases = strings_of(cr, "aliases").unwrap_or_default();
            for name in std::iter::once(&who.name).chain(&aliases) {
                let w = words(name);
                let key = w.concat();
                if key.chars().count() < MIN_NAME {
                    continue;
                }
                longest = longest.max(key.len());
                let entry: &mut Vec<(usize, bool)> = names.entry(key).or_default();
                match entry.iter_mut().find(|(j, _)| *j == i) {
                    // A name of several words with the same letters lets titles match too.
                    Some((_, one)) => *one &= w.len() == 1,
                    None => entry.push((i, w.len() == 1)),
                }
            }
            everyone.push(who);
        }
        c.owners = owners;
        c.everyone = everyone;
        c.listed = listed;
        c.names = names;
        c.longest = longest;
        c
    }

    /// Every creator, by slug, with the name her cards show (her slug when she has none).
    fn all(&self) -> impl Iterator<Item = (Who, &Value)> {
        self.file
            .get("creators")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(slug, c)| {
                let name = str_of(c, "name").filter(|n| !n.is_empty());
                let who = Who {
                    slug: slug.clone(),
                    name: name.unwrap_or_else(|| slug.clone()),
                };
                (who, c)
            })
    }

    /// The yes creators, by slug.
    fn yes(&self) -> impl Iterator<Item = (Who, &Value)> {
        self.all().filter(|(who, c)| self.is_yes(&who.slug, c))
    }

    /// The creators in `card` (`everyone` indexes), in order of evidence: the post is hers
    /// (the yes creator whose post it is first, as on her creator card, then every creator
    /// listing it, by slug); a scene's performer (`actors`) is her name or an alias, folded;
    /// her name or an alias runs in the title as whole words (one-word names of
    /// MIN_TITLE_WORD letters or more only), in the title's order.
    fn named(&self, card: &deck::DeckCard) -> Vec<usize> {
        let (key, title, actors) = match card {
            deck::DeckCard::Post(p) => (
                Some(format!("eroscripts:{}", p.id)),
                p.title.as_deref(),
                &[][..],
            ),
            deck::DeckCard::Scene(s) => (
                match s.source {
                    "faptap" => Some(format!("faptap:{}", s.id)),
                    SOURCE => post_key(&s.id),
                    _ => None,
                },
                s.title.as_deref(),
                s.actors.as_slice(),
            ),
        };
        let mut out = Vec::new();
        let mut add = |i: usize| {
            if !out.contains(&i) {
                out.push(i);
            }
        };
        if let Some(key) = &key
            && let Some(list) = self.listed.get(key)
        {
            let owner = self.owners.get(key).map(|w| w.slug.as_str());
            let (first, rest): (Vec<usize>, Vec<usize>) = list
                .iter()
                .partition(|&&i| Some(self.everyone[i].slug.as_str()) == owner);
            first.into_iter().chain(rest).for_each(&mut add);
        }
        for actor in actors {
            if let Some(found) = self.names.get(&words(actor).concat()) {
                found.iter().for_each(|&(i, _)| add(i));
            }
        }
        let title = words(title.unwrap_or_default());
        for start in 0..title.len() {
            let mut run = String::new();
            for w in &title[start..] {
                run.push_str(w);
                if run.len() > self.longest {
                    break;
                }
                let Some(found) = self.names.get(&run) else {
                    continue;
                };
                for &(i, one_word) in found {
                    if !one_word || run.chars().count() >= MIN_TITLE_WORD {
                        add(i);
                    }
                }
            }
        }
        out
    }

    /// Creator `i` (an `everyone` index) as a card's person, with her best picture; None
    /// without one on disk.
    fn person(&self, user_data: &Path, i: usize) -> Option<Person> {
        let who = &self.everyone[i];
        let c = self.file.get("creators")?.get(&who.slug)?;
        let (pic, picture) = best_picture(user_data, &who.slug, c)?;
        Some(Person {
            name: who.name.clone(),
            slug: who.slug.clone(),
            picture,
            picture_source: str_of(pic, "source").unwrap_or_default(),
            picture_url: str_of(pic, "url").filter(|u| u.starts_with("https://")),
        })
    }

    /// Name the creators in a dealt `card` (`people`, at most MAX_PEOPLE; named) and add each
    /// one's picture to its images (after its own, each once).
    pub fn add_people(&self, user_data: &Path, card: &mut deck::DeckCard) {
        let people: Vec<Person> = self
            .named(card)
            .into_iter()
            .filter_map(|i| self.person(user_data, i))
            .take(MAX_PEOPLE)
            .collect();
        let (images, slot) = match card {
            deck::DeckCard::Post(p) => (&mut p.images, &mut p.people),
            deck::DeckCard::Scene(s) => (&mut s.images, &mut s.people),
        };
        for p in &people {
            if !images.contains(&p.picture) {
                images.push(p.picture.clone());
            }
        }
        *slot = people;
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

    /// A creator card for post `p` (of `kind`) of creator `c`: the post's picture, then hers,
    /// best first (ranked).
    fn card(&self, who: &Who, c: &Value, p: &Value, kind: &str, id: String, vr: bool) -> SceneCard {
        let slug = who.slug.as_str();
        let post_score = p.get("score").and_then(Value::as_f64);
        let mut images: Vec<String> = str_of(p, "image")
            .and_then(|f| creator_pictures::url(slug, &f))
            .into_iter()
            .collect();
        for pic in ranked(c) {
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

    /// Write creator `slug`'s picture files under `dir`'s creator-pictures.
    fn on_disk_pictures(dir: &Path, slug: &str, files: &[&str]) {
        let pics = dir.join(creator_pictures::DIR).join(slug);
        std::fs::create_dir_all(&pics).unwrap();
        for f in files {
            std::fs::write(pics.join(f), b"jpeg").unwrap();
        }
    }

    fn scene(source: &'static str, id: &str, title: &str, actors: &[&str]) -> deck::DeckCard {
        deck::DeckCard::Scene(SceneCard {
            source,
            id: id.into(),
            title: Some(title.into()),
            actors: actors.iter().map(|a| a.to_string()).collect(),
            images: vec!["https://x.test/thumb.jpg".into()],
            ..SceneCard::default()
        })
    }

    #[test]
    fn names_the_creators_in_a_card() {
        let dir = temp("people");
        let pic = |file: &str| json!([{"file": file, "source": "instagram", "url": "https://www.instagram.com/p/A/"}]);
        let c = Creators::new(
            json!({"creators": {
                "agapilord": {"name": "Agapilord", "pictures": pic("ig-1.jpg")},
                "ann": {"name": "Ann", "pictures": pic("ig-1.jpg"), "posts": [{"kind": "faptap", "id": "7"}]},
                "bea": {"name": "Bea", "sources": ["favourite"], "pictures": pic("ig-1.jpg"),
                        "posts": [{"kind": "faptap", "id": "7"}, {"kind": "eroscripts", "id": 5}]},
                "kitty": {"name": "Kitty Sweet", "aliases": ["Zoë Löve"], "pictures": pic("ig-1.jpg")},
                "nopic": {"name": "No Pic", "pictures": pic("gone.jpg")},
                "pupu": {"name": "Pupu", "pictures": pic("ig-1.jpg")},
                "sweetie": {"name": "Sweetie Fox", "pictures": pic("ig-1.jpg")}
            }}),
            json!({"version": 1, "creators": {"sweetie": {"choice": "no"}}}),
            0.0,
        );
        for slug in ["agapilord", "ann", "bea", "kitty", "pupu", "sweetie"] {
            on_disk_pictures(&dir, slug, &["ig-1.jpg"]);
        }
        let names = |mut card: deck::DeckCard| {
            c.add_people(&dir, &mut card);
            let deck::DeckCard::Scene(s) = card else {
                unreachable!()
            };
            s.people.into_iter().map(|p| p.name).collect::<Vec<_>>()
        };
        // Her post first: the yes creator whose post it is, then the others listing it, by
        // slug; at most two, so the title's Sweetie Fox is left out.
        assert_eq!(
            names(scene("faptap", "7", "Sweetie Fox and Kitty Sweet", &[])),
            ["Bea", "Ann"]
        );
        assert_eq!(names(scene("creators", "eroscripts-5", "x", &[])), ["Bea"]);
        // A performer by an alias, accents and case folded; one-word names count there.
        assert_eq!(
            names(scene("vrb", "v1", "Hot scene", &["ZOE LOVE", "Pupu"])),
            ["Kitty Sweet", "Pupu"]
        );
        // In the title as whole words, or run together; any creator, swiped no too.
        for title in [
            "PMV …Sweetie Fox, Diana",
            "sweetiefox compilation",
            "SWEETIE-FOX",
        ] {
            assert_eq!(
                names(scene("pmvhaven", "p", title, &[])),
                ["Sweetie Fox"],
                "{title}"
            );
        }
        for title in ["Sweetie Foxes", "Asweetie Fox", "Pupu rides", "Bea and Ann"] {
            assert!(
                names(scene("pmvhaven", "p", title, &[])).is_empty(),
                "{title}"
            );
        }
        assert_eq!(
            names(scene("slr", "1", "agapilord pov", &[])),
            ["Agapilord"]
        );
        // A creator without a picture on disk is left out, and the next one counts.
        assert_eq!(
            names(scene(
                "pmvhaven",
                "p",
                "No Pic, Agapilord and Sweetie Fox",
                &[]
            )),
            ["Agapilord", "Sweetie Fox"]
        );

        // Her picture joins the card's images, once, with where it comes from.
        let mut card = scene("vrb", "v1", "Sweetie Fox", &["Sweetie Fox"]);
        c.add_people(&dir, &mut card);
        let deck::DeckCard::Scene(s) = card else {
            unreachable!()
        };
        assert_eq!(
            s.images,
            ["https://x.test/thumb.jpg", "/pictures/sweetie/ig-1.jpg"]
        );
        assert_eq!(
            serde_json::to_value(&s.people).unwrap(),
            json!([{"name": "Sweetie Fox", "slug": "sweetie", "picture": "/pictures/sweetie/ig-1.jpg",
                    "pictureSource": "instagram", "pictureUrl": "https://www.instagram.com/p/A/"}])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn her_best_picture_is_her_most_liked_instagram_post_on_disk() {
        let dir = temp("best");
        let best = |pictures: Value| {
            let c = json!({ "pictures": pictures });
            best_picture(&dir, "eva", &c)
                .map(|(p, url)| (p["file"].as_str().unwrap().to_string(), url))
        };
        let file = |pictures: Value| best(pictures).map(|(f, _)| f);
        on_disk_pictures(
            &dir,
            "eva",
            &[
                "ig-new.jpg",
                "ig-old.jpg",
                "ig-profile.jpg",
                "es-1-0.jpg",
                "ft-2.jpg",
                "of.jpg",
                "x.jpg",
            ],
        );
        let new = json!({"file": "ig-new.jpg", "source": "instagram", "takenAt": "2026-02-01T00:00:00.000Z", "likes": 10});
        let old = json!({"file": "ig-old.jpg", "source": "instagram", "takenAt": "2025-01-01T00:00:00.000Z", "likes": 500});
        let profile = json!({"file": "ig-profile.jpg", "source": "instagram"});
        let es = json!({"file": "es-1-0.jpg", "source": "eroscripts", "takenAt": "2026-11-01T00:00:00.000Z"});
        let ft = json!({"file": "ft-2.jpg", "source": "faptap"});
        let of = json!({"file": "of.jpg", "source": "onlyfans"});
        let x = json!({"file": "x.jpg", "source": "x"});
        // Most likes wins over newer.
        assert_eq!(
            best(json!([es, profile, new, old])),
            Some(("ig-old.jpg".into(), "/pictures/eva/ig-old.jpg".into()))
        );
        // No likes: the newest Instagram post.
        let unliked = |v: &Value| {
            let mut v = v.clone();
            v.as_object_mut().unwrap().remove("likes");
            v
        };
        assert_eq!(
            file(json!([unliked(&old), profile, unliked(&new)])).as_deref(),
            Some("ig-new.jpg")
        );
        // No Instagram post: her first profile-type picture, before her Instagram photo.
        assert_eq!(file(json!([es, profile, of, x])).as_deref(), Some("of.jpg"));
        assert_eq!(
            file(json!([es, profile])).as_deref(),
            Some("ig-profile.jpg")
        );
        // Never a post's picture.
        assert_eq!(file(json!([es, ft])), None);
        // A file missing on disk is skipped.
        let gone = json!({"file": "ig-gone.jpg", "source": "instagram", "likes": 9000});
        assert_eq!(file(json!([gone, new])).as_deref(), Some("ig-new.jpg"));
        assert_eq!(file(json!([{"file": "../x.jpg", "source": "x"}])), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn creator_cards_show_her_post_then_her_pictures_best_first() {
        let c = Creators::new(
            json!({"creators": {"ann": {"name": "Ann", "sources": ["favourite"],
                "pictures": [
                    {"file": "es-1-0.jpg", "source": "eroscripts"},
                    {"file": "ig-profile.jpg", "source": "instagram"},
                    {"file": "p.jpg", "source": "profile"},
                    {"file": "ig-a.jpg", "source": "instagram", "takenAt": "2026-01-01T00:00:00Z", "likes": 3},
                    {"file": "ig-b.jpg", "source": "instagram", "takenAt": "2025-01-01T00:00:00Z", "likes": 30}],
                "posts": [{"kind": "eroscripts", "id": 1, "image": "es-1-0.jpg", "downloadable": true}]}}}),
            Value::Null,
            0.0,
        );
        let (cards, _) = c.cards(Stack::Flat, &HashSet::new(), &HashMap::new());
        assert_eq!(
            cards[0].images,
            [
                "/pictures/ann/es-1-0.jpg",
                "/pictures/ann/ig-b.jpg",
                "/pictures/ann/ig-a.jpg",
                "/pictures/ann/p.jpg",
                "/pictures/ann/ig-profile.jpg"
            ]
        );
    }
}
