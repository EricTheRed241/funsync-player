//! What never comes in, for the Rust scouts (`vrb-scout`, `pmvhaven-scout`):
//! the `screening` values of content-preferences.json, applied the way the JS
//! scouts apply EXCLUDED_TEXT, BLOCKED_CREATORS, BLOCKED_TAGS, BLOCKED_TITLE,
//! FURRY_TAGS, SFW_TAGS and MIN_VIDEO_MS (shared/content-preferences.js).
//!
//! Words match whole words of the folded text (`text::normalise`: ASCII, lower
//! case, anything else a space), so a hyphenated tag (`futa-on-male`) matches
//! `futa` and `Futaba` doesn't. Tags match exactly (trimmed, lower case).
//! Creators match with case, spaces and punctuation ignored, as a performer /
//! uploader / studio or inside the title.

use crate::prefs::Screening;
use crate::score::tag_key;
use crate::text;
use std::collections::HashSet;
use std::fmt;

/// Why an item is turned down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// An excluded word (loli, futa, furry …) in the title, a tag or the text.
    Excluded(String),
    BlockedCreator(String),
    BlockedTag(String),
    /// A blocked word (mmd, dildo) in the title.
    BlockedTitle(String),
    /// A tag that is furry without saying so.
    Furry(String),
    /// A tag saying there is no porn in it.
    Sfw(String),
    /// Shorter than `minVideoMs` (the length in ms).
    TooShort(u64),
}

impl Reason {
    /// The verdict's short name, as the JS scouts record it.
    pub fn key(&self) -> &'static str {
        match self {
            Reason::Excluded(_) | Reason::Furry(_) => "excluded",
            Reason::BlockedCreator(_) | Reason::BlockedTag(_) | Reason::BlockedTitle(_) => {
                "blocked"
            }
            Reason::Sfw(_) => "sfw",
            Reason::TooShort(_) => "too-short",
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::Excluded(w) => write!(f, "excluded ({w})"),
            Reason::BlockedCreator(c) => write!(f, "blocked creator ({c})"),
            Reason::BlockedTag(t) => write!(f, "blocked tag ({t})"),
            Reason::BlockedTitle(w) => write!(f, "blocked title word ({w})"),
            Reason::Furry(t) => write!(f, "furry tag ({t})"),
            Reason::Sfw(t) => write!(f, "not porn ({t})"),
            Reason::TooShort(ms) => write!(f, "too short ({} s)", ms / 1000),
        }
    }
}

/// What the screen looks at.
#[derive(Debug, Clone, Copy, Default)]
pub struct Item<'a> {
    pub title: &'a str,
    pub tags: &'a [String],
    /// A description or first post (may be empty).
    pub text: &'a str,
    /// Performers, uploader, studio.
    pub creators: &'a [String],
    /// The video's length, when known.
    pub duration_ms: Option<u64>,
}

/// Lower case letters and digits only: `Aiden Het` → `aidenhet`.
fn squash(s: &str) -> String {
    text::normalise(s).replace(' ', "")
}

