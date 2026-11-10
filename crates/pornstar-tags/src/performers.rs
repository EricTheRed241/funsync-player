//! The performers a run looks for: the listed ones (preferences) plus those
//! the VRBangers scout named in its scene folders
//! (`<Performers> - <Title> (VRBangers)[_180x180_3dh]`).

use crate::prefs::Listed;
use crate::text::{self, Phrase};

#[derive(Debug, Clone, PartialEq)]
pub struct Performer {
    pub tag: String,
    pub name: String,
    /// Name, aliases and scene phrases; any one identifies her.
    pub phrases: Vec<Phrase>,
    /// Found in a VRBangers scene folder name, not in the list.
    pub discovered: bool,
}

impl Performer {
    pub fn listed(p: &Listed) -> Performer {
        let phrases = std::iter::once(&p.name)
            .chain(&p.aliases)
            .chain(&p.scenes)
            .filter_map(|s| Phrase::new(s))
            .collect();
        Performer {
            tag: p.tag(),
            name: p.name.clone(),
            phrases,
            discovered: false,
        }
    }

    /// Whether `name` is one of her names (name or alias, normalised), or
    /// makes her tag.
    pub fn is_named(&self, name: &str) -> bool {
        let Some(wanted) = Phrase::new(name) else {
            return false;
        };
        self.phrases.contains(&wanted) || text::slug(name) == self.tag
    }
}

/// The performers of a VRBangers scout scene folder: the part before the
/// first ` - `, split on `,`, `&` and ` and `. Empty for any other folder.
pub fn vrbangers_performers(folder: &str) -> Vec<String> {
    const MARK: &str = " (vrbangers)";
    let lower = folder.to_ascii_lowercase();
    let Some(at) = lower.rfind(MARK) else {
        return Vec::new();
    };
    let tail = &folder[at + MARK.len()..];
    let suffix_ok = tail.is_empty()
        || (tail.starts_with('_') && tail.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    if !suffix_ok {
        return Vec::new();
    }
    let Some((who, title)) = folder[..at].split_once(" - ") else {
        return Vec::new();
    };
    if title.trim().is_empty() {
        return Vec::new();
    }
    let mut names: Vec<String> = Vec::new();
    for part in who.split([',', '&']) {
        for name in split_and(part) {
            let name = name.trim();
            if !name.is_empty() && !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// Split on the word ` and ` (any case).
fn split_and(s: &str) -> Vec<&str> {
    let lower = s.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(i) = lower[start..].find(" and ") {
        out.push(&s[start..start + i]);
        start += i + " and ".len();
    }
    out.push(&s[start..]);
    out
}

/// The listed performers, then each discovered name not already listed
/// (by name, alias or tag), in the order given.
pub fn all(listed: &[Listed], discovered: &[String]) -> Vec<Performer> {
    let mut out: Vec<Performer> = listed.iter().map(Performer::listed).collect();
    for name in discovered {
        let tag = text::slug(name);
        if tag.is_empty() || out.iter().any(|p| p.tag == tag || p.is_named(name)) {
            continue;
        }
        out.push(Performer {
            tag,
            name: name.clone(),
            phrases: Phrase::new(name).into_iter().collect(),
            discovered: true,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(name: &str, aliases: &[&str]) -> Listed {
        Listed {
            name: name.into(),
            tag: None,
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            scenes: Vec::new(),
        }
    }

    #[test]
    fn reads_vrbangers_scene_folders() {
        assert_eq!(
            vrbangers_performers("Riley Reid - Anal Virginity (VRBangers)_180x180_3dh"),
            ["Riley Reid"]
        );
        assert_eq!(
            vrbangers_performers("Mia Malkova, Riley Reid - One Last Bang (VRBangers)_180x180_3dh"),
            ["Mia Malkova", "Riley Reid"]
        );
        assert_eq!(
            vrbangers_performers("Ann & Bea and Cleo - A - B (VRBangers)"),
            ["Ann", "Bea", "Cleo"]
        );
        assert!(vrbangers_performers("Riley Reid - Anal Virginity").is_empty());
        assert!(vrbangers_performers("VRBangers - Xxlayna Marie - Super Hole LVI").is_empty());
        assert!(
            vrbangers_performers("strangewind18 - Angel Wicky - Mistress Angel Wicky - VRBangers")
                .is_empty()
        );
        assert!(vrbangers_performers("Solo (VRBangers)").is_empty());
        assert!(vrbangers_performers("A - B (VRBangers) extra").is_empty());
        // A name containing "and" as part of a word stays whole.
        assert_eq!(
            vrbangers_performers("Sandy Andrews - X (VRBangers)"),
            ["Sandy Andrews"]
        );
    }

    #[test]
    fn discovered_names_join_unless_listed() {
        let list = [
            listed("Riley Reid", &[]),
            listed("NoFaceGirl", &["No Face Girl"]),
        ];
        let all = all(
            &list,
            &[
                "Riley Reid".into(),
                "No Face Girl".into(),
                "Jane Doe".into(),
                "jane doe".into(),
            ],
        );
        let tags: Vec<_> = all.iter().map(|p| (p.tag.as_str(), p.discovered)).collect();
        assert_eq!(
            tags,
            [
                ("riley-reid", false),
                ("nofacegirl", false),
                ("jane-doe", true)
            ]
        );
    }

    #[test]
    fn aliases_and_scene_phrases_name_her() {
        let mut vanna = listed("Vanna Bardot", &[]);
        vanna.scenes.push("VRBangers - A Gentlemans Wish".into());
        let p = Performer::listed(&vanna);
        assert_eq!(p.phrases.len(), 2);
        assert!(p.is_named("vanna  bardot"));
        let alice = Performer::listed(&Listed {
            name: "MissAliceNZ".into(),
            tag: Some("alicenz".into()),
            aliases: vec!["AliceNZ".into()],
            scenes: Vec::new(),
        });
        assert!(alice.is_named("AliceNZ") && alice.is_named("missalicenz"));
        assert!(!alice.is_named("Alice"));
    }
}
