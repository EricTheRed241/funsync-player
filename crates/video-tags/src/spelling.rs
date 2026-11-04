//! Resolve a path to the filesystem's own spelling.
//!
//! Playlists and the headset match scan paths byte for byte, and names from
//! MEGA can arrive in another Unicode form (NFC) than the volume lists (NFD).
//! APFS and exFAT find a file under either form, so `stat` can't tell which
//! one is on disk; each segment is looked up in its directory's listing
//! instead: an exact byte match first, else the one name that is the same
//! after NFC normalisation.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Component, Path, PathBuf};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::{is_nfc, is_nfd};

/// How a segment was found in its directory.
#[derive(Debug, PartialEq, Eq)]
pub enum SegmentMatch<'a> {
    /// The same bytes are on disk.
    Exact,
    /// Only this differently spelled, Unicode-equivalent name is on disk.
    Equivalent(&'a OsStr),
    /// Several equivalent names and no exact one.
    Ambiguous,
    Missing,
}

/// Find `wanted` among a directory's `names`.
pub fn match_segment<'a>(wanted: &OsStr, names: &'a [OsString]) -> SegmentMatch<'a> {
    if names.iter().any(|n| n == wanted) {
        return SegmentMatch::Exact;
    }
    let Some(wanted) = wanted.to_str() else {
        return SegmentMatch::Missing;
    };
    let wanted_nfc: String = wanted.nfc().collect();
    let mut found = None;
    for name in names {
        let Some(s) = name.to_str() else { continue };
        if s.nfc().eq(wanted_nfc.chars()) {
            if found.is_some() {
                return SegmentMatch::Ambiguous;
            }
            found = Some(name.as_os_str());
        }
    }
    found.map_or(SegmentMatch::Missing, SegmentMatch::Equivalent)
}

/// A segment stored with another spelling than proposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Respelled {
    pub proposed: String,
    pub on_disk: String,
}

impl Respelled {
    /// `"Ｘ" → on-disk NFD spelling (proposed NFC)`.
    pub fn describe(&self) -> String {
        format!(
            "\"{}\" stored in the on-disk spelling ({}; proposed {})",
            self.on_disk,
            form(&self.on_disk),
            form(&self.proposed)
        )
    }
}

fn form(s: &str) -> &'static str {
    match (is_nfc(s), is_nfd(s)) {
        (true, true) => "NFC = NFD",
        (true, false) => "NFC",
        (false, true) => "NFD",
        (false, false) => "mixed form",
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Resolved {
    pub path: PathBuf,
    pub respelled: Vec<Respelled>,
}

/// Resolves paths segment by segment, caching each directory listing.
#[derive(Default)]
pub struct Resolver {
    listings: HashMap<PathBuf, Vec<OsString>>,
}

impl Resolver {
    pub fn new() -> Self {
        Self::default()
    }

    fn listing(&mut self, dir: &Path) -> Result<&[OsString], String> {
        if !self.listings.contains_key(dir) {
            let names = fs::read_dir(dir)
                .and_then(|it| {
                    it.map(|e| e.map(|e| e.file_name()))
                        .collect::<Result<Vec<_>, _>>()
                })
                .map_err(|e| format!("can't list {}: {e}", dir.display()))?;
            self.listings.insert(dir.to_path_buf(), names);
        }
        Ok(&self.listings[dir])
    }

    /// The path as it is spelled on disk. Fails when it isn't absolute, has
    /// a `..`, or a segment doesn't exist (or exists in more than one form).
    pub fn resolve(&mut self, path: &Path) -> Result<Resolved, String> {
        if !path.is_absolute() {
            return Err("not an absolute path".into());
        }
        let mut out = PathBuf::new();
        let mut respelled = Vec::new();
        for comp in path.components() {
            match comp {
                Component::Prefix(_) | Component::RootDir => out.push(comp.as_os_str()),
                Component::CurDir => {}
                Component::ParentDir => return Err("contains '..'".into()),
                Component::Normal(seg) => {
                    let name = match match_segment(seg, self.listing(&out)?) {
                        SegmentMatch::Exact => seg.to_os_string(),
                        SegmentMatch::Equivalent(on_disk) => {
                            respelled.push(Respelled {
                                proposed: seg.to_string_lossy().into_owned(),
                                on_disk: on_disk.to_string_lossy().into_owned(),
                            });
                            on_disk.to_os_string()
                        }
                        SegmentMatch::Ambiguous => {
                            return Err(format!(
                                "\"{}\" exists in more than one Unicode form in {}",
                                seg.to_string_lossy(),
                                out.display()
                            ));
                        }
                        SegmentMatch::Missing => {
                            return Err(format!(
                                "no \"{}\" in {}",
                                seg.to_string_lossy(),
                                out.display()
                            ));
                        }
                    };
                    out.push(name);
                }
            }
        }
        Ok(Resolved {
            path: out,
            respelled,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NFC: &str = "Ren\u{e9}e"; // é precomposed
    const NFD: &str = "Rene\u{301}e"; // e + combining acute

    fn names(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn exact_bytes_win() {
        let dir = names(&[NFD, NFC]);
        assert_eq!(match_segment(OsStr::new(NFC), &dir), SegmentMatch::Exact);
    }

    #[test]
    fn nfc_proposal_finds_the_nfd_name_on_disk() {
        let dir = names(&["other", NFD]);
        assert_eq!(
            match_segment(OsStr::new(NFC), &dir),
            SegmentMatch::Equivalent(OsStr::new(NFD))
        );
        let dir = names(&[NFC]);
        assert_eq!(
            match_segment(OsStr::new(NFD), &dir),
            SegmentMatch::Equivalent(OsStr::new(NFC))
        );
        // Two non-NFC spellings of the same name (combining marks in either
        // order) and no exact one: refuse to guess.
        let dir = names(&["a\u{323}\u{301}", "a\u{301}\u{323}"]);
        assert_eq!(
            match_segment(OsStr::new("\u{1ea1}\u{301}"), &dir),
            SegmentMatch::Ambiguous
        );
    }

    #[test]
    fn missing_and_ambiguous() {
        assert_eq!(
            match_segment(OsStr::new("x"), &names(&["y"])),
            SegmentMatch::Missing
        );
        // No case folding: byte or Unicode-equivalent only.
        assert_eq!(
            match_segment(OsStr::new("Scene"), &names(&["scene"])),
            SegmentMatch::Missing
        );
        let third = "Rene\u{301}e\u{200b}";
        assert_eq!(
            match_segment(OsStr::new(NFC), &names(&[third])),
            SegmentMatch::Missing
        );
    }

    #[test]
    fn describes_the_forms() {
        let r = Respelled {
            proposed: NFC.into(),
            on_disk: NFD.into(),
        };
        assert_eq!(
            r.describe(),
            format!("\"{NFD}\" stored in the on-disk spelling (NFD; proposed NFC)")
        );
    }
}