/// The first reason `item` is turned down, if any.
pub fn screen(s: &Screening, item: &Item) -> Option<Reason> {
    let tags: Vec<String> = item.tags.iter().map(|t| tag_key(t)).collect();
    let title = text::normalise(item.title);
    let title_words: HashSet<&str> = text::words(&title).collect();
    let mut all = text::normalise(&format!(
        "{} {} {}",
        item.title,
        item.tags.join(" "),
        item.text
    ));
    all.push(' ');
    let all_words: HashSet<&str> = text::words(&all).collect();
    if let Some(w) = s
        .excluded_words
        .iter()
        .find(|w| all_words.contains(text::normalise(w).as_str()))
    {
        return Some(Reason::Excluded(w.clone()));
    }
    let squashed_title = squash(item.title);
    for b in &s.blocked_creators {
        let key = squash(b);
        if !key.is_empty()
            && (item.creators.iter().any(|c| squash(c) == key) || squashed_title.contains(&key))
        {
            return Some(Reason::BlockedCreator(b.clone()));
        }
    }
    let has = |list: &[String]| {
        list.iter()
            .find(|t| tags.contains(&tag_key(t)))
            .map(|t| tag_key(t))
    };
    if let Some(t) = has(&s.blocked_tags) {
        return Some(Reason::BlockedTag(t));
    }
    if let Some(w) = s
        .blocked_title_words
        .iter()
        .find(|w| title_words.contains(text::normalise(w).as_str()))
    {
        return Some(Reason::BlockedTitle(w.clone()));
    }
    if let Some(t) = has(&s.furry_tags) {
        return Some(Reason::Furry(t));
    }
    if let Some(t) = has(&s.sfw_tags) {
        return Some(Reason::Sfw(t));
    }
    match item.duration_ms {
        Some(ms) if ms < s.min_video_ms => Some(Reason::TooShort(ms)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screening() -> Screening {
        crate::prefs::load().unwrap().screening
    }

    fn tags(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn excluded_words_match_whole_words_in_title_tags_and_text() {
        let s = screening();
        let t = tags(&["futa-on-male"]);
        let r = screen(
            &s,
            &Item {
                title: "Night out",
                tags: &t,
                ..Item::default()
            },
        );
        assert_eq!(r, Some(Reason::Excluded("futa".into())));
        let r = screen(
            &s,
            &Item {
                title: "Futaba Sakura PMV",
                ..Item::default()
            },
        );
        assert_eq!(r, None);
        let r = screen(
            &s,
            &Item {
                title: "PMV",
                text: "a Loli edit",
                ..Item::default()
            },
        );
        assert_eq!(r.map(|r| r.key()), Some("excluded"));
    }

    #[test]
    fn blocked_creators_tags_and_title_words() {
        let s = screening();
        let creators = tags(&["Aiden Het"]);
        let r = screen(
            &s,
            &Item {
                title: "x",
                creators: &creators,
                ..Item::default()
            },
        );
        assert_eq!(r, Some(Reason::BlockedCreator("AidenHet".into())));
        let r = screen(
            &s,
            &Item {
                title: "[AidenHet] Tifa",
                ..Item::default()
            },
        );
        assert_eq!(r.map(|r| r.key()), Some("blocked"));
        let t = tags(&["MMD"]);
        assert_eq!(
            screen(
                &s,
                &Item {
                    title: "x",
                    tags: &t,
                    ..Item::default()
                }
            ),
            Some(Reason::BlockedTag("mmd".into()))
        );
        assert_eq!(
            screen(
                &s,
                &Item {
                    title: "Dildo riding compilation",
                    ..Item::default()
                }
            ),
            Some(Reason::BlockedTitle("dildo".into()))
        );
        assert_eq!(
            screen(
                &s,
                &Item {
                    title: "Riding compilation",
                    ..Item::default()
                }
            ),
            None
        );
    }

    #[test]
    fn furry_sfw_and_length() {
        let s = screening();
        let t = tags(&["animal-crossing"]);
        assert_eq!(
            screen(
                &s,
                &Item {
                    title: "x",
                    tags: &t,
                    ..Item::default()
                }
            ),
            Some(Reason::Furry("animal-crossing".into()))
        );
        let t = tags(&["SFW"]);
        assert_eq!(
            screen(
                &s,
                &Item {
                    title: "x",
                    tags: &t,
                    ..Item::default()
                }
            )
            .map(|r| r.key()),
            Some("sfw")
        );
        let short = Some(s.min_video_ms - 1);
        assert_eq!(
            screen(
                &s,
                &Item {
                    title: "x",
                    duration_ms: short,
                    ..Item::default()
                }
            ),
            Some(Reason::TooShort(s.min_video_ms - 1))
        );
        let ok = Some(s.min_video_ms);
        assert_eq!(
            screen(
                &s,
                &Item {
                    title: "x",
                    duration_ms: ok,
                    ..Item::default()
                }
            ),
            None
        );
    }
}
