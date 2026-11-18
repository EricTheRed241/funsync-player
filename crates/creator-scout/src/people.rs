//! The people the user likes, for `--profiles`: the creators swiped yes on
//! the Creators site (source `swiped`), the favourite artists
//! (content-preferences.json `favourites` with config.json
//! `library.favouriteArtists` on top: source `favourite`) and the pornstar
//! list (`pornstarTags.pornstars`: source `pornstar`). Every run sets each
//! creator's `sources` from them, so a favourite turned off loses
//! `favourite`.

use crate::state::Swipes;
use crate::store::{self, CreatorsFile, SOURCES};
use slr_scout::prefs::Pornstar;

/// Someone the user likes.
#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    pub name: String,
    pub aliases: Vec<String>,
    /// Members of [`SOURCES`], in its order.
    pub sources: Vec<&'static str>,
    /// Her creator's key in creators.json, or the one she gets.
    pub slug: String,
}

impl Person {
    /// The name and aliases.
    pub fn names(&self) -> Vec<String> {
        std::iter::once(self.name.clone())
            .chain(self.aliases.iter().cloned())
            .collect()
    }

    fn keys(&self) -> impl Iterator<Item = String> + '_ {
        std::iter::once(&self.name)
            .chain(&self.aliases)
            .map(|n| store::slug(n))
            .filter(|s| !s.is_empty())
    }
}

/// Add a person, or her source to the one she already is (a name or alias
/// in common, or the same creator). Her creator's name and aliases count
/// as her names too.
fn add_person(
    out: &mut Vec<Person>,
    file: &CreatorsFile,
    name: &str,
    aliases: &[String],
    source: &'static str,
    slug: Option<&str>,
) {
    let given: Vec<&str> = std::iter::once(name)
        .chain(aliases.iter().map(String::as_str))
        .map(str::trim)
        .filter(|n| !store::slug(n).is_empty())
        .collect();
    let Some(&first) = given.first() else { return };
    let slug = slug
        .map(str::to_string)
        .or_else(|| given.iter().find_map(|n| file.find(n)))
        .unwrap_or_else(|| store::slug(first));
    let mut names: Vec<String> = Vec::new();
    let known = file
        .creators
        .get(&slug)
        .map(|c| std::iter::once(&c.name).chain(&c.aliases));
    for n in given
        .into_iter()
        .map(str::to_string)
        .chain(known.into_iter().flatten().map(|n| n.trim().to_string()))
    {
        let k = store::slug(&n);
        if !k.is_empty() && !names.iter().any(|x| store::slug(x) == k) {
            names.push(n);
        }
    }
    let keys: Vec<String> = names.iter().map(|n| store::slug(n)).collect();
    if let Some(p) = out
        .iter_mut()
        .find(|p| p.slug == slug || p.keys().any(|k| keys.contains(&k)))
    {
        if !p.sources.contains(&source) {
            p.sources.push(source);
            p.sources
                .sort_by_key(|s| SOURCES.iter().position(|x| x == s));
        }
        for n in names {
            let k = store::slug(&n);
            if !p.keys().any(|x| x == k) {
                p.aliases.push(n);
            }
        }
        return;
    }
    let mut names = names.into_iter();
    out.push(Person {
        name: names.next().unwrap_or_default(),
        aliases: names.collect(),
        sources: vec![source],
        slug,
    });
}

/// The people, swiped yes first (slug order), then the favourite artists
/// (`builtin` with `changes` applied: true adds, false removes), then the
/// pornstar list; one person per name, aliases included, with all her
/// sources.
pub fn people(
    builtin: &[String],
    changes: &[(String, bool)],
    pornstars: &[Pornstar],
    file: &CreatorsFile,
    swipes: &Swipes,
) -> Vec<Person> {
    let mut out = Vec::new();
    for (slug, choice) in &swipes.creators {
        if choice != "yes" {
            continue;
        }
        if let Some(c) = file.creators.get(slug) {
            let name = if c.name.trim().is_empty() {
                slug
            } else {
                &c.name
            };
            add_person(&mut out, file, name, &c.aliases, "swiped", Some(slug));
        }
    }
    for name in slr_scout::library::effective_favourites(builtin, changes) {
        add_person(&mut out, file, &name, &[], "favourite", None);
    }
    for p in pornstars {
        add_person(&mut out, file, &p.name, &p.aliases, "pornstar", None);
    }
    out
}

