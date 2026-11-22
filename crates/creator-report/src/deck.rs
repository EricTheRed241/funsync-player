//! The Creators deck, the Swiped and Collection lists and the summary, from creators.json
//! and the swipes.
//!
//! A creator is yes when swiped yes, or, without a swipe, when she is a favourite
//! artist or on the pornstar list (creators.json `sources`, set by creator-scout
//! --favourites); swiped no wins over both (scout_report::creators applies the same rule:
//! the swipe deck deals the yes creators' posts).
//!
//! - Creators deck: the creators without a swipe that aren't yes by source, best score
//!   first (no score last), with a seed shifting every score by up to JITTER either way
//!   (scout_report::deck::jitter, by the slug's hash), so each deal is a little different
//!   but stays near score order.
//! - Swiped list: the swiped creators (yes, no or both), newest swipe first.
//! - Collection: the yes creators, by name.
//!
//! A page of the deck is `offset` cards into the order, `limit` long; `remaining` counts
//! the whole deck. The lists come whole, each entry with her full card.

use crate::creators::{Creator, Creators, Post};
use crate::swipes::{Choice, PostChoice, Swipes};
use scout_report::creator_pictures as pictures;
use serde::Serialize;
use serde_json::Value;
use std::cmp::Ordering;

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
    /// Our Instagram account follows her: `following` / `requested`.
    pub followed: Option<String>,
}

/// A confirmed profile page, shown as a small link.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProfileLink {
    pub site: String,
    pub url: String,
    pub handle: String,
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
    /// Why the user wants her: `swiped`, `favourite`, `pornstar` (creator-scout).
    pub sources: Vec<String>,
    /// Her confirmed profile pages (https links only).
    pub profiles: Vec<ProfileLink>,
}

/// What the deck and the lists send besides the cards.
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
    /// Yes creators (swiped yes, or a favourite / pornstar-list one without a swipe).
    pub yes: usize,
    #[serde(flatten)]
    pub meta: Meta,
}

/// A creator with her swipe, for the Swiped and Collection tabs: her whole card, so the
/// page can show it without another request.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedCreator {
    #[serde(flatten)]
    pub card: CreatorCard,
    /// Her swipe; None when she isn't swiped (yes by source).
    pub choice: Option<Choice>,
    /// When she was swiped.
    pub at: Option<String>,
    /// `favourite` or `pornstar` when she is yes by source, not by a swipe.
    pub from: Option<&'static str>,
    /// Her posts in the Posts deck (counted only while she is yes).
    pub unswiped: usize,
    /// Her posts swiped want.
    pub wanted: usize,
}

/// The Swiped or Collection list.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatorList {
    pub cards: Vec<ListedCreator>,
    #[serde(flatten)]
    pub meta: Meta,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatorCounts {
    pub total: usize,
    pub yes: usize,
    pub no: usize,
    pub unswiped: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub creators: CreatorCounts,
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

/// The source that makes a creator yes without a swipe: `favourite`, else `pornstar`.
pub fn yes_source(c: &Creator) -> Option<&'static str> {
    ["favourite", "pornstar"]
        .into_iter()
        .find(|s| c.sources.iter().any(|x| x == s))
}

/// Whether creator `slug` is yes: swiped yes, or not swiped and yes by source.
pub fn is_yes(slug: &str, c: &Creator, swipes: &Swipes) -> bool {
    match swipes.creator(slug) {
        Some(choice) => choice == Choice::Yes,
        None => yes_source(c).is_some(),
    }
}

/// The creators not swiped yet and not yes by source, in deck order for `seed`.
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
        .filter(|(slug, c)| swipes.creator(slug).is_none() && yes_source(c).is_none())
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

/// Whether post `p` of a yes creator is still to swipe: the swipe deck deals it (as her
/// creator card, or as the waiting post or FapTap scene it is).
fn to_swipe(p: &Post, swipes: &Swipes) -> bool {
    p.downloadable && !p.in_library && !swipes.posts.contains_key(&p.key())
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
            followed: i.followed.clone(),
        }),
        twitch: c.twitch.as_ref().map(|t| TwitchCard {
            handle: t.handle.clone(),
            followers: t.followers.clone(),
            last_live: t.last_live.clone(),
        }),
        counts: counts(c),
        sources: c.sources.clone(),
        profiles: c
            .profiles
            .iter()
            .filter(|p| p.url.starts_with("https://"))
            .map(|p| ProfileLink {
                site: p.site.clone(),
                url: p.url.clone(),
                handle: p.handle.clone(),
            })
            .collect(),
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
        .iter()
        .filter(|(s, c)| is_yes(s, c, swipes))
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

