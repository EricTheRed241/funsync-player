//! Which scenes the library already has (`inLibrary`). They came in three
//! ways, each leaving its own trace in the scene folder:
//!
//! - this scout: a `vrb.json` sidecar with the site and id;
//! - the EroScripts scout: an `eroscripts.json` whose `vrbangers.slug` names
//!   the scene, or whose `video` is the site's file name
//!   (`VRBANGERS_<short name>_<Q>_<projection>.mp4`), in a folder named
//!   `<Performers> - <Title> (VRBangers)_180x180_3dh` (older ones
//!   `VRBangers - <Performer> - <Title>`);
//! - slr-scout: an `slr.json` whose studio is the site and whose title is the
//!   scene's.
//!
//! Folders without a sidecar count by name: a name that mentions the site
//! and has the scene's title as one of its ` - ` parts (bracketed parts such
//! as `(Remastered)` left out on both sides), or a video file named with the
//! site's short name.

use crate::sites::{SITES, Site};
use crate::store::Scene;
use serde_json::Value;
use slr_scout::library::{self, Source};
use slr_scout::text;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub const SIDECAR: &str = "vrb.json";

/// What the library holds, per site key.
#[derive(Debug, Default)]
pub struct Index {
    ids: HashSet<(String, String)>,
    slugs: HashSet<(String, String)>,
    shorts: HashSet<(String, String)>,
    /// Titles (`core` form) by site.
    titles: HashSet<(String, String)>,
}

/// A title without its bracketed parts, normalised: `Wet Adventures
/// (Remastered)` → `wet adventures`.
pub fn core(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut depth = 0u32;
    for c in title.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    text::normalise(&out)
}

/// The site a name mentions (`VRBangers`, `vrbangers`, `VR Bangers`).
fn site_in(name: &str) -> Option<&'static Site> {
    let squashed = text::normalise(name).replace(' ', "");
    SITES
        .iter()
        .find(|s| squashed.contains(&text::normalise(s.name).replace(' ', "")))
}

/// `<PREFIX>_<short>_<Q>K_<projection>[.<axis>].<ext>` → (site, short name).
pub fn short_of(file: &str) -> Option<(&'static Site, String)> {
    let stem = file.split('.').next()?;
    SITES.iter().find_map(|s| {
        let rest = stem
            .get(..s.file_prefix.len() + 1)
            .filter(|p| p.eq_ignore_ascii_case(&format!("{}_", s.file_prefix)))
            .map(|p| &stem[p.len()..])?;
        // Cut at the quality part (`_8K_`), else keep it whole.
        let parts: Vec<&str> = rest.split('_').collect();
        let at = parts
            .iter()
            .position(|p| {
                p.len() >= 2
                    && p.ends_with(['K', 'k'])
                    && p[..p.len() - 1].chars().all(|c| c.is_ascii_digit())
            })
            .unwrap_or(parts.len());
        let short = parts[..at].join("_").to_lowercase();
        (!short.is_empty()).then_some((s, short))
    })
}

impl Index {
    fn add(set: &mut HashSet<(String, String)>, site: &Site, value: &str) {
        let v = value.trim();
        if !v.is_empty() {
            set.insert((site.key.to_string(), v.to_lowercase()));
        }
    }

    /// A folder name: its ` - ` parts as titles when it mentions a site.
    pub fn add_name(&mut self, name: &str) {
        let Some(site) = site_in(name) else { return };
        // `… (VRBangers)_180x180_3dh` → `… (VRBangers)`
        let base = name.split('_').next().unwrap_or(name);
        for part in base.split(" - ") {
            let c = core(part);
            if !c.is_empty() {
                self.titles.insert((site.key.to_string(), c));
            }
        }
    }

    pub fn add_file(&mut self, name: &str) {
        if let Some((site, short)) = short_of(name) {
            Self::add(&mut self.shorts, site, &short);
        }
    }

    /// A sidecar by its file name (`vrb.json`, `eroscripts.json`, `slr.json`).
    pub fn add_sidecar(&mut self, file: &str, v: &Value) {
        match file {
            SIDECAR => {
                if let (Some(site), Some(id)) = (
                    v["site"].as_str().and_then(crate::sites::by_key),
                    crate::api::id_of(&v["id"]),
                ) {
                    Self::add(&mut self.ids, site, &id);
                }
            }
            "eroscripts.json" => {
                if let Some(slug) = v["vrbangers"]["slug"].as_str()
                    && let Some(site) = crate::sites::by_key("vrbangers")
                {
                    Self::add(&mut self.slugs, site, slug);
                }
                if let Some(video) = v["video"].as_str() {
                    self.add_file(video);
                }
            }
            "slr.json" => {
                if let (Some(site), Some(title)) =
                    (v["studio"].as_str().and_then(site_in), v["title"].as_str())
                {
                    let c = core(title);
                    if !c.is_empty() {
                        self.titles.insert((site.key.to_string(), c));
                    }
                }
            }
            _ => {}
        }
    }

    pub fn contains(&self, s: &Scene) -> bool {
        let key = |v: &str| (s.site.clone(), v.trim().to_lowercase());
        self.ids.contains(&key(&s.id))
            || self.slugs.contains(&key(&s.slug))
            || s.short_name
                .as_deref()
                .is_some_and(|n| self.shorts.contains(&key(n)))
            || {
                let c = core(&s.title);
                !c.is_empty() && self.titles.contains(&(s.site.clone(), c))
            }
    }

