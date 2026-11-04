//! Fold validated proposals and removals into the tag file, reporting what
//! changes per video. Pure: no IO.

use crate::store::{Entry, TagFile};
use serde_json::Map;
use unicode_normalization::UnicodeNormalization;

/// How proposed tags combine with an entry's current ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Current tags plus the proposed ones it lacks.
    Union,
    /// Exactly the proposed tags; an empty list removes the entry.
    Replace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    New {
        tags: Vec<String>,
    },
    Changed {
        added: Vec<String>,
        dropped: Vec<String>,
        kept: Vec<String>,
    },
    Unchanged {
        tags: Vec<String>,
    },
    Removed {
        tags: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub path: String,
    pub change: Change,
}

impl Outcome {
    pub fn is_change(&self) -> bool {
        !matches!(self.change, Change::Unchanged { .. })
    }
}

/// A tag slug: `^[a-z0-9][a-z0-9-]*$`.
pub fn is_slug(tag: &str) -> bool {
    let mut bytes = tag.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'0'..=b'9'))
        && bytes.all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'))
}

/// Trim and lower-case each tag, drop repeats, keep the order. Fails with
/// the tags that aren't slugs.
pub fn normalize_tags(raw: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut bad = Vec::new();
    for tag in raw {
        let t = tag.trim().to_lowercase();
        if !is_slug(&t) {
            bad.push(format!("\"{tag}\""));
        } else if !out.contains(&t) {
            out.push(t);
        }
    }
    if bad.is_empty() {
        Ok(out)
    } else {
        Err(format!(
            "invalid tag {} (slugs: a-z, 0-9, '-')",
            bad.join(", ")
        ))
    }
}

/// A validated proposal: on-disk path, normalised tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Valid {
    pub path: String,
    pub tags: Vec<String>,
    pub by: Option<String>,
}

/// Apply proposals in order. `now` stamps entries whose tags change.
pub fn apply_proposals(
    file: &mut TagFile,
    proposals: &[Valid],
    mode: Mode,
    now: &str,
) -> Vec<Outcome> {
    proposals
        .iter()
        .map(|p| {
            let change = apply_one(file, p, mode, now);
            Outcome {
                path: p.path.clone(),
                change,
            }
        })
        .collect()
}

fn apply_one(file: &mut TagFile, p: &Valid, mode: Mode, now: &str) -> Change {
    let Some(entry) = file.videos.get_mut(&p.path) else {
        if p.tags.is_empty() {
            return Change::Unchanged { tags: Vec::new() };
        }
        file.videos.insert(
            p.path.clone(),
            Entry {
                tags: p.tags.clone(),
                added_at: now.to_string(),
                by: p.by.clone(),
                extra: Map::new(),
            },
        );
        return Change::New {
            tags: p.tags.clone(),
        };
    };
    let added: Vec<String> = p
        .tags
        .iter()
        .filter(|t| !entry.tags.contains(t))
        .cloned()
        .collect();
    let (dropped, kept): (Vec<String>, Vec<String>) = match mode {
        Mode::Union => (Vec::new(), entry.tags.clone()),
        Mode::Replace => entry
            .tags
            .iter()
            .cloned()
            .partition(|t| !p.tags.contains(t)),
    };
    if added.is_empty() && dropped.is_empty() {
        return Change::Unchanged {
            tags: entry.tags.clone(),
        };
    }
    if mode == Mode::Replace && p.tags.is_empty() {
        let removed = file
            .videos
            .remove(&p.path)
            .map(|e| e.tags)
            .unwrap_or_default();
        return Change::Removed { tags: removed };
    }
    match mode {
        Mode::Union => entry.tags.extend(added.iter().cloned()),
        Mode::Replace => entry.tags = p.tags.clone(),
    }
    entry.added_at = now.to_string();
    if p.by.is_some() {
        entry.by = p.by.clone();
    }
    Change::Changed {
        added,
        dropped,
        kept,
    }
}

/// Find the stored key for a path: the exact string, else the one key that
/// is the same after NFC normalisation (the video may be gone from disk, so
/// the filesystem can't be asked).
pub fn find_key(file: &TagFile, path: &str) -> Option<String> {
    if file.videos.contains_key(path) {
        return Some(path.to_string());
    }
    let want: String = path.nfc().collect();
    let mut hits = file.videos.keys().filter(|k| k.nfc().eq(want.chars()));
    match (hits.next(), hits.next()) {
        (Some(k), None) => Some(k.clone()),
        _ => None,
    }
}