/// Set every creator's `sources` from the people; returns how many
/// changed.
pub fn merge_sources(file: &mut CreatorsFile, people: &[Person]) -> usize {
    let mut changed = 0;
    for (slug, c) in file.creators.iter_mut() {
        let want: Vec<String> = SOURCES
            .iter()
            .filter(|s| {
                people
                    .iter()
                    .any(|p| p.slug == *slug && p.sources.contains(s))
            })
            .map(|s| s.to_string())
            .collect();
        if c.sources != want {
            c.sources = want;
            changed += 1;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Creator, Instagram};
    use serde_json::json;

    fn creator(name: &str, aliases: &[&str], ig: bool) -> Creator {
        Creator {
            name: name.into(),
            aliases: aliases.iter().map(|a| a.to_string()).collect(),
            instagram: ig.then(|| Instagram {
                handle: store::slug(name),
                ..Instagram::default()
            }),
            ..Creator::default()
        }
    }

    fn sample() -> (CreatorsFile, Swipes) {
        let mut file = CreatorsFile::empty();
        file.creators
            .insert("lucycat".into(), creator("Lucy Cat", &[], true));
        file.creators.insert(
            "nofacegirl".into(),
            creator("NoFaceGirl", &["No Face Girl"], false),
        );
        file.creators
            .insert("ann".into(), creator("Ann", &[], true));
        file.creators
            .insert("bea".into(), creator("Bea", &[], false));
        let mut swipes = Swipes::default();
        swipes.creators.insert("lucycat".into(), "yes".into());
        swipes.creators.insert("ann".into(), "no".into());
        swipes.creators.insert("gone".into(), "yes".into());
        (file, swipes)
    }

    fn pornstar(name: &str, aliases: &[&str]) -> Pornstar {
        serde_json::from_value(json!({"name": name, "aliases": aliases})).unwrap()
    }

    #[test]
    fn people_from_prefs_config_and_swipes() {
        let (file, swipes) = sample();
        let builtin: Vec<String> = ["Candy Love", "JJULEZ", "No Face Girl"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // config.json: JJULEZ turned off, Lucy Cat and Miss Lexa added.
        let user = slr_scout::library::from_json(&json!({"settings": {"library": {
            "favouriteArtists": {"jjulez": false, "Lucy Cat": true, "Miss Lexa": true}}}}));
        let pornstars = [
            pornstar("Candy Love", &[]),
            pornstar("NoFaceGirl", &["No Face Girl"]),
            pornstar("Nicole Aniston", &["Nicole A"]),
        ];
        let list = people(
            &builtin,
            &user.favourite_changes,
            &pornstars,
            &file,
            &swipes,
        );
        let got: Vec<(&str, &str, Vec<&str>)> = list
            .iter()
            .map(|p| (p.name.as_str(), p.slug.as_str(), p.sources.clone()))
            .collect();
        assert_eq!(
            got,
            [
                // Swiped yes (a swipe for a creator no longer listed, and
                // one swiped no, aren't people).
                ("Lucy Cat", "lucycat", vec!["swiped", "favourite"]),
                ("Candy Love", "candylove", vec!["favourite", "pornstar"]),
                // Found by alias: the creator's slug.
                ("No Face Girl", "nofacegirl", vec!["favourite", "pornstar"]),
                ("Miss Lexa", "misslexa", vec!["favourite"]),
                ("Nicole Aniston", "nicoleaniston", vec!["pornstar"]),
            ]
        );
        // The creator's own spelling is the same name: no alias.
        assert!(list[2].aliases.is_empty());
        assert_eq!(list[4].names(), ["Nicole Aniston", "Nicole A"]);
    }

    #[test]
    fn sources_follow_the_people() {
        let (mut file, swipes) = sample();
        file.creators.get_mut("bea").unwrap().sources = vec!["favourite".into()];
        let builtin = vec!["Bea".to_string(), "Ann".to_string()];
        let list = people(&builtin, &[], &[pornstar("Bea", &[])], &file, &swipes);
        assert_eq!(merge_sources(&mut file, &list), 3);
        let src = |s: &str| file.creators[s].sources.clone();
        assert_eq!(src("lucycat"), ["swiped"]);
        assert_eq!(src("bea"), ["favourite", "pornstar"]);
        // Swiped no keeps the source (the site shows her as no).
        assert_eq!(src("ann"), ["favourite"]);
        assert!(src("nofacegirl").is_empty());
        assert_eq!(merge_sources(&mut file, &list), 0);
        // Bea turned off as a favourite loses `favourite`.
        let user = slr_scout::library::from_json(
            &json!({"library": {"favouriteArtists": {"Bea": false}}}),
        );
        let list = people(
            &builtin,
            &user.favourite_changes,
            &[pornstar("Bea", &[])],
            &file,
            &swipes,
        );
        assert_eq!(merge_sources(&mut file, &list), 1);
        assert_eq!(file.creators["bea"].sources, ["pornstar"]);
    }
}
