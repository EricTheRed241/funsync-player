//! The two decks, the Yes list and the summary, from creators.json and the swipes.
//!
//! - Creators deck: the creators without a swipe, best score first (no score last),
//!   with a seed shifting every score by up to JITTER either way (scout_report::deck::jitter,
//!   by the slug's hash), so each deal is a little different but stays near score order.
//! - Posts deck: the posts of the creators swiped yes that are downloadable, not in the
//!   library and not swiped, best score first (no score last); a post listed under two
//!   creators comes once.
//!
//! A page of either is `offset` cards into the order, `limit` long; `remaining` counts
//! the whole deck.

use crate::creators::{Creator, Creators, Post};
use crate::pictures;
use crate::swipes::{Choice, PostChoice, Swipes};
use serde::Serialize;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::HashSet;

/// Score points a seeded creators deal moves each card by, at most, either way.
pub const JITTER: f64 = 2.0;
pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 100;
/// Top posts a creator card lists.
pub const TOP_POSTS: usize = 3;

/// Which part of a deck to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    /// Shuffle a little (the creators deck only); the same seed deals the same order.
    pub seed: Option<u64>,
    pub offset: usize,
    pub limit: usize,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            seed: None,
            offset: 0,
            limit: DEFAULT_LIMIT,
        }
    }
}