/// Creator `slug` with her swipe and post counts.
fn listed(slug: &str, c: &Creator, swipes: &Swipes) -> ListedCreator {
    let swipe = swipes.creators.get(slug);
    ListedCreator {
        card: creator_card(slug, c),
        choice: swipe.map(|s| s.choice),
        at: swipe.map(|s| s.at.clone()),
        from: swipe.map_or_else(|| yes_source(c), |_| None),
        unswiped: if is_yes(slug, c, swipes) {
            c.posts.iter().filter(|p| to_swipe(p, swipes)).count()
        } else {
            0
        },
        wanted: swipes
            .posts
            .values()
            .filter(|s| s.creator == slug && s.choice == PostChoice::Want)
            .count(),
    }
}

/// The Swiped tab: the swiped creators (yes and no) that creators.json lists, newest
/// swipe first.
pub fn swiped_list(
    creators: Option<&Creators>,
    swipes: &Swipes,
    problems: Vec<String>,
) -> CreatorList {
    let mut cards: Vec<ListedCreator> = creators.map_or_else(Vec::new, |all| {
        swipes
            .creators
            .keys()
            .filter_map(|slug| Some(listed(slug, all.creators.get(slug)?, swipes)))
            .collect()
    });
    cards.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.card.slug.cmp(&b.card.slug)));
    CreatorList {
        cards,
        meta: meta(creators, problems),
    }
}

/// The Collection tab: the yes creators (swiped yes, or by source), by name.
pub fn collection(
    creators: Option<&Creators>,
    swipes: &Swipes,
    problems: Vec<String>,
) -> CreatorList {
    let mut cards: Vec<ListedCreator> = creators.map_or_else(Vec::new, |all| {
        all.creators
            .iter()
            .filter(|(slug, c)| is_yes(slug, c, swipes))
            .map(|(slug, c)| listed(slug, c, swipes))
            .collect()
    });
    cards.sort_by(|a, b| {
        a.card
            .name
            .to_lowercase()
            .cmp(&b.card.name.to_lowercase())
            .then_with(|| a.card.slug.cmp(&b.card.slug))
    });
    CreatorList {
        cards,
        meta: meta(creators, problems),
    }
}

