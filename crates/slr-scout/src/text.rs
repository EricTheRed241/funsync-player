//! Title matching: NFKD folded to ASCII, lower case, every non-alphanumeric
//! character → one space. A phrase matches when it appears as whole words.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// NFKD → ASCII (accents dropped), lower case, anything else (punctuation,
/// curly quotes, other scripts) → one space, trimmed.
pub fn normalise(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.nfkd().filter(|&c| !is_combining_mark(c)) {
        if c.is_ascii_alphanumeric() {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c.to_ascii_lowercase());
        } else {
            space = true;
        }
    }
    out
}

/// Whether `phrase` (already normalised) appears in `text` (already
/// normalised) as whole words.
pub fn contains_words(text: &str, phrase: &str) -> bool {
    !phrase.is_empty() && format!(" {text} ").contains(&format!(" {phrase} "))
}

/// The words of a normalised text.
pub fn words(normalised: &str) -> impl Iterator<Item = &str> {
    normalised.split(' ').filter(|w| !w.is_empty())
}

/// Percent-encode a query value (RFC 3986 unreserved characters kept).
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_to_ascii_words() {
        assert_eq!(
            normalise("Latina Milf oiled up & creampied"),
            "latina milf oiled up creampied"
        );
        assert_eq!(normalise("  Zoë’s  BEST-of!! "), "zoe s best of");
        assert_eq!(normalise("Ｍｉｓｓ Lexa"), "miss lexa");
        assert_eq!(normalise("--"), "");
    }

    #[test]
    fn matches_whole_words() {
        let post = normalise("[SLR] Miss Lexa - Please Creampie Me (VR, multi-axis)");
        assert!(contains_words(&post, &normalise("Please Creampie Me")));
        assert!(!contains_words(
            &post,
            &normalise("Please Creampie Melanie")
        ));
        assert!(!contains_words(&post, "lease creampie"));
        assert!(!contains_words(&post, ""));
    }

    #[test]
    fn encodes_query_values() {
        assert_eq!(url_encode("\"Miss Lexa\""), "%22Miss%20Lexa%22");
        assert_eq!(url_encode("a-b_c.d~e"), "a-b_c.d~e");
    }
}
