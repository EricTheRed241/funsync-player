//! Does an account exist, and is it the creator's? Instagram's public page
//! as a link-preview bot sees it (`facebookexternalhit`: the page's
//! description or og:description meta holds the counts, name and maybe
//! the bio; no meta, no account),
//! Twitch's public GraphQL. How well it matches the name: `high` (the
//! name run together is in the handle or the full name), `medium` (a word
//! of 4+ letters of the name is), else `low` (not kept).

use crate::html;
use crate::store;
use serde_json::{Value, json};
use slr_scout::http::{self, Http, NetError, Pacer, Request};
use slr_scout::text;
use std::time::Duration;

pub const INSTAGRAM: &str = "https://www.instagram.com";
pub const TWITCH_GQL: &str = "https://gql.twitch.tv/gql";
pub const TWITCH_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
pub const PREVIEW_UA: &str = "facebookexternalhit/1.1";
pub const INSTAGRAM_GAP: Duration = Duration::from_secs(3);
pub const TWITCH_GAP: Duration = Duration::from_millis(500);

/// An Instagram profile as its page's description meta gives it.
#[derive(Debug, Clone, PartialEq)]
pub struct IgPage {
    pub handle: String,
    pub followers: String,
    pub following: String,
    pub posts: String,
    pub full_name: String,
    pub bio: Option<String>,
    /// og:image: the profile photo.
    pub picture: Option<String>,
}

fn count_before<'a>(s: &'a str, word: &str) -> Option<(&'a str, &'a str)> {
    // "5M Followers, " / "1 Follower, "
    for w in [
        format!(" {word}s, "),
        format!(" {word}, "),
        format!(" {word}s - "),
        format!(" {word} - "),
    ] {
        if let Some((n, rest)) = s.split_once(&w) {
            let n = n.trim();
            if !n.is_empty() && n.len() <= 12 && !n.contains(' ') {
                return Some((n, rest));
            }
        }
    }
    None
}

/// Parse `N Followers, N Following, N Posts - Name (@handle) on Instagram:
/// "bio"`.
pub fn parse_description(
    d: &str,
) -> Option<(String, String, String, String, String, Option<String>)> {
    let (followers, rest) = count_before(d, "Follower")?;
    let (following, rest) = rest
        .split_once(" Following, ")
        .map(|(n, r)| (n.trim(), r))?;
    let (posts, rest) = count_before(rest, "Post")?;
    let open = rest
        .find(" (@")
        .or_else(|| rest.starts_with("(@").then_some(0))?;
    // The og:description form: "… Posts - See Instagram photos and videos
    // from Name (@handle)", no bio.
    let name = rest[..open].trim();
    let full_name = name
        .strip_prefix("See Instagram photos and videos from")
        .unwrap_or(name)
        .trim()
        .to_string();
    let after = &rest[open..].trim_start()[2..];
    let close = after.find(')')?;
    let handle = after[..close].to_string();
    if !crate::judge::valid_handle(&handle, false) {
        return None;
    }
    let tail = after[close + 1..].trim();
    let bio = tail
        .strip_prefix("on Instagram")
        .map(str::trim)
        .and_then(|t| t.strip_prefix(':'))
        .map(|t| {
            let t = t.trim();
            t.strip_prefix('"')
                .and_then(|t| t.strip_suffix('"'))
                .unwrap_or(t)
                .trim()
                .to_string()
        })
        .filter(|b| !b.is_empty());
    Some((
        handle,
        followers.to_string(),
        following.to_string(),
        posts.to_string(),
        full_name,
        bio,
    ))
}

/// The profile in an Instagram page (its description meta, else
/// og:description); `None` when it has neither (no such account).
pub fn parse_instagram_page(page: &str) -> Option<IgPage> {
    let (handle, followers, following, posts, full_name, bio) = html::meta(page, "description")
        .and_then(|d| parse_description(&d))
        .or_else(|| parse_description(&html::meta(page, "og:description")?))?;
    Some(IgPage {
        handle,
        followers,
        following,
        posts,
        full_name,
        bio,
        picture: html::meta(page, "og:image"),
    })
}

