//! Which videos need screening, and what a judged `p_minor` means.

use crate::backend::Video;
use crate::prefs::Prefs;
use crate::state::{Action, StateFile};
use std::collections::HashSet;

/// A video to screen and the file size it has now.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub video: Video,
    pub size: u64,
}

/// Why a listed video isn't screened this run.
#[derive(Debug, Default, PartialEq)]
pub struct Skipped {
    /// Screened before, same size.
    pub screened: usize,
    /// AppleDouble `._*` sidecars.
    pub apple_double: usize,
    /// Not on disk (volume unplugged, deleted since the scan).
    pub missing: usize,
    /// Rated at or below the delete level: the app deletes them anyway.
    pub rated_out: usize,
}

/// Videos never screened or whose file size changed since, in list order.
/// `size_of` gives the file's size, `None` when it doesn't exist.
pub fn select(
    videos: &[Video],
    state: &StateFile,
    prefs: &Prefs,
    size_of: impl Fn(&str) -> Option<u64>,
) -> (Vec<Candidate>, Skipped) {
    let mut out = Vec::new();
    let mut skipped = Skipped::default();
    let mut seen = HashSet::new();
    for v in videos {
        if !seen.insert(v.path.as_str()) {
            continue;
        }
        let name = v.path.rsplit('/').next().unwrap_or(&v.path);
        if name.starts_with("._") {
            skipped.apple_double += 1;
            continue;
        }
        if v.rating > 0.0 && v.rating <= prefs.ratings.delete_at_or_below {
            skipped.rated_out += 1;
            continue;
        }
        let Some(size) = size_of(&v.path) else {
            skipped.missing += 1;
            continue;
        };
        if state.videos.get(&v.path).is_some_and(|e| e.size == size) {
            skipped.screened += 1;
            continue;
        }
        out.push(Candidate {
            video: v.clone(),
            size,
        });
    }
    (out, skipped)
}

/// What to do with a judged video.
pub fn decide(p_minor: f64, favorite: bool, prefs: &Prefs) -> Action {
    let a = &prefs.age_screen;
    if p_minor >= a.purge_at {
        if favorite {
            Action::ReviewFavourite
        } else {
            Action::Purged
        }
    } else if p_minor >= a.review_at {
        Action::Review
    } else {
        Action::Clear
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Entry;
    use serde_json::{Map, json};
    use std::collections::HashMap;

    fn prefs() -> Prefs {
        crate::prefs::parse(
            r#"{"ageScreen": {"purgeAt": 0.8, "reviewAt": 0.5, "playlist": "Underage review", "intervalDays": 7},
                "ratings": {"deleteAtOrBelow": 2}}"#,
        )
        .unwrap()
    }

    fn video(id: &str, path: &str, rating: f64) -> Video {
        Video {
            id: json!(id),
            path: path.into(),
            favorite: false,
            rating,
            duration: 60.0,
        }
    }

    fn screened(size: u64) -> Entry {
        Entry {
            size,
            screened_at: "2026-11-04T12:00:00.000Z".into(),
            p_minor: 0.1,
            reason: String::new(),
            action: Action::Clear,
            extra: Map::new(),
        }
    }

    #[test]
    fn selects_new_and_resized_videos_only() {
        let videos = vec![
            video("1", "/V/new.mp4", 0.0),
            video("2", "/V/same.mp4", 0.0),
            video("3", "/V/resized.mp4", 0.0),
            video("4", "/V/._new.mp4", 0.0),
            video("5", "/V/gone.mp4", 0.0),
            video("6", "/V/one-star.mp4", 1.0),
            video("7", "/V/two-stars.mp4", 2.0),
            video("8", "/V/two-and-a-half.mp4", 2.5),
            video("1", "/V/new.mp4", 0.0),
        ];
        let sizes: HashMap<&str, u64> = [
            ("/V/new.mp4", 10),
            ("/V/same.mp4", 20),
            ("/V/resized.mp4", 31),
            ("/V/._new.mp4", 4),
            ("/V/one-star.mp4", 5),
            ("/V/two-stars.mp4", 5),
            ("/V/two-and-a-half.mp4", 5),
        ]
        .into();
        let mut state = StateFile::empty();
        state.videos.insert("/V/same.mp4".into(), screened(20));
        state.videos.insert("/V/resized.mp4".into(), screened(30));
        let (picked, skipped) = select(&videos, &state, &prefs(), |p| sizes.get(p).copied());
        let paths: Vec<(&str, u64)> = picked
            .iter()
            .map(|c| (c.video.path.as_str(), c.size))
            .collect();
        assert_eq!(
            paths,
            vec![
                ("/V/new.mp4", 10),
                ("/V/resized.mp4", 31),
                ("/V/two-and-a-half.mp4", 5)
            ]
        );
        assert_eq!(
            skipped,
            Skipped {
                screened: 1,
                apple_double: 1,
                missing: 1,
                rated_out: 2
            }
        );
    }

    #[test]
    fn thresholds_and_favourites() {
        let p = prefs();
        assert_eq!(decide(0.8, false, &p), Action::Purged);
        assert_eq!(decide(1.0, false, &p), Action::Purged);
        assert_eq!(decide(0.8, true, &p), Action::ReviewFavourite);
        assert_eq!(decide(0.7999, false, &p), Action::Review);
        assert_eq!(decide(0.7999, true, &p), Action::Review);
        assert_eq!(decide(0.5, false, &p), Action::Review);
        assert_eq!(decide(0.4999, false, &p), Action::Clear);
        assert_eq!(decide(0.4999, true, &p), Action::Clear);
        assert_eq!(decide(0.0, false, &p), Action::Clear);
    }
}