pub fn summary(creators: Option<&Creators>, swipes: &Swipes, problems: Vec<String>) -> Summary {
    let mut s = Summary::default();
    if let Some(all) = creators {
        let c = &mut s.creators;
        c.total = all.creators.len();
        for (slug, cr) in &all.creators {
            match swipes.creator(slug) {
                Some(Choice::No) => c.no += 1,
                _ if is_yes(slug, cr, swipes) => c.yes += 1,
                _ => c.unswiped += 1,
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
        assert_eq!(srcs, ["pictures/ann/ig-1.jpg", "pictures/ann/es-1-0.jpg"]);
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
    fn a_yes_creators_posts_to_swipe_are_downloadable_not_in_library_not_swiped() {
        let all = sample();
        let unswiped = |s: &Swipes| -> Vec<(String, usize)> {
            collection(Some(&all), s, vec![])
                .cards
                .iter()
                .map(|c| (c.card.slug.clone(), c.unswiped))
                .collect()
        };
        // Paid, no-video and in-library posts don't count.
        let s = swipes(&[("ann", Choice::Yes), ("bea", Choice::No)], &[]);
        assert_eq!(unswiped(&s), [("ann".to_string(), 2)]);
        // A swiped post (either way) doesn't either.
        let s = swipes(
            &[("ann", Choice::Yes), ("bea", Choice::Yes)],
            &[("faptap:100", PostChoice::Hide, "ann")],
        );
        assert_eq!(
            unswiped(&s),
            [("ann".to_string(), 1), ("bea".to_string(), 2)]
        );
    }

    #[test]
    fn collection_and_summary() {
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
        // A swipe of a creator creators.json no longer lists ("gone") has no card.
        let yes = collection(Some(&all), &s, vec![]).cards;
        assert_eq!(yes.len(), 1);
        assert_eq!(
            (yes[0].card.slug.as_str(), yes[0].unswiped, yes[0].wanted),
            ("ann", 1, 2)
        );
        assert_eq!(yes[0].card.pictures[0].src, "pictures/ann/ig-1.jpg");
        assert_eq!(yes[0].card.instagram.as_ref().unwrap().handle, "ann");
        let v = serde_json::to_value(&yes[0]).unwrap();
        assert_eq!((&v["slug"], &v["choice"]), (&json!("ann"), &json!("yes")));

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
        let v = serde_json::to_value(&sum).unwrap();
        assert_eq!(v["updatedAt"], "2026-11-18T01:40:00Z");
        assert_eq!(v["missing"], false);
        assert!(summary(None, &Swipes::default(), vec![]).meta.missing);
    }

    #[test]
    fn favourites_and_the_pornstar_list_are_yes_unless_swiped_no() {
        let all = creators(json!({"creators": {
            "fav": {"name": "Fav", "score": 9.0, "sources": ["favourite", "pornstar"], "posts": [
                {"kind": "faptap", "id": "1", "title": "F", "free": true, "downloadable": true, "score": 5.0}]},
            "star": {"name": "Star", "score": 8.0, "sources": ["pornstar"], "posts": [
                {"kind": "eroscripts", "id": "2", "title": "S", "free": true, "downloadable": true, "score": 6.0}]},
            "said": {"name": "Said No", "score": 7.0, "sources": ["favourite"], "posts": [
                {"kind": "faptap", "id": "3", "title": "N", "free": true, "downloadable": true, "score": 7.0}]},
            "swiped": {"name": "Swiped", "score": 6.0, "sources": ["swiped"]},
            "plain": {"name": "Plain", "score": 5.0}
        }}));
        let s = swipes(
            &[("said", Choice::No), ("swiped", Choice::Yes)],
            &[("eroscripts:2", PostChoice::Want, "star")],
        );
        // Yes by source: not in the Creators deck; only Plain is left to swipe.
        assert_eq!(slugs(&creator_order(&all, &s, None)), ["plain"]);
        let d = creator_deck(Some(&all), &s, &Page::default(), vec![]);
        assert_eq!((d.remaining, d.yes), (1, 3));
        // Their downloadable posts are to swipe; swiped no's aren't listed.
        let to_swipe: Vec<_> = collection(Some(&all), &s, vec![])
            .cards
            .iter()
            .map(|c| (c.card.slug.clone(), c.unswiped))
            .collect();
        assert_eq!(
            to_swipe,
            [
                ("fav".to_string(), 1),
                ("star".to_string(), 0),
                ("swiped".to_string(), 0)
            ]
        );
        let yes = collection(Some(&all), &s, vec![]).cards;
        let got: Vec<_> = yes
            .iter()
            .map(|y| (y.card.slug.as_str(), y.from, y.at.is_some()))
            .collect();
        assert_eq!(
            got,
            [
                ("fav", Some("favourite"), false),
                ("star", Some("pornstar"), false),
                ("swiped", None, true)
            ]
        );
        assert_eq!(yes[0].card.sources, ["favourite", "pornstar"]);
        assert_eq!(yes[1].wanted, 1);
        let sum = summary(Some(&all), &s, vec![]);
        assert_eq!(
            sum.creators,
            CreatorCounts {
                total: 5,
                yes: 3,
                no: 1,
                unswiped: 1
            }
        );
        // A card shows the sources.
        assert_eq!(
            creator_card("fav", &all.creators["fav"]).sources,
            ["favourite", "pornstar"]
        );
        // Swiped no wins; undoing it makes her yes by source again.
        let none = Swipes::default();
        assert!(is_yes("said", &all.creators["said"], &none));
        assert!(!is_yes("said", &all.creators["said"], &s));
        assert!(!is_yes("plain", &all.creators["plain"], &none));
    }

    #[test]
    fn swiped_lists_swipes_newest_first() {
        let all = sample();
        let mut s = swipes(
            &[
                ("ann", Choice::Yes),
                ("bea", Choice::No),
                ("dee", Choice::Yes),
                ("gone", Choice::No),
            ],
            &[],
        );
        for (slug, at) in [
            ("ann", "2026-11-18T09:00:00Z"),
            ("bea", "2026-11-18T10:00:00Z"),
            ("dee", "2026-11-17T23:00:00Z"),
            ("gone", "2026-11-18T11:00:00Z"),
        ] {
            s.creators.get_mut(slug).unwrap().at = at.into();
        }
        let list = swiped_list(Some(&all), &s, vec![]).cards;
        let got: Vec<_> = list
            .iter()
            .map(|c| (c.card.slug.as_str(), c.choice))
            .collect();
        // "gone" isn't in creators.json; "cat" isn't swiped.
        assert_eq!(
            got,
            [
                ("bea", Some(Choice::No)),
                ("ann", Some(Choice::Yes)),
                ("dee", Some(Choice::Yes))
            ]
        );
        assert_eq!(list[0].at.as_deref(), Some("2026-11-18T10:00:00Z"));
        // A no's posts aren't in the Posts deck.
        assert_eq!(list[0].unswiped, 0);
        let missing = swiped_list(None, &s, vec![]);
        assert!(missing.cards.is_empty() && missing.meta.missing);
    }
}