/// One picture of a card: its URL here, where it comes from, and its page there.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pic {
    pub src: String,
    pub source: String,
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstagramCard {
    pub handle: String,
    pub followers: Value,
    pub posts: Value,
    /// None until creator-scout has an Instagram login ("no login yet").
    pub last_post_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TwitchCard {
    pub handle: String,
    pub followers: Value,
    pub last_live: Option<String>,
}

/// A creator's online content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    /// Free EroScripts posts with a video the scout can fetch.
    pub eroscripts_free: usize,
    pub eroscripts_paid: usize,
    pub faptap: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TopPost {
    pub kind: String,
    pub title: String,
    pub url: String,
    pub score: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatorCard {
    pub slug: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub score: Option<f64>,
    pub first_seen: Option<String>,
    pub pictures: Vec<Pic>,
    pub instagram: Option<InstagramCard>,
    pub twitch: Option<TwitchCard>,
    pub counts: Counts,
    pub top_posts: Vec<TopPost>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostCard {
    /// `<kind>:<id>`, its key in creator-swipes.json.
    pub key: String,
    pub kind: String,
    pub id: String,
    pub title: String,
    pub url: String,
    /// The creator's slug.
    pub creator: String,
    pub creator_name: String,
    /// The post's image, else the creator's pictures.
    pub pictures: Vec<Pic>,
    pub hosts: Vec<String>,
    pub creampie: bool,
    pub multi_axis: bool,
    pub score: Option<f64>,
    pub likes: Option<u64>,
    pub duration_s: Option<f64>,
    pub tags: Vec<String>,
    pub created_at: Option<String>,
}

/// What both decks send besides the cards.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    /// creators.json doesn't exist yet.
    pub missing: bool,
    pub updated_at: Option<String>,
    /// Files that couldn't be read.
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatorDeck {
    pub cards: Vec<CreatorCard>,
    /// Creators not swiped yet.
    pub remaining: usize,
    /// Creators swiped yes.
    pub yes: usize,
    #[serde(flatten)]
    pub meta: Meta,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostDeck {
    pub cards: Vec<PostCard>,
    /// Posts in the deck (not swiped yet).
    pub remaining: usize,
    /// Posts swiped want.
    pub wanted: usize,
    #[serde(flatten)]
    pub meta: Meta,
}

/// A creator swiped yes, for the Yes list.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YesEntry {
    pub slug: String,
    pub name: String,
    pub at: String,
    pub score: Option<f64>,
    pub picture: Option<String>,
    /// Their posts in the Posts deck.
    pub unswiped: usize,
    /// Their posts swiped want.
    pub wanted: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatorCounts {
    pub total: usize,
    pub yes: usize,
    pub no: usize,
    pub unswiped: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostCounts {
    /// In the Posts deck.
    pub unswiped: usize,
    pub wanted: usize,
    /// Wanted and not in the library yet: the next daily run downloads them.
    pub pending_download: usize,
    /// Wanted and in the library now.
    pub in_library: usize,
    pub hidden: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub creators: CreatorCounts,
    pub posts: PostCounts,
    #[serde(flatten)]
    pub meta: Meta,
}

/// FNV-1a: a slug as a number for the shuffle.
fn slug_hash(slug: &str) -> u64 {
    slug.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Higher scores first, None last.
fn by_score(a: Option<f64>, b: Option<f64>) -> Ordering {
    let key = |s: Option<f64>| s.unwrap_or(f64::NEG_INFINITY);
    key(b).total_cmp(&key(a))
}

fn display_name<'a>(slug: &'a str, c: &'a Creator) -> &'a str {
    if c.name.is_empty() { slug } else { &c.name }
}

/// The creators not swiped yet, in deck order for `seed`.
pub fn creator_order<'a>(
    creators: &'a Creators,
    swipes: &Swipes,
    seed: Option<u64>,
) -> Vec<(&'a str, &'a Creator)> {
    let key = |slug: &str, c: &Creator| match (c.score, seed) {
        (Some(s), Some(seed)) => {
            Some(s + scout_report::deck::jitter(seed, slug_hash(slug), JITTER))
        }
        (score, _) => score,
    };
    let mut list: Vec<_> = creators
        .creators
        .iter()
        .filter(|(slug, _)| swipes.creator(slug).is_none())
        .map(|(slug, c)| (key(slug, c), slug.as_str(), c))
        .collect();
    list.sort_by(|(ka, sa, a), (kb, sb, b)| {
        by_score(*ka, *kb)
            .then_with(|| {
                display_name(sa, a)
                    .to_lowercase()
                    .cmp(&display_name(sb, b).to_lowercase())
            })
            .then(sa.cmp(sb))
    });
    list.into_iter().map(|(_, slug, c)| (slug, c)).collect()
}

/// Whether post `p` belongs in the Posts deck (for a creator swiped yes).
fn in_post_deck(p: &Post, swipes: &Swipes) -> bool {
    p.downloadable && !p.in_library && !swipes.posts.contains_key(&p.key())
}

/// The Posts deck, in order: (slug, creator, post).
pub fn post_order<'a>(
    creators: &'a Creators,
    swipes: &Swipes,
) -> Vec<(&'a str, &'a Creator, &'a Post)> {
    let mut seen = HashSet::new();
    let mut list: Vec<_> = creators
        .creators
        .iter()
        .filter(|(slug, _)| swipes.creator(slug) == Some(Choice::Yes))
        .flat_map(|(slug, c)| c.posts.iter().map(move |p| (slug.as_str(), c, p)))
        .filter(|(_, _, p)| in_post_deck(p, swipes))
        .collect();
    list.sort_by(|(sa, _, a), (sb, _, b)| {
        by_score(a.score, b.score)
            .then_with(|| sa.cmp(sb))
            .then_with(|| a.key().cmp(&b.key()))
    });
    list.retain(|(_, _, p)| seen.insert(p.key()));
    list
}

/// A creator's pictures as served here: Instagram's first, then the rest, each in the file's order.
pub fn creator_pictures(slug: &str, c: &Creator) -> Vec<Pic> {
    let (mut list, rest): (Vec<_>, Vec<_>) =
        c.pictures.iter().partition(|p| p.source == "instagram");
    list.extend(rest);
    list.into_iter()
        .filter_map(|p| {
            Some(Pic {
                src: pictures::url(slug, &p.file)?,
                source: p.source.clone(),
                link: p.url.clone(),
            })
        })
        .collect()
}

pub fn counts(c: &Creator) -> Counts {
    let mut n = Counts::default();
    for p in &c.posts {
        if p.is_eroscripts() {
            if !p.free {
                n.eroscripts_paid += 1;
            } else if p.downloadable {
                n.eroscripts_free += 1;
            }
        } else if p.is_faptap() {
            n.faptap += 1;
        }
    }
    n
}

pub fn creator_card(slug: &str, c: &Creator) -> CreatorCard {
    let mut top: Vec<&Post> = c.posts.iter().collect();
    top.sort_by(|a, b| by_score(a.score, b.score));
    CreatorCard {
        slug: slug.to_string(),
        name: display_name(slug, c).to_string(),
        aliases: c.aliases.clone(),
        score: c.score,
        first_seen: c.first_seen.clone(),
        pictures: creator_pictures(slug, c),
        instagram: c.instagram.as_ref().map(|i| InstagramCard {
            handle: i.handle.clone(),
            followers: i.followers.clone(),
            posts: i.posts.clone(),
            last_post_at: i.last_post_at.clone(),
        }),
        twitch: c.twitch.as_ref().map(|t| TwitchCard {
            handle: t.handle.clone(),
            followers: t.followers.clone(),
            last_live: t.last_live.clone(),
        }),
        counts: counts(c),
        top_posts: top
            .into_iter()
            .take(TOP_POSTS)
            .map(|p| TopPost {
                kind: p.kind.clone(),
                title: p.title.clone(),
                url: p.url.clone(),
                score: p.score,
            })
            .collect(),
    }
}

pub fn post_card(slug: &str, c: &Creator, p: &Post) -> PostCard {
    let image = p
        .image
        .as_deref()
        .and_then(|f| pictures::url(slug, f))
        .map(|src| Pic {
            src,
            source: p.kind.clone(),
            link: Some(p.url.clone()).filter(|u| !u.is_empty()),
        });
    PostCard {
        key: p.key(),
        kind: p.kind.clone(),
        id: p.id.clone(),
        title: p.title.clone(),
        url: p.url.clone(),
        creator: slug.to_string(),
        creator_name: display_name(slug, c).to_string(),
        pictures: image.map_or_else(|| creator_pictures(slug, c), |i| vec![i]),
        hosts: p.hosts.clone(),
        creampie: p.creampie,
        multi_axis: p.multi_axis,
        score: p.score,
        likes: p.likes,
        duration_s: p.duration_s,
        tags: p.tags.clone(),
        created_at: p.created_at.clone(),
    }
}

fn meta(creators: Option<&Creators>, problems: Vec<String>) -> Meta {
    Meta {
        missing: creators.is_none() && problems.is_empty(),
        updated_at: creators.and_then(|c| c.updated_at.clone()),
        problems,
    }
}

fn yes_count(creators: &Creators, swipes: &Swipes) -> usize {
    creators
        .creators
        .keys()
        .filter(|s| swipes.creator(s) == Some(Choice::Yes))
        .count()
}

fn wanted_count(swipes: &Swipes) -> usize {
    swipes
        .posts
        .values()
        .filter(|s| s.choice == PostChoice::Want)
        .count()
}

/// A page of the Creators deck. `creators` None: the file is missing (or broken: `problems`).
pub fn creator_deck(
    creators: Option<&Creators>,
    swipes: &Swipes,
    page: &Page,
    problems: Vec<String>,
) -> CreatorDeck {
    let (cards, remaining, yes) = match creators {
        Some(all) => {
            let order = creator_order(all, swipes, page.seed);
            let cards = order
                .iter()
                .skip(page.offset)
                .take(page.limit)
                .map(|(slug, c)| creator_card(slug, c))
                .collect();
            (cards, order.len(), yes_count(all, swipes))
        }
        None => (Vec::new(), 0, 0),
    };
    CreatorDeck {
        cards,
        remaining,
        yes,
        meta: meta(creators, problems),
    }
}

/// A page of the Posts deck.
pub fn post_deck(
    creators: Option<&Creators>,
    swipes: &Swipes,
    page: &Page,
    problems: Vec<String>,
) -> PostDeck {
    let (cards, remaining) = match creators {
        Some(all) => {
            let order = post_order(all, swipes);
            let cards = order
                .iter()
                .skip(page.offset)
                .take(page.limit)
                .map(|(slug, c, p)| post_card(slug, c, p))
                .collect();
            (cards, order.len())
        }
        None => (Vec::new(), 0),
    };
    PostDeck {
        cards,
        remaining,
        wanted: wanted_count(swipes),
        meta: meta(creators, problems),
    }
}

/// The creators swiped yes that creators.json lists, by name.
pub fn yes_list(creators: &Creators, swipes: &Swipes) -> Vec<YesEntry> {
    let mut list: Vec<YesEntry> = creators
        .creators
        .iter()
        .filter_map(|(slug, c)| {
            let swipe = swipes.creators.get(slug)?;
            (swipe.choice == Choice::Yes).then(|| YesEntry {
                slug: slug.clone(),
                name: display_name(slug, c).to_string(),
                at: swipe.at.clone(),
                score: c.score,
                picture: creator_pictures(slug, c).into_iter().next().map(|p| p.src),
                unswiped: c.posts.iter().filter(|p| in_post_deck(p, swipes)).count(),
                wanted: swipes
                    .posts
                    .values()
                    .filter(|s| s.creator == *slug && s.choice == PostChoice::Want)
                    .count(),
            })
        })
        .collect();
    list.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.slug.cmp(&b.slug))
    });
    list
}

pub fn summary(creators: Option<&Creators>, swipes: &Swipes, problems: Vec<String>) -> Summary {
    let mut s = Summary::default();
    if let Some(all) = creators {
        let c = &mut s.creators;
        c.total = all.creators.len();
        for slug in all.creators.keys() {
            match swipes.creator(slug) {
                Some(Choice::Yes) => c.yes += 1,
                Some(Choice::No) => c.no += 1,
                None => c.unswiped += 1,
            }
        }
        s.posts.unswiped = post_order(all, swipes).len();
    }
    for (key, swipe) in &swipes.posts {
        match swipe.choice {
            PostChoice::Hide => s.posts.hidden += 1,
            PostChoice::Want => {
                s.posts.wanted += 1;
                if creators
                    .and_then(|c| c.find_post(key))
                    .is_some_and(|p| p.in_library)
                {
                    s.posts.in_library += 1;
                } else {
                    s.posts.pending_download += 1;
                }
            }
        }
    }
    s.meta = meta(creators, problems);
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swipes::{CreatorSwipe, PostSwipe};
    use serde_json::json;

    fn creators(v: Value) -> Creators {
        serde_json::from_value(v).unwrap()
    }

    fn sample() -> Creators {
        creators(json!({"updatedAt": "2026-11-18T01:40:00Z", "creators": {
            "ann": {"name": "Ann", "score": 5.0,
                "pictures": [{"file": "es-1-0.jpg", "source": "eroscripts"},
                             {"file": "ig-1.jpg", "source": "instagram", "url": "https://www.instagram.com/p/a/"},
                             {"file": "../evil.jpg", "source": "instagram"}],
                "instagram": {"handle": "ann", "followers": "5M", "lastPostAt": null},
                "posts": [
                    {"kind": "eroscripts", "id": "1", "title": "Free one", "free": true, "downloadable": true, "score": 7.0, "image": "es-1-0.jpg", "url": "https://discuss.eroscripts.com/t/1"},
                    {"kind": "eroscripts", "id": "2", "title": "Paid", "free": false, "downloadable": false, "score": 9.0},
                    {"kind": "eroscripts", "id": "3", "title": "No video", "free": true, "downloadable": false, "score": 1.0},
                    {"kind": "faptap", "id": "100", "title": "Tap", "free": true, "downloadable": true, "score": null},
                    {"kind": "faptap", "id": "101", "title": "Had", "free": true, "downloadable": true, "score": 8.0, "inLibrary": true}]},
            "bea": {"name": "Bea", "score": 6.0, "posts": [
                    {"kind": "eroscripts", "id": "5", "title": "Bea's", "free": true, "downloadable": true, "score": 7.5},
                    {"kind": "eroscripts", "id": "1", "title": "Collab", "free": true, "downloadable": true, "score": 7.0}]},
            "cat": {"name": "", "score": null},
            "dee": {"name": "Dee", "score": 5.0}
        }}))
    }

    fn swipes(creators: &[(&str, Choice)], posts: &[(&str, PostChoice, &str)]) -> Swipes {
        let mut s = Swipes::default();
        for (slug, choice) in creators {
            s.creators.insert(
                slug.to_string(),
                CreatorSwipe {
                    choice: *choice,
                    at: "t".into(),
                },
            );
        }
        for (key, choice, creator) in posts {
            s.posts.insert(
                key.to_string(),
                PostSwipe {
                    choice: *choice,
                    at: "t".into(),
                    creator: creator.to_string(),
                },
            );
        }
        s
    }

    fn slugs(order: &[(&str, &Creator)]) -> Vec<String> {
        order.iter().map(|(s, _)| s.to_string()).collect()
    }

    #[test]
    fn creators_deal_best_score_first_nulls_last_unswiped_only() {
        let all = sample();
        let order = creator_order(&all, &Swipes::default(), None);
        // Ties by name: Ann before Dee.
        assert_eq!(slugs(&order), ["bea", "ann", "dee", "cat"]);
        let s = swipes(&[("bea", Choice::Yes), ("dee", Choice::No)], &[]);
        assert_eq!(slugs(&creator_order(&all, &s, None)), ["ann", "cat"]);
    }

    #[test]
    fn a_seed_shuffles_a_little_and_repeats() {
        let mut v = serde_json::Map::new();
        for i in 0..60 {
            v.insert(
                format!("c{i}"),
                json!({"name": format!("C{i}"), "score": f64::from(i) / 2.0}),
            );
        }
        v.insert("none".into(), json!({"name": "None"}));
        let all = creators(json!({"creators": v}));
        let none = Swipes::default();
        let plain = slugs(&creator_order(&all, &none, None));
        let a = slugs(&creator_order(&all, &none, Some(7)));
        assert_eq!(a, slugs(&creator_order(&all, &none, Some(7))));
        assert_ne!(a, plain);
        assert_ne!(a, slugs(&creator_order(&all, &none, Some(8))));
        // Unscored stays last; a card only passes ones within 2 × JITTER points.
        assert_eq!(a.last().unwrap(), "none");
        let score = |s: &str| all.creators[s].score.unwrap();
        for (i, x) in a.iter().enumerate().take(a.len() - 1) {
            for y in &a[i + 1..a.len() - 1] {
                assert!(score(x) >= score(y) - 2.0 * JITTER, "{x} before {y}");
            }
        }
    }

    #[test]
    fn pages_by_offset_and_counts() {
        let all = sample();
        let none = Swipes::default();
        let page = |offset, limit| Page {
            seed: None,
            offset,
            limit,
        };
        let d = creator_deck(Some(&all), &none, &page(1, 2), vec![]);
        let got: Vec<_> = d.cards.iter().map(|c| c.slug.as_str()).collect();
        assert_eq!(got, ["ann", "dee"]);
        assert_eq!((d.remaining, d.yes), (4, 0));
        assert!(!d.meta.missing);
        assert_eq!(d.meta.updated_at.as_deref(), Some("2026-11-18T01:40:00Z"));
        assert!(
            creator_deck(Some(&all), &none, &page(9, 2), vec![])
                .cards
                .is_empty()
        );

        let d = creator_deck(None, &none, &Page::default(), vec![]);
        assert!(d.meta.missing && d.cards.is_empty() && d.remaining == 0);
        let d = creator_deck(
            None,
            &none,
            &Page::default(),
            vec!["creators.json: bad".into()],
        );
        assert!(!d.meta.missing);
    }

    #[test]
    fn a_creator_card_has_pictures_instagram_first_counts_and_top_posts() {
        let all = sample();
        let c = creator_card("ann", &all.creators["ann"]);
        let srcs: Vec<_> = c.pictures.iter().map(|p| p.src.as_str()).collect();
        // Instagram first; the bad file name is left out.
        assert_eq!(srcs, ["/pictures/ann/ig-1.jpg", "/pictures/ann/es-1-0.jpg"]);
        assert_eq!(
            c.pictures[0].link.as_deref(),
            Some("https://www.instagram.com/p/a/")
        );
        assert_eq!(
            c.counts,
            Counts {
                eroscripts_free: 1,
                eroscripts_paid: 1,
                faptap: 2
            }
        );
        let top: Vec<_> = c.top_posts.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(top, ["Paid", "Had", "Free one"]);
        assert_eq!(c.instagram.unwrap().last_post_at, None);
        // A creator without a name shows the slug.
        assert_eq!(creator_card("cat", &all.creators["cat"]).name, "cat");
    }

    #[test]
    fn posts_come_from_yes_creators_downloadable_not_in_library_not_swiped() {
        let all = sample();
        // Nobody swiped yes: no posts.
        assert!(post_order(&all, &Swipes::default()).is_empty());
        let s = swipes(&[("ann", Choice::Yes), ("bea", Choice::No)], &[]);
        let keys: Vec<_> = post_order(&all, &s)
            .iter()
            .map(|(_, _, p)| p.key())
            .collect();
        // Score order, null last; paid, no-video and in-library posts left out.
        assert_eq!(keys, ["eroscripts:1", "faptap:100"]);

        let s = swipes(
            &[("ann", Choice::Yes), ("bea", Choice::Yes)],
            &[("faptap:100", PostChoice::Hide, "ann")],
        );
        let order = post_order(&all, &s);
        let got: Vec<_> = order.iter().map(|(slug, _, p)| (*slug, p.key())).collect();
        // The collab (in both) comes once; swiped posts are gone.
        assert_eq!(
            got,
            [
                ("bea", "eroscripts:5".to_string()),
                ("ann", "eroscripts:1".to_string())
            ]
        );
        let d = post_deck(Some(&all), &s, &Page::default(), vec![]);
        assert_eq!((d.remaining, d.wanted), (2, 0));
    }

    #[test]
    fn a_post_card_shows_its_image_else_the_creators_pictures() {
        let all = sample();
        let ann = &all.creators["ann"];
        let c = post_card("ann", ann, &ann.posts[0]);
        assert_eq!(c.key, "eroscripts:1");
        assert_eq!(c.creator_name, "Ann");
        assert_eq!(c.pictures.len(), 1);
        assert_eq!(c.pictures[0].src, "/pictures/ann/es-1-0.jpg");
        assert_eq!(
            c.pictures[0].link.as_deref(),
            Some("https://discuss.eroscripts.com/t/1")
        );
        let c = post_card("ann", ann, &ann.posts[3]);
        let srcs: Vec<_> = c.pictures.iter().map(|p| p.src.as_str()).collect();
        assert_eq!(srcs, ["/pictures/ann/ig-1.jpg", "/pictures/ann/es-1-0.jpg"]);
    }

    #[test]
    fn yes_list_and_summary() {
        let all = sample();
        let s = swipes(
            &[
                ("ann", Choice::Yes),
                ("dee", Choice::No),
                ("gone", Choice::Yes),
            ],
            &[
                ("eroscripts:1", PostChoice::Want, "ann"),
                ("faptap:101", PostChoice::Want, "ann"),
                ("faptap:999", PostChoice::Want, "gone"),
                ("eroscripts:3", PostChoice::Hide, "ann"),
            ],
        );
        let yes = yes_list(&all, &s);
        assert_eq!(yes.len(), 1);
        assert_eq!(
            (yes[0].slug.as_str(), yes[0].unswiped, yes[0].wanted),
            ("ann", 1, 2)
        );
        assert_eq!(yes[0].picture.as_deref(), Some("/pictures/ann/ig-1.jpg"));

        let sum = summary(Some(&all), &s, vec![]);
        assert_eq!(
            sum.creators,
            CreatorCounts {
                total: 4,
                yes: 1,
                no: 1,
                unswiped: 2
            }
        );
        assert_eq!(
            sum.posts,
            PostCounts {
                unswiped: 1,
                wanted: 3,
                pending_download: 2,
                in_library: 1,
                hidden: 1
            }
        );
        let v = serde_json::to_value(&sum).unwrap();
        assert_eq!(v["posts"]["pendingDownload"], 2);
        assert_eq!(v["updatedAt"], "2026-11-18T01:40:00Z");
        assert_eq!(v["missing"], false);
        assert!(summary(None, &Swipes::default(), vec![]).meta.missing);
    }
}
