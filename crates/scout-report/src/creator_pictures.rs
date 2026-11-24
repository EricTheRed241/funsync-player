//! The creators' pictures creator-scout saves under `<userData>/creator-pictures/<slug>/`,
//! served read-only at `/pictures/<slug>/<file>` by both the Creators site and the swipe
//! deck (its creator cards): both parts strictly `[A-Za-z0-9._-]`, not starting with a dot
//! (so no `..`, no hidden files), and a picture extension.

use std::path::{Path, PathBuf};

pub const DIR: &str = "creator-pictures";

/// Longest slug or file name served.
const MAX_NAME: usize = 200;

/// Whether `name` may be a slug or file name in a picture URL.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The content type of a picture file name, by extension; None for anything else.
pub fn content_type(file: &str) -> Option<&'static str> {
    let (_, ext) = file.rsplit_once('.')?;
    match ext.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

/// The URL of creator `slug`'s picture `file`, if both names are valid: relative
/// (`pictures/<slug>/<file>`) so it resolves under the app's `/swipe/` and `/creators/`
/// proxies too.
pub fn url(slug: &str, file: &str) -> Option<String> {
    (valid_name(slug) && valid_name(file) && content_type(file).is_some())
        .then(|| format!("pictures/{slug}/{file}"))
}

/// The file a request path names, `/pictures/<slug>/<file>`, with its content type;
/// None for any other shape.
pub fn file_for(user_data: &Path, path: &str) -> Option<(PathBuf, &'static str)> {
    let (slug, file) = path.strip_prefix("/pictures/")?.split_once('/')?;
    if !valid_name(slug) || !valid_name(file) {
        return None;
    }
    let kind = content_type(file)?;
    Some((user_data.join(DIR).join(slug).join(file), kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_strict_names_are_valid() {
        for ok in ["sweetiefox", "ig-3456789.jpg", "es-41415-0.jpg", "A_b.c-1"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in [
            "", ".", "..", ".hidden", "a/b", "a\\b", "a b", "%2e%2e", "é.jpg", "a\0b", "a:b",
        ] {
            assert!(!valid_name(bad), "{bad:?}");
        }
        assert!(!valid_name(&"a".repeat(201)));
    }

    #[test]
    fn maps_picture_urls_to_files() {
        let ud = Path::new("/ud");
        assert_eq!(
            file_for(ud, "/pictures/sweetiefox/ig-1.jpg"),
            Some((
                PathBuf::from("/ud/creator-pictures/sweetiefox/ig-1.jpg"),
                "image/jpeg"
            ))
        );
        assert_eq!(
            file_for(ud, "/pictures/a/b.PNG").map(|f| f.1),
            Some("image/png")
        );
        for bad in [
            "/pictures/",
            "/pictures/a",
            "/pictures/a/",
            "/pictures/../x.jpg",
            "/pictures/a/../b.jpg",
            "/pictures/a/b/c.jpg",
            "/pictures/a/%2e%2e.jpg",
            "/pictures/a/.jpg",
            "/pictures/a/b.txt",
            "/pictures/a/b",
            "/pictures/a/b.json",
            "/frames/a/b.jpg",
        ] {
            assert_eq!(file_for(ud, bad), None, "{bad}");
        }
    }

    #[test]
    fn builds_urls_for_valid_names_only() {
        assert_eq!(
            url("sweetiefox", "ft-1.jpg").as_deref(),
            Some("pictures/sweetiefox/ft-1.jpg")
        );
        assert_eq!(url("sweetiefox", "../x.jpg"), None);
        assert_eq!(url("a b", "x.jpg"), None);
        assert_eq!(url("a", "x.svg"), None);
    }
}
