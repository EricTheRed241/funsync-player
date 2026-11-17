//! Just enough HTML: the opening tags of one element name and their
//! attributes (entity-decoded). Pages are read, never rendered.

use slr_scout::forum::decode_entities;

/// The attribute text of every `<name …>` opening tag (case-insensitive).
pub fn open_tags<'a>(html: &'a str, name: &str) -> Vec<&'a str> {
    let lower = html.to_ascii_lowercase();
    let needle = format!("<{}", name.to_ascii_lowercase());
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = lower[from..].find(&needle) {
        let start = from + at + needle.len();
        from = start;
        // `<a` must not match `<abbr`.
        if !html[start..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_whitespace() || c == '>' || c == '/')
        {
            continue;
        }
        let Some(end) = tag_end(&html[start..]) else {
            break;
        };
        out.push(&html[start..start + end]);
        from = start + end;
    }
    out
}

/// Where a tag's `>` is, skipping quoted attribute values.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

/// Every `name=value` attribute of a tag's attribute text, names in lower
/// case, values entity-decoded.
pub fn attrs(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = tag;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '/');
        if rest.is_empty() {
            break;
        }
        let name_end = rest
            .find(|c: char| c.is_ascii_whitespace() || c == '=' || c == '>')
            .unwrap_or(rest.len());
        let name = rest[..name_end].to_ascii_lowercase();
        rest = rest[name_end..].trim_start();
        let Some(after) = rest.strip_prefix('=') else {
            if name.is_empty() {
                break;
            }
            out.push((name, String::new()));
            continue;
        };
        let after = after.trim_start();
        let (value, next) = match after.chars().next() {
            Some(q @ ('"' | '\'')) => {
                let body = &after[1..];
                let end = body.find(q).unwrap_or(body.len());
                (&body[..end], body.get(end + 1..).unwrap_or(""))
            }
            _ => {
                let end = after
                    .find(|c: char| c.is_ascii_whitespace())
                    .unwrap_or(after.len());
                (&after[..end], &after[end..])
            }
        };
        out.push((name, decode_entities(value)));
        rest = next;
    }
    out
}

pub fn attr(tag: &str, name: &str) -> Option<String> {
    attrs(tag)
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v)
}

/// The content of `<meta name|property="key" content="…">`.
pub fn meta(html: &str, key: &str) -> Option<String> {
    open_tags(html, "meta").into_iter().find_map(|t| {
        let a = attrs(t);
        let named = a
            .iter()
            .any(|(n, v)| (n == "name" || n == "property") && v.eq_ignore_ascii_case(key));
        if !named {
            return None;
        }
        a.into_iter().find(|(n, _)| n == "content").map(|(_, v)| v)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_and_attributes() {
        let html = r#"<p><abbr x>.</abbr><a class="x" href="/a?b=1&amp;c=2">t</a><A HREF='https://x.y/'>
            <img src=//cdn.x/i.jpg alt="a > b" width=10 /><meta content="c &quot;q&quot;" name="description">"#;
        let a = open_tags(html, "a");
        assert_eq!(a.len(), 2);
        assert_eq!(attr(a[0], "href").as_deref(), Some("/a?b=1&c=2"));
        assert_eq!(attr(a[1], "href").as_deref(), Some("https://x.y/"));
        let img = open_tags(html, "img");
        assert_eq!(attr(img[0], "src").as_deref(), Some("//cdn.x/i.jpg"));
        assert_eq!(attr(img[0], "alt").as_deref(), Some("a > b"));
        assert_eq!(attr(img[0], "width").as_deref(), Some("10"));
        assert_eq!(meta(html, "description").as_deref(), Some("c \"q\""));
        assert_eq!(meta(html, "og:image"), None);
    }
}