/// GET the public page as a link-preview bot.
pub fn instagram(
    http: &dyn Http,
    pacer: &mut Pacer,
    base: &str,
    handle: &str,
) -> Result<Option<IgPage>, NetError> {
    pacer.wait();
    let url = format!("{base}/{handle}/");
    let res = http
        .send(&Request::get(&url).header("User-Agent", PREVIEW_UA))
        .map_err(|e| NetError::Failed(format!("{url}: {e}")))?;
    match res.status {
        200..=299 => Ok(parse_instagram_page(&res.body)),
        404 => Ok(None),
        429 => Err(NetError::RateLimited(format!("{url}: HTTP 429"))),
        s => Err(NetError::Failed(format!("{url}: HTTP {s}"))),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TwitchUser {
    pub login: String,
    pub display_name: String,
    pub description: String,
    pub followers: u64,
    pub last_live: Option<String>,
}

pub fn twitch_body(login: &str) -> String {
    json!({
        "query": "query($login:String!){user(login:$login){login displayName description followers{totalCount} lastBroadcast{startedAt}}}",
        "variables": {"login": login}
    })
    .to_string()
}

/// A GraphQL answer: `Ok(None)` when there is no such user.
pub fn parse_twitch(v: &Value) -> Result<Option<TwitchUser>, String> {
    if let Some(errors) = v["errors"].as_array().filter(|e| !e.is_empty()) {
        return Err(format!(
            "Twitch: {}",
            errors[0]["message"].as_str().unwrap_or("error")
        ));
    }
    let u = &v["data"]["user"];
    if u.is_null() {
        return Ok(None);
    }
    let login = u["login"].as_str().ok_or("Twitch: user without login")?;
    Ok(Some(TwitchUser {
        login: login.to_string(),
        display_name: u["displayName"].as_str().unwrap_or(login).to_string(),
        description: u["description"].as_str().unwrap_or_default().to_string(),
        followers: u["followers"]["totalCount"].as_u64().unwrap_or(0),
        last_live: u["lastBroadcast"]["startedAt"].as_str().map(str::to_string),
    }))
}

pub fn twitch(
    http: &dyn Http,
    pacer: &mut Pacer,
    url: &str,
    login: &str,
) -> Result<Option<TwitchUser>, NetError> {
    pacer.wait();
    let req = Request::post(url, twitch_body(&login.to_lowercase()))
        .header("Client-Id", TWITCH_CLIENT_ID)
        .header("Content-Type", "application/json");
    let v = http::json(http, &req)?;
    parse_twitch(&v).map_err(NetError::Failed)
}

/// `high` / `medium` / `low`: how well an account (handle, display or full
/// name) matches one of the creator's names.
pub fn match_quality(names: &[String], handle: &str, full_name: &str) -> &'static str {
    let h = store::slug(handle);
    let f = store::slug(full_name);
    let mut best = "low";
    for n in names {
        let joined = store::slug(n);
        if joined.len() >= 3 && (h.contains(&joined) || f == joined || f.contains(&joined)) {
            return "high";
        }
        let norm = text::normalise(n);
        if text::words(&norm)
            .filter(|w| w.len() >= 4)
            .any(|w| h.contains(w) || f.contains(w))
        {
            best = "medium";
        }
    }
    best
}

/// Followers as Instagram writes them: `4,924`, `35.5K`, `355K`, `1.2M`.
pub fn compact(n: u64) -> String {
    fn one_decimal(x: f64, unit: &str) -> String {
        let t = (x * 10.0).floor() / 10.0;
        if t >= 100.0 || t.fract() == 0.0 {
            format!("{}{unit}", t.floor() as u64)
        } else {
            format!("{t:.1}{unit}")
        }
    }
    if n < 10_000 {
        let s = n.to_string();
        let mut out = String::new();
        for (i, c) in s.chars().enumerate() {
            if i > 0 && (s.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(c);
        }
        out
    } else if n < 1_000_000 {
        one_decimal(n as f64 / 1_000.0, "K")
    } else {
        one_decimal(n as f64 / 1_000_000.0, "M")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_instagram_meta() {
        let page = r#"<html><head><meta property="og:image" content="https://cdn.ig/p.jpg?a=1&amp;b=2" />
            <meta content="5M Followers, 12 Following, 225 Posts - Sweetie Fox (&#064;swfx_real) on Instagram: &quot;Hi &amp; welcome&quot;" name="description" /></head></html>"#;
        let p = parse_instagram_page(page).unwrap();
        assert_eq!(p.handle, "swfx_real");
        assert_eq!(
            (p.followers.as_str(), p.following.as_str(), p.posts.as_str()),
            ("5M", "12", "225")
        );
        assert_eq!(p.full_name, "Sweetie Fox");
        assert_eq!(p.bio.as_deref(), Some("Hi & welcome"));
        assert_eq!(p.picture.as_deref(), Some("https://cdn.ig/p.jpg?a=1&b=2"));

        let no_bio = r#"<meta name="description" content="4,924 Followers, 1 Following, 1 Post - (@stormi.skies) on Instagram">"#;
        let p = parse_instagram_page(no_bio).unwrap();
        assert_eq!(
            (p.handle.as_str(), p.full_name.as_str()),
            ("stormi.skies", "")
        );
        assert_eq!(
            (p.followers.as_str(), p.posts.as_str(), p.bio),
            ("4,924", "1", None)
        );
        // What Instagram served on 2026-11-17: og:description only.
        let og = r#"<meta property="og:image" content="https://scontent.cdninstagram.com/p.jpg?a=1&amp;b=2" />
            <meta property="og:description" content="668K Followers, 192 Following, 39 Posts - See Instagram photos and videos from Hannah (&#064;itshannahowo)" />"#;
        let p = parse_instagram_page(og).unwrap();
        assert_eq!(
            (
                p.handle.as_str(),
                p.full_name.as_str(),
                p.followers.as_str()
            ),
            ("itshannahowo", "Hannah", "668K")
        );
        assert_eq!((p.posts.as_str(), p.bio), ("39", None));

        assert_eq!(
            parse_instagram_page("<html><title>Instagram</title></html>"),
            None
        );
        assert_eq!(
            parse_instagram_page(
                r#"<meta name="description" content="Create an account or log in to Instagram">"#
            ),
            None
        );
    }

    #[test]
    fn parses_twitch_answers() {
        let v: Value = serde_json::from_str(
            r#"{"data":{"user":{"login":"sweetiefoxxx","displayName":"SweetieFoxxx","description":"hi",
                "followers":{"totalCount":9225},"lastBroadcast":{"startedAt":"2020-09-11T18:17:44Z"}}}}"#,
        )
        .unwrap();
        let u = parse_twitch(&v).unwrap().unwrap();
        assert_eq!((u.login.as_str(), u.followers), ("sweetiefoxxx", 9225));
        assert_eq!(u.last_live.as_deref(), Some("2020-09-11T18:17:44Z"));
        let never: Value = serde_json::from_str(
            r#"{"data":{"user":{"login":"x","displayName":"X","description":"","followers":{"totalCount":0},"lastBroadcast":{"startedAt":null}}}}"#,
        )
        .unwrap();
        assert_eq!(parse_twitch(&never).unwrap().unwrap().last_live, None);
        assert_eq!(
            parse_twitch(&json!({"data": {"user": null}})).unwrap(),
            None
        );
        assert!(parse_twitch(&json!({"errors": [{"message": "bad"}]})).is_err());
        let body: Value = serde_json::from_str(&twitch_body("abc")).unwrap();
        assert_eq!(body["variables"]["login"], "abc");
    }

    #[test]
    fn match_quality_and_counts() {
        let names = vec!["Sweetie Fox".to_string()];
        assert_eq!(match_quality(&names, "swfx_real", "Sweetie Fox"), "high");
        assert_eq!(match_quality(&names, "sweetiefoxxx", ""), "high");
        assert_eq!(match_quality(&names, "foxy_girl", "Foxy"), "low");
        assert_eq!(match_quality(&names, "sweetie_official", "S"), "medium");
        assert_eq!(compact(4924), "4,924");
        assert_eq!(compact(355_400), "355K");
        assert_eq!(compact(35_560), "35.5K");
        assert_eq!(compact(5_000_000), "5M");
        assert_eq!(compact(1_234_567), "1.2M");
        assert_eq!(compact(12), "12");
    }
}