    /// Walk a folder: `depth` levels of folders below it (artist / scene),
    /// dot names (AppleDouble `._*`) skipped.
    pub fn walk(&mut self, dir: &Path, depth: u32) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let Ok(name) = e.file_name().into_string() else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = e.file_type() else { continue };
            if kind.is_dir() {
                self.add_name(&name);
                if depth > 0 {
                    self.walk(&e.path(), depth - 1);
                }
            } else if matches!(name.as_str(), SIDECAR | "eroscripts.json" | "slr.json") {
                if let Some(v) = fs::read_to_string(e.path())
                    .ok()
                    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                {
                    self.add_sidecar(&name, &v);
                }
            } else if library::is_video_name(&name) {
                self.add_file(&name);
            }
        }
    }
}

/// The enabled sources' scenes (source / artist / scene / files).
pub fn index(sources: &[Source]) -> Index {
    let mut i = Index::default();
    for s in library::enabled(sources) {
        i.walk(&s.path, 2);
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::scene;
    use serde_json::json;

    fn vrb(id: &str, slug: &str, short: &str, title: &str) -> Scene {
        let mut s = scene(id);
        s.slug = slug.into();
        s.short_name = Some(short.into());
        s.title = title.into();
        s
    }

    #[test]
    fn reads_short_names() {
        let (site, short) =
            short_of("VRBANGERS_wet_adventures_remastered_8K_180x180_3dh.mp4").unwrap();
        assert_eq!(
            (site.key, short.as_str()),
            ("vrbangers", "wet_adventures_remastered")
        );
        let (site, short) = short_of("VRCONK_vrcosplayx_the_substance_6K_180x180_3dh.mp4").unwrap();
        assert_eq!(
            (site.key, short.as_str()),
            ("vrconk", "vrcosplayx_the_substance")
        );
        assert_eq!(
            short_of("VRBANGERS_a_scene_toy.funscript").unwrap().1,
            "a_scene_toy"
        );
        assert!(short_of("Riley Reid - X (VRBangers)_180x180_3dh.mp4").is_none());
        assert!(short_of("VRBANGERS_.mp4").is_none());
        assert_eq!(core("Wet Adventures (Remastered)"), "wet adventures");
        assert_eq!(core("V.I.P. [8K]"), "v i p");
    }

    #[test]
    fn matches_every_trace() {
        let mut i = Index::default();
        i.add_sidecar("vrb.json", &json!({"site": "vrconk", "id": "c1"}));
        i.add_sidecar(
            "eroscripts.json",
            &json!({"vrbangers": {"slug": "fully-fashioned"},
            "video": "VRBANGERS_no_time_to_die_hard_6K_180x180_3dh.mp4"}),
        );
        i.add_sidecar(
            "slr.json",
            &json!({"studio": "VR Bangers", "title": "Slr Scene (Remastered)"}),
        );
        i.add_name("Adriana Chechik - Wet Adventures (VRBangers)_180x180_3dh");
        i.add_name("VRBangers - Xxlayna Marie - Super Hole LVI");
        i.add_name("strangewind18 - Angel Wicky - Mistress Angel Wicky - VRBangers");
        i.add_name("Some Artist - Not A Site Folder");
        i.add_file("VRCONK_the_queen_grabs_it_8K_180x180_3dh.mp4");

        let mut c = vrb("c1", "x", "x", "X");
        c.site = "vrconk".into();
        assert!(i.contains(&c), "vrb.json id");
        assert!(!i.contains(&vrb("c1", "x", "x", "X")), "the id is VRConk's");
        assert!(i.contains(&vrb("1", "fully-fashioned", "f", "Fully Fashioned")));
        assert!(i.contains(&vrb("2", "no-time", "no_time_to_die_hard", "Other Title")));
        assert!(i.contains(&vrb("3", "s", "s", "Slr Scene")));
        assert!(i.contains(&vrb("4", "w", "w", "Wet Adventures (Remastered)")));
        assert!(i.contains(&vrb("5", "h", "h", "Super Hole LVI")));
        assert!(i.contains(&vrb("6", "m", "m", "Mistress Angel Wicky")));
        let mut q = vrb("7", "q", "the_queen_grabs_it", "Q");
        q.site = "vrconk".into();
        assert!(i.contains(&q));
        assert!(!i.contains(&vrb("8", "n", "n", "Not A Site Folder")));
        // Only whole parts: a word of a title isn't the title.
        assert!(!i.contains(&vrb("9", "w2", "w2", "Wet")));
    }

    #[test]
    fn walks_the_library() {
        let dir = std::env::temp_dir().join(format!("vrb-scout-library-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let scene_dir = dir.join("Other/Angel Youngs/Angel Youngs - Wet (VRBangers)_180x180_3dh");
        fs::create_dir_all(&scene_dir).unwrap();
        fs::write(
            scene_dir.join("vrb.json"),
            r#"{"site": "vrbangers", "id": "a1"}"#,
        )
        .unwrap();
        fs::write(scene_dir.join("._vrb.json"), "junk").unwrap();
        let raw = dir.join("Other/VRBANGERS_raw_file_8K_180x180_3dh.mp4");
        fs::write(&raw, "").unwrap();
        let sources = vec![
            Source {
                name: Some("Other".into()),
                path: dir.join("Other"),
                enabled: true,
            },
            Source {
                name: None,
                path: dir.join("missing"),
                enabled: true,
            },
        ];
        let i = index(&sources);
        assert!(i.contains(&vrb("a1", "s", "s", "T")));
        assert!(i.contains(&vrb("b", "s", "s", "Wet")));
        assert!(i.contains(&vrb("c", "s", "raw_file", "T")));
        let _ = fs::remove_dir_all(&dir);
    }
}