/// Remove entries. Fails with the paths that aren't in the file.
pub fn remove_paths(file: &mut TagFile, paths: &[String]) -> Result<Vec<Outcome>, Vec<String>> {
    let keys: Vec<Result<String, String>> = paths
        .iter()
        .map(|p| find_key(file, p).ok_or_else(|| p.clone()))
        .collect();
    let missing: Vec<String> = keys
        .iter()
        .filter_map(|k| k.as_ref().err().cloned())
        .collect();
    if !missing.is_empty() {
        return Err(missing);
    }
    Ok(keys
        .into_iter()
        .flatten()
        .filter_map(|key| {
            let entry = file.videos.remove(&key)?;
            Some(Outcome {
                path: key,
                change: Change::Removed { tags: entry.tags },
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-11-04T12:00:00.000Z";
    const LATER: &str = "2026-11-05T12:00:00.000Z";

    fn tags(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn valid(path: &str, list: &[&str]) -> Valid {
        Valid {
            path: path.into(),
            tags: tags(list),
            by: None,
        }
    }

    #[test]
    fn slugs() {
        for ok in ["pov", "vr", "anal-creampie", "4k", "a"] {
            assert!(is_slug(ok), "{ok}");
        }
        for bad in ["", "-pov", "Pov", "po v", "pov_x", "é", "pov!"] {
            assert!(!is_slug(bad), "{bad}");
        }
        assert_eq!(
            normalize_tags(&tags(&[" POV ", "pov", "Riding"])),
            Ok(tags(&["pov", "riding"]))
        );
        assert!(
            normalize_tags(&tags(&["pov", "big tits"]))
                .unwrap_err()
                .contains("\"big tits\"")
        );
    }

    #[test]
    fn union_adds_missing_tags_after_the_current_ones() {
        let mut f = TagFile::empty();
        let out = apply_proposals(&mut f, &[valid("/a.mp4", &["pov"])], Mode::Union, NOW);
        assert_eq!(
            out[0].change,
            Change::New {
                tags: tags(&["pov"])
            }
        );
        let out = apply_proposals(
            &mut f,
            &[valid("/a.mp4", &["creampie", "pov"])],
            Mode::Union,
            LATER,
        );
        assert_eq!(
            out[0].change,
            Change::Changed {
                added: tags(&["creampie"]),
                dropped: vec![],
                kept: tags(&["pov"])
            }
        );
        assert_eq!(f.videos["/a.mp4"].tags, tags(&["pov", "creampie"]));
        assert_eq!(f.videos["/a.mp4"].added_at, LATER);
        let out = apply_proposals(&mut f, &[valid("/a.mp4", &["pov"])], Mode::Union, LATER);
        assert!(!out[0].is_change());
    }

    #[test]
    fn replace_sets_exactly_and_empty_removes() {
        let mut f = TagFile::empty();
        apply_proposals(&mut f, &[valid("/a.mp4", &["pov", "vr"])], Mode::Union, NOW);
        let out = apply_proposals(
            &mut f,
            &[valid("/a.mp4", &["riding", "pov"])],
            Mode::Replace,
            LATER,
        );
        assert_eq!(
            out[0].change,
            Change::Changed {
                added: tags(&["riding"]),
                dropped: tags(&["vr"]),
                kept: tags(&["pov"])
            }
        );
        assert_eq!(f.videos["/a.mp4"].tags, tags(&["riding", "pov"]));
        // Same set, other order: nothing to do.
        let out = apply_proposals(
            &mut f,
            &[valid("/a.mp4", &["pov", "riding"])],
            Mode::Replace,
            LATER,
        );
        assert!(!out[0].is_change());
        let out = apply_proposals(&mut f, &[valid("/a.mp4", &[])], Mode::Replace, LATER);
        assert_eq!(
            out[0].change,
            Change::Removed {
                tags: tags(&["riding", "pov"])
            }
        );
        assert!(f.videos.is_empty());
        // Nothing proposed for an unknown video: nothing to do.
        let out = apply_proposals(&mut f, &[valid("/b.mp4", &[])], Mode::Replace, LATER);
        assert!(!out[0].is_change());
    }

    #[test]
    fn removal_by_exact_or_equivalent_key() {
        let mut f = TagFile::empty();
        apply_proposals(
            &mut f,
            &[
                valid("/Rene\u{301}e.mp4", &["pov"]),
                valid("/b.mp4", &["vr"]),
            ],
            Mode::Union,
            NOW,
        );
        assert_eq!(
            remove_paths(&mut f, &["/nope.mp4".into(), "/b.mp4".into()]),
            Err(vec!["/nope.mp4".to_string()])
        );
        assert_eq!(
            f.videos.len(),
            2,
            "nothing removed when one path is unknown"
        );
        let out = remove_paths(&mut f, &["/Ren\u{e9}e.mp4".into()]).unwrap();
        assert_eq!(
            out,
            vec![Outcome {
                path: "/Rene\u{301}e.mp4".into(),
                change: Change::Removed {
                    tags: tags(&["pov"])
                }
            }]
        );
        assert_eq!(f.videos.keys().collect::<Vec<_>>(), vec!["/b.mp4"]);
    }

    #[test]
    fn keeps_by_and_unknown_fields() {
        let mut f = TagFile::empty();
        let p = Valid {
            path: "/a.mp4".into(),
            tags: tags(&["pov"]),
            by: Some("tagger".into()),
        };
        apply_proposals(&mut f, &[p], Mode::Union, NOW);
        f.videos
            .get_mut("/a.mp4")
            .unwrap()
            .extra
            .insert("note".into(), "x".into());
        apply_proposals(&mut f, &[valid("/a.mp4", &["vr"])], Mode::Union, LATER);
        let e = &f.videos["/a.mp4"];
        assert_eq!(e.by.as_deref(), Some("tagger"));
        assert_eq!(e.extra["note"], "x");
    }
}
