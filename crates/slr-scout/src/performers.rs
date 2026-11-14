//! Who to scout: `slrScout.actors`, then the favourite artists
//! (`includeFavourites`: FAVOURITES with the user's `library.favouriteArtists`
//! changes) and the listed pornstars (`includePornstars`:
//! `pornstarTags.pornstars`, with their aliases), each name once. `--actor`
//! replaces the list.

use crate::prefs::Prefs;
use crate::slr::Wanted;
use crate::text;

/// The performers to look up on SLR, in order, each once (names compared
/// like titles: any case, accents or punctuation; a later entry's aliases
/// join an earlier one).
pub fn assemble(prefs: &Prefs, favourites: &[String], cli: &[String]) -> Vec<Wanted> {
    let mut out: Vec<Wanted> = Vec::new();
    let mut add = |name: &str, aliases: &[String]| {
        let spellings: Vec<String> = std::iter::once(name)
            .chain(aliases.iter().map(String::as_str))
            .filter(|n| !text::normalise(n).is_empty())
            .map(|n| n.trim().to_string())
            .collect();
        let Some(first) = spellings.first().cloned() else {
            return;
        };
        let known = |w: &Wanted, n: &str| {
            let key = text::normalise(n);
            std::iter::once(&w.name)
                .chain(&w.aliases)
                .any(|m| text::normalise(m) == key)
        };
        match out
            .iter_mut()
            .find(|w| spellings.iter().any(|n| known(w, n)))
        {
            Some(w) => {
                for n in &spellings {
                    if !known(w, n) {
                        w.aliases.push(n.clone());
                    }
                }
            }
            None => out.push(Wanted {
                name: first,
                aliases: spellings[1..].to_vec(),
            }),
        }
    };
    if !cli.is_empty() {
        for name in cli {
            add(name, &[]);
        }
        return out;
    }
    let s = &prefs.slr_scout;
    for name in &s.actors {
        add(name, &[]);
    }
    if s.include_favourites {
        for name in favourites {
            add(name, &[]);
        }
    }
    if s.include_pornstars {
        for p in &prefs.pornstar_tags.pornstars {
            add(&p.name, &p.aliases);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::tests::with_slr_scout;

    fn prefs(include_favourites: bool, include_pornstars: bool) -> Prefs {
        let mut p = crate::prefs::parse(&with_slr_scout(&format!(
            r#"{{"actors": ["Miss Lexa", "candy love"], "includeFavourites": {include_favourites},
            "includePornstars": {include_pornstars}, "vrOnly": true, "requireCreampie": false,
            "bonuses": {{"handmadeScript": 4, "pmv": 2, "compilation": 2}}, "popularityPer": 1500}}"#
        )))
        .unwrap();
        p.pornstar_tags.pornstars = serde_json::from_str(
            r#"[{"name": "Candy Love"}, {"name": "NoFaceGirl", "aliases": ["No Face Girl"]},
                {"name": "Riley Reid"}]"#,
        )
        .unwrap();
        p
    }

    fn names(list: &[Wanted]) -> Vec<&str> {
        list.iter().map(|w| w.name.as_str()).collect()
    }

    #[test]
    fn lists_actors_favourites_and_pornstars_once() {
        let favourites = vec!["No Face Girl".to_string(), "VRBangers".to_string()];
        let all = assemble(&prefs(true, true), &favourites, &[]);
        assert_eq!(
            names(&all),
            [
                "Miss Lexa",
                "candy love",
                "No Face Girl",
                "VRBangers",
                "Riley Reid"
            ]
        );
        // The pornstar's own name joined the favourite's entry.
        assert_eq!(all[2].aliases, ["NoFaceGirl"]);
        assert!(all[1].aliases.is_empty());
        assert_eq!(
            names(&assemble(&prefs(false, true), &favourites, &[])),
            ["Miss Lexa", "candy love", "NoFaceGirl", "Riley Reid"]
        );
        assert_eq!(
            names(&assemble(&prefs(true, false), &favourites, &[])),
            ["Miss Lexa", "candy love", "No Face Girl", "VRBangers"]
        );
        assert_eq!(
            names(&assemble(
                &prefs(true, true),
                &favourites,
                &["Lexi Luna".to_string(), "lexi  luna".to_string()]
            )),
            ["Lexi Luna"]
        );
    }
}
