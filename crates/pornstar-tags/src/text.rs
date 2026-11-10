//! Text matching: a video's text (path segments, sidecar titles, performer
//! names, the EroScripts topic slug) and the phrases that name a performer
//! are normalised the same way (NFKC, lower case, every non-alphanumeric
//! character → space, spaces collapsed); a phrase matches when it appears as
//! whole words.

use unicode_normalization::UnicodeNormalization;

/// NFKC, lower case, non-alphanumerics → one space, trimmed.
pub fn normalise(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.nfkc().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        } else {
            space = true;
        }
    }
    out
}

/// A tag made from a name: normalised, words joined with '-'
/// (`Candy Love` → `candy-love`, `NoFaceGirl` → `nofacegirl`).
pub fn slug(name: &str) -> String {
    normalise(name).replace(' ', "-")
}

/// The slug of an EroScripts topic URL
/// (`https://discuss.eroscripts.com/t/<slug>/<id>`).
pub fn topic_slug(url: &str) -> Option<&str> {
    let (_, rest) = url.split_once("/t/")?;
    let slug = rest.split(['/', '?', '#']).next()?;
    (!slug.is_empty()).then_some(slug)
}

/// A video's searchable text: each part normalised, parts kept apart so a
/// phrase can't match across two of them.
#[derive(Debug, Clone, Default)]
pub struct Text(String);

impl Text {
    pub fn new<S: AsRef<str>>(parts: impl IntoIterator<Item = S>) -> Text {
        let mut s = String::from(" ");
        for part in parts {
            let n = normalise(part.as_ref());
            if !n.is_empty() {
                s.push_str(&n);
                s.push_str(" | ");
            }
        }
        Text(s)
    }

    pub fn contains(&self, phrase: &Phrase) -> bool {
        self.0.contains(&phrase.0)
    }
}

/// A normalised phrase, padded for whole-word search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phrase(String);

impl Phrase {
    /// `None` when nothing alphanumeric is left.
    pub fn new(raw: &str) -> Option<Phrase> {
        let n = normalise(raw);
        (!n.is_empty()).then(|| Phrase(format!(" {n} ")))
    }

    /// The normalised words.
    pub fn words(&self) -> &str {
        self.0.trim()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(text: &[&str], phrase: &str) -> bool {
        Text::new(text).contains(&Phrase::new(phrase).unwrap())
    }

    #[test]
    fn normalises_case_punctuation_and_width() {
        assert_eq!(
            normalise("  Candy_Love — Gorgeous ✨ brunette!! "),
            "candy love gorgeous brunette"
        );
        assert_eq!(normalise("Ｒｉｌｅｙ Reid"), "riley reid");
        assert_eq!(normalise("Zoë"), normalise("Zoe\u{308}"));
        assert_eq!(normalise("--"), "");
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Candy Love"), "candy-love");
        assert_eq!(slug("NoFaceGirl"), "nofacegirl");
        assert_eq!(slug(" Xxlayna  Marie! "), "xxlayna-marie");
    }

    #[test]
    fn matches_whole_words_only() {
        assert!(matches(
            &["Candy Love - gets fucked missionary POV"],
            "Candy Love"
        ));
        assert!(matches(&["candy_love.mp4"], "Candy Love"));
        assert!(!matches(&["CandyLove - x"], "Candy Love"));
        assert!(!matches(&["Candy Lovely"], "Candy Love"));
        assert!(!matches(&["Eva Elfies"], "Eva Elfie"));
        assert!(matches(
            &["Mia Malkova, Riley Reid - One Last Bang"],
            "Riley Reid"
        ));
    }

    #[test]
    fn joined_names_match_as_one_word() {
        assert!(matches(&["NoFaceGirl - Edges you"], "NoFaceGirl"));
        assert!(!matches(&["No Face Girl - Edges you"], "NoFaceGirl"));
        assert!(matches(&["No Face Girl - Edges you"], "No Face Girl"));
        assert!(!matches(&["NoFaceGirls"], "NoFaceGirl"));
    }

    #[test]
    fn parts_dont_join() {
        assert!(!matches(&["Artist Candy", "Love Story"], "Candy Love"));
        assert!(matches(&["Artist", "Candy Love Story"], "Candy Love"));
    }

    #[test]
    fn topic_slugs_are_text() {
        let url = "https://discuss.eroscripts.com/t/nofacegirl-creampied-after-she-cums/123456";
        assert_eq!(topic_slug(url), Some("nofacegirl-creampied-after-she-cums"));
        assert!(matches(&[topic_slug(url).unwrap()], "NoFaceGirl"));
        assert!(matches(
            &["whoispiperpresley-riding-pov"],
            "whoispiperpresley"
        ));
        assert_eq!(
            topic_slug("https://discuss.eroscripts.com/t/abc"),
            Some("abc")
        );
        assert_eq!(topic_slug("https://example.com/x"), None);
    }
}
