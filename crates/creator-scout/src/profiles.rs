//! Who a creator is online, and free pictures of her. The judge (an omp
//! run with web search) proposes her Instagram / Twitch handles, public
//! profile pages (OnlyFans, Fansly, Pornhub, X, Reddit, ManyVids, a link
//! page, IAFD / Babepedia / Boobpedia / FreeOnes) and pictures; each is
//! kept only when checked live: the page answers and its own name or
//! handle (og:title, title, h1) matches hers by the verify rule. Pictures
//! come from those pages (og:image), from the video pages her posts link
//! (poster frames) and from the judge when they sit on a page that passed;
//! under 300 px on the short side or wider than 3:1 (logos, banners) they
//! are dropped. Used by `--discover` for new creators and by `--profiles`
//! (the people the user likes, and creators short of pictures).

use crate::discover::{self, Filing, combine};
use crate::eroscripts;
use crate::faptap;
use crate::html;
use crate::judge::{self, Handles, PersonItem};
use crate::library::LibraryIds;
use crate::people::{Person, merge_sources};
use crate::state::{State, Swipes};
use crate::store::{self, Creator, CreatorsFile, Instagram, Picture, Profile, Twitch};
use crate::verify;
use serde_json::Map;
use slr_scout::http::{Http, NetError, Pacer, Request};
use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

/// A page, search or post that failed for one person: logged, and the step
/// says work is left (exit 2; the person or post is tried next run) rather
/// than failing: one unreachable site (a TLS error on a link page) mustn't
/// fail a run that confirmed everyone else.
fn note_skip(e: &NetError, what: &str, code: &mut i32, out: &mut dyn Write) {
    let _ = writeln!(out, "{what}: {e}");
    *code = combine(*code, 2);
}

/// A creator with fewer pictures than this is due for `--profiles`.
pub const MIN_PICTURES: usize = 3;
/// Nothing found (or still short of pictures): looked up again after this
/// long.
pub const MISS_DAYS: i64 = 30;
/// Posts filed for a new creator at most.
pub const MAX_POSTS: usize = 30;
/// Profile pages checked per creator, at most.
const MAX_PAGES: usize = 8;
/// Video pages read for poster frames per creator and run, at most.
const MAX_VIDEO_PAGES: usize = 5;
/// Pictures smaller than this on the short side are dropped.
pub const MIN_SIDE: u32 = 300;
/// Pictures wider (or taller) than this ratio are logos or banners.
pub const MAX_ASPECT: f64 = 3.0;
/// Between public page fetches.
pub const PAGE_GAP: Duration = Duration::from_millis(1500);

/// The site of a profile URL and the handle in its path; `None` for
/// Instagram and Twitch (checked their own way) and for anything not
/// https.
pub fn site_of(url: &str) -> Option<(&'static str, String)> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.to_ascii_lowercase();
    let host = host.split(':').next()?;
    let segs: Vec<&str> = path
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let on = |d: &str| host == d || host.ends_with(&format!(".{d}"));
    let seg = |i: usize| segs.get(i).map(|s| s.to_string());
    let (site, handle) = if on("instagram.com") || on("twitch.tv") {
        return None;
    } else if on("onlyfans.com") {
        ("onlyfans", seg(0))
    } else if on("fansly.com") {
        ("fansly", seg(0))
    } else if on("pornhub.com") || on("pornhub.org") {
        match segs.first() {
            Some(&("model" | "pornstar" | "channels" | "users")) => ("pornhub", seg(1)),
            _ => return None,
        }
    } else if on("x.com") || on("twitter.com") {
        match segs.first() {
            Some(&("i" | "home" | "search" | "intent" | "hashtag")) | None => return None,
            _ => ("x", seg(0)),
        }
    } else if on("reddit.com") {
        match segs.first() {
            Some(&("u" | "user")) => ("reddit", seg(1)),
            _ => return None,
        }
    } else if on("manyvids.com") {
        match segs.first() {
            Some(s) if s.eq_ignore_ascii_case("profile") => ("manyvids", seg(2)),
            _ => return None,
        }
    } else if on("linktr.ee") || on("beacons.ai") || on("allmylinks.com") {
        ("linktree", seg(0))
    } else if on("iafd.com") {
        (
            "iafd",
            segs.last().map(|s| s.trim_end_matches(".htm").to_string()),
        )
    } else if on("babepedia.com") {
        ("babepedia", seg(1))
    } else if on("boobpedia.com") {
        ("boobpedia", seg(1))
    } else if on("freeones.com") {
        // `/<slug>` or `/<lang>/<slug>`, then `/bio` and the like.
        match segs.first() {
            Some(s) if s.len() == 2 => ("freeones", seg(1)),
            _ => ("freeones", seg(0)),
        }
    } else {
        let label = host.trim_start_matches("www.").split('.').next()?;
        ("profile", Some(label.to_string()))
    };
    let handle = handle?.trim_start_matches('@').to_string();
    (!handle.is_empty()).then_some((site, handle))
}

/// The picture source of a profile site (`pictures[].source`).
fn picture_source(site: &str) -> &'static str {
    match site {
        "onlyfans" => "onlyfans",
        "fansly" => "fansly",
        "x" => "x",
        "reddit" => "reddit",
        "pornhub" => "pornhub",
        _ => "profile",
    }
}

/// The text of the first `<tag>…</tag>`, inner tags removed, entities
/// decoded, trimmed.
fn element_text(page: &str, tag: &str) -> Option<String> {
    let lower = page.to_ascii_lowercase();
    let open = lower.find(&format!("<{tag}"))?;
    let body = open + lower[open..].find('>')? + 1;
    let end = body + lower[body..].find(&format!("</{tag}"))?;
    let mut text = String::new();
    let mut in_tag = false;
    for c in page[body..end].chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    let t = slr_scout::forum::decode_entities(
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .as_str(),
    );
    let t = t.trim().to_string();
    (!t.is_empty()).then_some(t)
}

/// The page's own names: og:title, twitter:title, `<title>`, the first
/// `<h1>`.
pub fn page_names(page: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in [
        html::meta(page, "og:title"),
        html::meta(page, "twitter:title"),
        element_text(page, "title"),
        element_text(page, "h1"),
    ]
    .into_iter()
    .flatten()
    {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// Titles of pages that aren't a profile: a login wall, an error.
const WALLS: &[&str] = &[
    "log in",
    "login",
    "sign in",
    "sign up",
    "not found",
    "doesn't exist",
    "doesn’t exist",
    "does not exist",
    "suspended",
    "unavailable",
    "access denied",
    "just a moment",
    "attention required",
    "error",
];

/// Whether a fetched profile page is hers: one of its own names (og:title,
/// title, h1) must match her full name by the verify rule's `high` level
/// (the URL's handle counts only when the page shows it too). A single word
/// of her name isn't enough for these pages: any site can host a lookalike.
/// `None` for a login wall, an error page or a page about someone else.
pub fn confirm(names: &[String], handle: &str, page: &str) -> Option<&'static str> {
    let own = page_names(page);
    if own.is_empty() {
        return None;
    }
    if own.iter().take(3).any(|t| {
        let t = t.to_lowercase();
        WALLS.iter().any(|w| t.contains(w))
    }) {
        return None;
    }
    let shown = |text: &str| {
        let h = store::slug(handle);
        h.len() >= 3 && store::slug(text).contains(&h)
    };
    let described = html::meta(page, "og:description").unwrap_or_default();
    let handle_seen = own.iter().any(|t| shown(t)) || shown(&described);
    own.iter()
        .any(|t| verify::match_quality(names, if handle_seen { handle } else { "" }, t) == "high")
        .then_some("high")
}

/// A page's own pictures (og:image, twitter:image), https only.
pub fn page_images(page: &str) -> Vec<String> {
    let mut out = Vec::new();
    for key in ["og:image", "og:image:secure_url", "twitter:image"] {
        if let Some(u) = html::meta(page, key).filter(|u| u.starts_with("https://"))
            && !out.contains(&u)
        {
            out.push(u);
        }
    }
    out
}

/// Sites that answer a link-preview bot (their pages are apps for a
/// browser); the rest get the browser User-Agent curl sends.
fn preview_ua(url: &str) -> bool {
    matches!(
        site_of(url).map(|(s, _)| s),
        Some("onlyfans" | "fansly" | "x" | "linktree")
    )
}

/// GET a public page, never logged in: `Ok(None)` when it doesn't answer
/// with a page (gone, a redirect to a login, refused).
pub fn fetch_page(
    http: &dyn Http,
    pacer: &mut Pacer,
    url: &str,
) -> Result<Option<String>, NetError> {
    pacer.wait();
    let mut req = Request::get(url).header("Accept", "text/html,application/xhtml+xml,*/*;q=0.8");
    if preview_ua(url) {
        req = req.header("User-Agent", verify::PREVIEW_UA);
    }
    let short = url.split('?').next().unwrap_or(url);
    match http.send(&req) {
        Ok(res) if res.status == 429 => Err(NetError::RateLimited(format!("{short}: HTTP 429"))),
        Ok(res) if (200..300).contains(&res.status) => Ok(Some(res.body)),
        Ok(_) => Ok(None),
        Err(e) => Err(NetError::Failed(format!("{short}: {e}"))),
    }
}

/// An image's format (file extension) and size from its first bytes:
/// JPEG, PNG, GIF, WebP.
pub fn image_size(b: &[u8]) -> Option<(&'static str, u32, u32)> {
    let be16 = |i: usize| Some(u32::from(*b.get(i)?) << 8 | u32::from(*b.get(i + 1)?));
    let le16 = |i: usize| Some(u32::from(*b.get(i)?) | u32::from(*b.get(i + 1)?) << 8);
    let le24 = |i: usize| Some(le16(i)? | u32::from(*b.get(i + 2)?) << 16);
    let be32 = |i: usize| Some(be16(i)? << 16 | be16(i + 2)?);
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(("png", be32(16)?, be32(20)?));
    }
    if b.starts_with(b"GIF8") {
        return Some(("gif", le16(6)?, le16(8)?));
    }
    if b.len() >= 30 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return match &b[12..16] {
            b"VP8 " => Some(("webp", le16(26)? & 0x3fff, le16(28)? & 0x3fff)),
            b"VP8L" => {
                let v = u32::from(b[21])
                    | u32::from(b[22]) << 8
                    | u32::from(b[23]) << 16
                    | u32::from(b[24]) << 24;
                Some(("webp", (v & 0x3fff) + 1, ((v >> 14) & 0x3fff) + 1))
            }
            b"VP8X" => Some(("webp", le24(24)? + 1, le24(27)? + 1)),
            _ => None,
        };
    }
    if b.starts_with(&[0xff, 0xd8]) {
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xff {
                i += 1;
                continue;
            }
            let m = b[i + 1];
            if matches!(m, 0xc0..=0xcf) && !matches!(m, 0xc4 | 0xc8 | 0xcc) {
                return Some(("jpg", be16(i + 7)?, be16(i + 5)?));
            }
            if m == 0xd8 || m == 0x01 || (0xd0..=0xd7).contains(&m) || m == 0xff {
                i += if m == 0xff { 1 } else { 2 };
                continue;
            }
            i += 2 + be16(i + 2)? as usize;
        }
    }
    None
}

/// Whether a picture of this size is worth keeping: at least 300 px on the
/// short side, at most 3:1.
pub fn keep_size(w: u32, h: u32) -> bool {
    let (short, long) = (w.min(h), w.max(h));
    short >= MIN_SIDE && f64::from(long) <= MAX_ASPECT * f64::from(short)
}

/// Download a picture into `dir` as `web-<hash>.<ext>`; `Ok(None)` when it
/// isn't a picture or is too small or a banner. An earlier copy is reused.
pub fn save_picture(http: &dyn Http, url: &str, dir: &Path) -> Result<Option<String>, String> {
    let stem = store::web_picture(url, "");
    if let Some(name) = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .find_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with(&stem).then_some(n)
        })
    {
        return Ok(Some(name));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = dir.join(format!(".{stem}part"));
    let _ = std::fs::remove_file(&part);
    let got = http
        .download(&Request::get(url), &part)
        .and_then(|_| std::fs::read(&part).map_err(|e| e.to_string()));
    let bytes = match got {
        Ok(b) => b,
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    };
    let kept = image_size(&bytes).filter(|(_, w, h)| keep_size(*w, *h));
    let Some((ext, _, _)) = kept else {
        let _ = std::fs::remove_file(&part);
        return Ok(None);
    };
    let name = store::web_picture(url, ext);
    std::fs::rename(&part, dir.join(&name)).map_err(|e| format!("{name}: {e}"))?;
    Ok(Some(name))
}

/// The pacers of everything this module and the post filing fetch.
pub struct Pacers {
    pub forum: Pacer,
    pub faptap: Pacer,
    pub instagram: Pacer,
    pub twitch: Pacer,
    pub pages: Pacer,
}

impl Pacers {
    /// No waiting with the canned network (tests).
    pub fn new(fake: bool) -> Pacers {
        let gap = |g: Duration| if fake { Duration::ZERO } else { g };
        Pacers {
            forum: Pacer::new(gap(eroscripts::GAP)),
            faptap: Pacer::new(gap(faptap::GAP)),
            instagram: Pacer::new(gap(verify::INSTAGRAM_GAP)),
            twitch: Pacer::new(gap(verify::TWITCH_GAP)),
            pages: Pacer::new(gap(PAGE_GAP)),
        }
    }
}

/// A picture to save: where it is, its source, the page it belongs to,
/// when it was taken (a post's time).
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub image: String,
    pub source: String,
    pub page: String,
    pub taken_at: Option<String>,
}

/// What checked out for a creator.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Finds {
    pub instagram: Option<Instagram>,
    pub twitch: Option<Twitch>,
    pub profiles: Vec<Profile>,
    pub pictures: Vec<Found>,
}

impl Finds {
    /// Whether anything is confirmed hers.
    pub fn any(&self) -> bool {
        self.instagram.is_some() || self.twitch.is_some() || !self.profiles.is_empty()
    }

    /// e.g. `Instagram @a (5M followers, high match), OnlyFans @a, IAFD`.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(i) = &self.instagram {
            parts.push(format!(
                "Instagram @{} ({} followers, {} match)",
                i.handle,
                i.followers.as_deref().unwrap_or("?"),
                i.match_.as_deref().unwrap_or("?")
            ));
        }
        if let Some(t) = &self.twitch {
            parts.push(format!(
                "Twitch {} ({} followers)",
                t.handle,
                t.followers.unwrap_or(0)
            ));
        }
        for p in &self.profiles {
            parts.push(format!("{} {}", p.site, p.handle));
        }
        parts.join(", ")
    }
}

/// A URL compared as a page: no scheme, `www.`, query, fragment or
/// trailing slash; lower case.
fn page_key(url: &str) -> String {
    let u = url.split(['?', '#']).next().unwrap_or(url).to_lowercase();
    let u = u
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    u.trim_start_matches("www.")
        .trim_end_matches('/')
        .to_string()
}

/// Check the judge's finds for a creator known by `names`: the first
/// Instagram and Twitch candidates that exist and match (unless she has
/// them: `has_instagram`, `has_twitch`), every profile page that answers
/// and is hers, their pictures, and the judge's pictures on a page that
/// passed.
pub fn verify(
    http: &dyn Http,
    p: &mut Pacers,
    names: &[String],
    h: &Handles,
    has_instagram: bool,
    has_twitch: bool,
    now: &str,
) -> Result<Finds, NetError> {
    let mut f = Finds::default();
    let mut passed: Vec<String> = Vec::new();
    if !has_instagram {
        for cand in &h.instagram {
            if let Some(page) = verify::instagram(http, &mut p.instagram, verify::INSTAGRAM, cand)?
            {
                let q = verify::match_quality(names, &page.handle, &page.full_name);
                if q != "low" {
                    passed.push(format!("{}/{}/", verify::INSTAGRAM, page.handle));
                    f.instagram = Some(Instagram {
                        handle: page.handle,
                        followers: Some(page.followers),
                        posts: Some(page.posts),
                        full_name: Some(page.full_name).filter(|n| !n.is_empty()),
                        bio: page.bio,
                        last_post_at: None,
                        match_: Some(q.into()),
                        checked_at: Some(now.to_string()),
                        followed: None,
                        extra: Map::new(),
                    });
                    break;
                }
            }
        }
    }
    if !has_twitch {
        for cand in &h.twitch {
            if let Some(u) = verify::twitch(http, &mut p.twitch, verify::TWITCH_GQL, cand)? {
                let q = verify::match_quality(names, &u.login, &u.display_name);
                if q != "low" {
                    passed.push(format!("https://www.twitch.tv/{}", u.login));
                    f.twitch = Some(Twitch {
                        handle: u.login,
                        followers: Some(u.followers),
                        last_live: u.last_live,
                        match_: Some(q.into()),
                        checked_at: Some(now.to_string()),
                        extra: Map::new(),
                    });
                    break;
                }
            }
        }
    }
    for url in h.profiles.iter().take(MAX_PAGES) {
        let Some((site, handle)) = site_of(url) else {
            continue;
        };
        let Some(page) = fetch_page(http, &mut p.pages, url)? else {
            continue;
        };
        let Some(q) = confirm(names, &handle, &page) else {
            continue;
        };
        passed.push(url.clone());
        for image in page_images(&page) {
            f.pictures.push(Found {
                image,
                source: picture_source(site).into(),
                page: url.clone(),
                taken_at: None,
            });
        }
        f.profiles.push(Profile {
            site: site.into(),
            url: url.clone(),
            handle,
            match_: Some(q.into()),
            checked_at: Some(now.to_string()),
            extra: Map::new(),
        });
    }
    let passed: HashSet<String> = passed.iter().map(|u| page_key(u)).collect();
    for i in &h.images {
        if passed.contains(&page_key(&i.page)) && !f.pictures.iter().any(|x| x.image == i.url) {
            f.pictures.push(Found {
                image: i.url.clone(),
                source: "web".into(),
                page: i.page.clone(),
                taken_at: None,
            });
        }
    }
    Ok(f)
}

/// Poster frames of the video pages her posts link that aren't pictured
/// yet (og:image), newest posts first.
pub fn video_pictures(
    http: &dyn Http,
    pacer: &mut Pacer,
    c: &Creator,
) -> Result<Vec<Found>, NetError> {
    let mut posts: Vec<&store::Post> = c
        .posts
        .iter()
        .filter(|p| !p.video_pages.is_empty())
        .collect();
    posts.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    let mut out = Vec::new();
    let mut read = 0;
    for post in posts {
        for url in &post.video_pages {
            if read >= MAX_VIDEO_PAGES {
                return Ok(out);
            }
            if c.pictures.iter().any(|p| p.url == *url) {
                continue;
            }
            read += 1;
            let Some(page) = fetch_page(http, pacer, url)? else {
                continue;
            };
            if let Some(image) = page_images(&page).into_iter().next() {
                out.push(Found {
                    image,
                    source: if eroscripts::host_of(url, eroscripts::FORUM) == Some("pornhub") {
                        "pornhub".into()
                    } else {
                        "web".into()
                    },
                    page: url.clone(),
                    taken_at: post.created_at.clone(),
                });
            }
        }
    }
    Ok(out)
}

/// Save `found` as her pictures (until she has the most kept besides
/// Instagram's); returns how many were added.
pub fn add_pictures(
    http: &dyn Http,
    user_data: &Path,
    slug: &str,
    c: &mut Creator,
    found: &[Found],
    out: &mut dyn Write,
) -> usize {
    let dir = store::pictures_dir(user_data, slug);
    let mut added = 0;
    for f in found {
        let others = c
            .pictures
            .iter()
            .filter(|p| p.source != "instagram")
            .count();
        if others >= store::MAX_OTHER_PICTURES {
            break;
        }
        match save_picture(http, &f.image, &dir) {
            Ok(Some(file)) => {
                if !c.pictures.iter().any(|p| p.file == file) {
                    c.pictures.push(Picture {
                        file,
                        source: f.source.clone(),
                        url: f.page.clone(),
                        taken_at: f.taken_at.clone(),
                        likes: None,
                    });
                    added += 1;
                }
            }
            Ok(None) => {}
            Err(e) => {
                let _ = writeln!(
                    out,
                    "  {slug}: picture {}: {e}",
                    f.image.split('?').next().unwrap_or("")
                );
            }
        }
    }
    store::rebuild_pictures(c);
    added
}

/// Merge newly confirmed profiles into hers (one per URL, refreshed).
pub fn merge_profiles(c: &mut Creator, found: Vec<Profile>) {
    for p in found {
        match c
            .profiles
            .iter_mut()
            .find(|x| page_key(&x.url) == page_key(&p.url))
        {
            Some(old) => *old = p,
            None => c.profiles.push(p),
        }
    }
}

/// Where a person or creator stands for `--profiles`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// A creator with enough pictures (counting the Instagram pictures
    /// still to come).
    Done,
    /// Looked up within [`MISS_DAYS`] without enough found.
    Missing,
    /// Her creator was swiped no: not looked up.
    SwipedNo,
    /// A lookup is due.
    Waiting,
}

/// Whether a creator is short of pictures. One with an Instagram account
/// whose pictures were never fetched isn't: `--pictures` brings them.
pub fn short_of_pictures(slug: &str, c: &Creator, state: &State) -> bool {
    c.pictures.len() < MIN_PICTURES && (c.instagram.is_none() || state.pictures.contains_key(slug))
}

pub fn standing(
    slug: &str,
    file: &CreatorsFile,
    state: &State,
    swipes: &Swipes,
    now: i64,
) -> Standing {
    if let Some(c) = file.creators.get(slug) {
        if swipes.creator_is_no(slug) {
            return Standing::SwipedNo;
        }
        if !short_of_pictures(slug, c, state) {
            return Standing::Done;
        }
    }
    let fresh = state
        .profile_misses
        .get(slug)
        .and_then(|t| crate::instagram::parse_iso(t))
        .is_some_and(|t| now - t < MISS_DAYS * 86_400);
    if fresh {
        Standing::Missing
    } else {
        Standing::Waiting
    }
}

/// Who `--profiles` looks up, in order: the people (swiped yes, favourites,
/// pornstar list), then the other creators short of pictures (slug order);
/// each as a person (an other creator has no source).
pub fn due(
    people: &[Person],
    file: &CreatorsFile,
    state: &State,
    swipes: &Swipes,
    now: i64,
) -> Vec<Person> {
    let mut list: Vec<Person> = people
        .iter()
        .filter(|p| standing(&p.slug, file, state, swipes, now) == Standing::Waiting)
        .cloned()
        .collect();
    for (slug, c) in &file.creators {
        if people.iter().any(|p| p.slug == *slug)
            || standing(slug, file, state, swipes, now) != Standing::Waiting
        {
            continue;
        }
        list.push(Person {
            name: if c.name.trim().is_empty() {
                slug.clone()
            } else {
                c.name.clone()
            },
            aliases: c.aliases.clone(),
            sources: Vec::new(),
            slug: slug.clone(),
        });
    }
    list
}

/// A new creator's posts: the forum search's script posts with one of her
/// names in the title, then FapTap videos by her name; [`MAX_POSTS`] at
/// most, none filed already.
#[allow(clippy::too_many_arguments)]
fn add_posts(
    f: &Filing,
    p: &mut Pacers,
    c: &mut Creator,
    slug: &str,
    names: &[String],
    file_keys: &HashSet<String>,
    state: &mut State,
    code: &mut i32,
    out: &mut dyn Write,
) -> usize {
    let mut added = 0;
    let is_new = |key: &str| !file_keys.contains(key);
    if let Some(cookie) = f.cookie {
        match eroscripts::search(f.http, &mut p.forum, eroscripts::FORUM, cookie, &c.name) {
            Ok(topics) => {
                for t in topics {
                    if added >= MAX_POSTS {
                        break;
                    }
                    let id = t.id.to_string();
                    if !is_new(&store::post_key("eroscripts", &id))
                        || c.post("eroscripts", &id).is_some()
                        || !names.iter().any(|n| faptap::occurs(n, &t.title))
                    {
                        continue;
                    }
                    let mut post = eroscripts::listed_post(&t);
                    post.score = Some(f.scoring.score(&post, &c.name));
                    c.upsert_post(post);
                    added += 1;
                    if let Err(e) = discover::add_details(f, &mut p.forum, c, slug, t.id) {
                        note_skip(&e, &format!("  EroScripts post {id}"), code, out);
                        state.pending_details.insert(id, slug.to_string());
                        if matches!(e, NetError::RateLimited(_)) {
                            break;
                        }
                    }
                }
            }
            Err(e) => note_skip(&e, &format!("  EroScripts search {}", c.name), code, out),
        }
    }
    if added < MAX_POSTS {
        match faptap::search(f.http, &mut p.faptap, faptap::SITE, &c.name) {
            Ok(videos) => {
                let mine: Vec<faptap::Video> = videos
                    .into_iter()
                    .filter(|v| faptap::by_creator(v, names))
                    .filter(|v| {
                        is_new(&store::post_key("faptap", &v.id))
                            && c.post("faptap", &v.id).is_none()
                    })
                    .take(MAX_POSTS - added)
                    .collect();
                for v in &mine {
                    discover::add_video(f, &mut p.faptap, c, slug, v);
                }
                added += mine.len();
            }
            Err(e) => note_skip(&e, &format!("  FapTap search {}", c.name), code, out),
        }
    }
    added
}

pub struct Profiles<'a> {
    pub filing: Filing<'a>,
    pub judge_cmd: String,
    /// Canned network (tests): no pacing.
    pub fake: bool,
    pub library: &'a LibraryIds,
    /// Lookups this run, at most.
    pub limit: usize,
}

fn sources_text(p: &Person) -> String {
    if p.sources.is_empty() {
        return "creator".into();
    }
    p.sources
        .iter()
        .map(|s| match *s {
            "swiped" => "swiped yes",
            "favourite" => "favourite",
            _ => "pornstar list",
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// `--profiles N`. Writes creators.json and the state after each person.
pub fn run(
    f: &Profiles,
    people: &[Person],
    file: &mut CreatorsFile,
    state: &mut State,
    swipes: &Swipes,
    out: &mut dyn Write,
) -> Result<i32, String> {
    let user_data = f.filing.user_data;
    let http = f.filing.http;
    let mut p = Pacers::new(f.fake);
    let now = video_tags::timefmt::now_iso();
    let now_s = now_secs();
    let mut code = 0;
    let changed = merge_sources(file, people);
    let due = due(people, file, state, swipes, now_s);
    let todo: Vec<&Person> = due.iter().take(f.limit).collect();
    let left = due.len() - todo.len();
    let _ = writeln!(
        out,
        "Profiles: {} people ({} swiped yes, {} favourite(s), {} on the pornstar list); {} to look up{}.",
        people.len(),
        people
            .iter()
            .filter(|x| x.sources.contains(&"swiped"))
            .count(),
        people
            .iter()
            .filter(|x| x.sources.contains(&"favourite"))
            .count(),
        people
            .iter()
            .filter(|x| x.sources.contains(&"pornstar"))
            .count(),
        due.len(),
        if left > 0 {
            format!(", {} this run", todo.len())
        } else {
            String::new()
        }
    );
    let (mut looked, mut confirmed, mut missed, mut created, mut pictured) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    if !todo.is_empty() {
        if f.filing.cookie.is_none() {
            let _ = writeln!(
                out,
                "EroScripts: no session cookie (log in to EroScripts in the app); forum posts skipped."
            );
        }
        let items: Vec<PersonItem> = todo
            .iter()
            .map(|x| PersonItem {
                name: x.name.clone(),
                aliases: x.aliases.clone(),
                sources: x.sources.iter().map(|s| s.to_string()).collect(),
                posts: file
                    .creators
                    .get(&x.slug)
                    .map(|c| c.posts.iter().take(10).map(|p| p.title.clone()).collect())
                    .unwrap_or_default(),
            })
            .collect();
        let work = user_data.join(judge::WORK_DIR);
        let batches = judge::write_people(&work, &items)?;
        let _ = writeln!(
            out,
            "Asking the judge about {} people in {} batch(es)...",
            items.len(),
            batches.len()
        );
        if let Err(e) = judge::run(&f.judge_cmd, &judge::people_prompt(&batches), &work) {
            let _ = writeln!(out, "Judge: {e}; reading what it wrote.");
        }
        // A batch without a results file is asked again next run; the
        // others count.
        let mut results: BTreeMap<String, Handles> = BTreeMap::new();
        for b in &batches {
            match judge::read_people_results(&b.results, &b.names) {
                Ok(r) => results.extend(r.into_iter().map(|h| (store::slug(&h.name), h))),
                Err(e) => {
                    let _ = writeln!(
                        out,
                        "Judge, {}: {e}; these people are looked up again next run.",
                        b.results.display()
                    );
                    code = combine(code, 2);
                }
            }
        }
        if results.is_empty() {
            store::write(user_data, file)?;
            crate::state::write(user_data, state)?;
            return Ok(combine(code, 1));
        }
        for person in todo {
            let Some(h) = results.get(&store::slug(&person.name)).filter(|h| {
                !(h.instagram.is_empty()
                    && h.twitch.is_empty()
                    && h.profiles.is_empty()
                    && h.images.is_empty())
            }) else {
                // Not researched (no entry, or an empty one): due next run,
                // not marked missing.
                let _ = writeln!(
                    out,
                    "  {}: the judge found nothing to check; next run.",
                    person.name
                );
                code = combine(code, 2);
                continue;
            };
            let names = person.names();
            let existing = file.creators.get(&person.slug);
            let finds = match verify(
                http,
                &mut p,
                &names,
                h,
                existing.is_some_and(|c| c.instagram.is_some()),
                existing.is_some_and(|c| c.twitch.is_some()),
                &now,
            ) {
                Ok(finds) => finds,
                Err(e) => {
                    note_skip(&e, &format!("  {}", person.name), &mut code, out);
                    if matches!(e, NetError::RateLimited(_)) {
                        break;
                    }
                    continue;
                }
            };
            looked += 1;
            if finds.any() {
                confirmed += 1;
            }
            let what = finds.describe();
            let (c, new_posts) = match file.creators.remove(&person.slug) {
                Some(mut c) => {
                    if finds.instagram.is_some() {
                        c.instagram = finds.instagram.clone();
                    }
                    if finds.twitch.is_some() {
                        c.twitch = finds.twitch.clone();
                    }
                    (Some(c), None)
                }
                None if finds.any() => {
                    let mut c = Creator {
                        name: person.name.clone(),
                        aliases: person.aliases.clone(),
                        first_seen: Some(now.clone()),
                        instagram: finds.instagram.clone(),
                        twitch: finds.twitch.clone(),
                        sources: person.sources.iter().map(|s| s.to_string()).collect(),
                        ..Creator::default()
                    };
                    let keys = file.post_keys();
                    let n = add_posts(
                        &f.filing,
                        &mut p,
                        &mut c,
                        &person.slug,
                        &names,
                        &keys,
                        state,
                        &mut code,
                        out,
                    );
                    created += 1;
                    (Some(c), Some(n))
                }
                None => (None, None),
            };
            let Some(mut c) = c else {
                missed += 1;
                state
                    .profile_misses
                    .insert(person.slug.clone(), now.clone());
                let _ = writeln!(
                    out,
                    "  {} ({}): nothing confirmed hers; looked up again in {MISS_DAYS} days.",
                    person.name,
                    sources_text(person)
                );
                store::write(user_data, file)?;
                crate::state::write(user_data, state)?;
                continue;
            };
            merge_profiles(&mut c, finds.profiles.clone());
            let mut pictures = finds.pictures.clone();
            match video_pictures(http, &mut p.pages, &c) {
                Ok(v) => pictures.extend(v),
                Err(e) => note_skip(
                    &e,
                    &format!("  {}: video pages", person.name),
                    &mut code,
                    out,
                ),
            }
            let added = add_pictures(http, user_data, &person.slug, &mut c, &pictures, out);
            pictured += added;
            let _ = writeln!(
                out,
                "  {}{} ({}): {}{}; {added} picture(s) added.",
                if new_posts.is_some() {
                    "New creator "
                } else {
                    ""
                },
                person.name,
                sources_text(person),
                if what.is_empty() {
                    "nothing new confirmed".into()
                } else {
                    what
                },
                new_posts.map_or(String::new(), |n| format!("; {n} post(s)"))
            );
            if short_of_pictures(&person.slug, &c, state) {
                state
                    .profile_misses
                    .insert(person.slug.clone(), now.clone());
            } else {
                state.profile_misses.remove(&person.slug);
            }
            f.library.mark_creator(&mut c);
            c.update_score();
            store::prune(user_data, &person.slug, &c);
            file.creators.insert(person.slug.clone(), c);
            store::write(user_data, file)?;
            crate::state::write(user_data, state)?;
        }
    }
    merge_sources(file, people);
    store::write(user_data, file)?;
    crate::state::write(user_data, state)?;
    let _ = writeln!(
        out,
        "Profiles: {looked} looked up, {confirmed} with a confirmed profile, {missed} not found, {created} new creator(s), {pictured} picture(s) added; sources changed on {changed} creator(s); {left} waiting."
    );
    if left > 0 {
        code = combine(code, 2);
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Canned;

    fn names(n: &str) -> Vec<String> {
        vec![n.to_string()]
    }

    #[test]
    fn sites_and_handles_of_profile_urls() {
        let cases = [
            (
                "https://onlyfans.com/sweetiefox",
                Some(("onlyfans", "sweetiefox")),
            ),
            (
                "https://fansly.com/SweetieFox/posts",
                Some(("fansly", "SweetieFox")),
            ),
            (
                "https://www.pornhub.com/model/sweetie-fox",
                Some(("pornhub", "sweetie-fox")),
            ),
            (
                "https://de.pornhub.com/pornstar/candy-love?o=mr",
                Some(("pornhub", "candy-love")),
            ),
            ("https://www.pornhub.com/view_video.php?viewkey=1", None),
            ("https://x.com/sweetiefox_of", Some(("x", "sweetiefox_of"))),
            ("https://twitter.com/i/flow/login", None),
            (
                "https://www.reddit.com/user/SweetieFox/",
                Some(("reddit", "SweetieFox")),
            ),
            ("https://www.reddit.com/r/nsfw", None),
            (
                "https://www.manyvids.com/Profile/123/Sweetie-Fox/Store",
                Some(("manyvids", "Sweetie-Fox")),
            ),
            (
                "https://linktr.ee/sweetiefox",
                Some(("linktree", "sweetiefox")),
            ),
            (
                "https://www.iafd.com/person.rme/id=1/sweetie-fox.htm",
                Some(("iafd", "sweetie-fox")),
            ),
            (
                "https://www.babepedia.com/babe/Sweetie_Fox",
                Some(("babepedia", "Sweetie_Fox")),
            ),
            (
                "https://www.freeones.com/sweetie-fox/bio",
                Some(("freeones", "sweetie-fox")),
            ),
            (
                "https://www.freeones.com/de/sweetie-fox",
                Some(("freeones", "sweetie-fox")),
            ),
            ("https://sweetiefox.com/", Some(("profile", "sweetiefox"))),
            ("https://www.instagram.com/sweetiefox/", None),
            ("http://onlyfans.com/x", None),
        ];
        for (url, want) in cases {
            let got = site_of(url);
            assert_eq!(got.as_ref().map(|(s, h)| (*s, h.as_str())), want, "{url}");
        }
    }

    #[test]
    fn confirms_pages_by_their_own_names() {
        let of = r#"<html><head><title>Sweetie Fox OnlyFans</title>
            <meta property="og:title" content="Sweetie Fox (@sweetiefox)">
            <meta property="og:image" content="https://cdn.of/avatar.jpg"></head></html>"#;
        assert_eq!(
            confirm(&names("Sweetie Fox"), "sweetiefox", of),
            Some("high")
        );
        assert_eq!(page_images(of), ["https://cdn.of/avatar.jpg"]);
        // The page doesn't show the URL's handle: only its own name counts.
        let generic = r#"<title>OnlyFans</title><meta property="og:title" content="OnlyFans">"#;
        assert_eq!(confirm(&names("Sweetie Fox"), "sweetiefox", generic), None);
        // A Pornhub model page names her in the h1.
        let ph = r#"<title>Free Porn Videos - Pornhub</title><h1><span>Candy</span> Love</h1>"#;
        assert_eq!(
            confirm(&names("Candy Love"), "candy-love", ph),
            Some("high")
        );
        // A word of her name only isn't enough on a profile page.
        let medium = r#"<meta property="og:title" content="Candy official / X">"#;
        assert_eq!(confirm(&names("Candy Love"), "candyofficial", medium), None);
        // Login walls and error pages aren't profiles, whatever they say.
        for wall in [
            r#"<title>Log in to see Sweetie Fox</title>"#,
            r#"<title>Page Not Found | Sweetie Fox</title>"#,
            r#"<title>Just a moment...</title>"#,
        ] {
            assert_eq!(
                confirm(&names("Sweetie Fox"), "sweetiefox", wall),
                None,
                "{wall}"
            );
        }
        // Someone else.
        let other = r#"<meta property="og:title" content="Jane Doe (@janedoe)">"#;
        assert_eq!(confirm(&names("Sweetie Fox"), "janedoe", other), None);
        assert_eq!(
            page_names(r#"<title> A &amp; B </title><h1>A<br>B</h1>"#),
            ["A & B", "AB"]
        );
    }

    pub fn gif(w: u16, h: u16) -> Vec<u8> {
        let mut b = b"GIF89a".to_vec();
        b.extend(w.to_le_bytes());
        b.extend(h.to_le_bytes());
        b.extend([0; 8]);
        b
    }

    #[test]
    fn image_sizes_and_the_keep_rule() {
        assert_eq!(image_size(&gif(400, 600)), Some(("gif", 400, 600)));
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(1080u32.to_be_bytes());
        png.extend(1350u32.to_be_bytes());
        assert_eq!(image_size(&png), Some(("png", 1080, 1350)));
        // JPEG: an APP0 segment, then SOF0 (height before width).
        let mut jpg = vec![
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0, 0, 0xff, 0xc0, 0x00, 0x11, 8,
        ];
        jpg.extend(720u16.to_be_bytes());
        jpg.extend(1280u16.to_be_bytes());
        jpg.extend([0; 8]);
        assert_eq!(image_size(&jpg), Some(("jpg", 1280, 720)));
        let mut vp8x = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0".to_vec();
        vp8x.extend([0x7f, 0x02, 0x00, 0xff, 0x01, 0x00]);
        assert_eq!(image_size(&vp8x), Some(("webp", 640, 512)));
        assert_eq!(image_size(b"<html>not a picture</html>"), None);
        assert!(keep_size(400, 600));
        assert!(!keep_size(299, 1000), "too small");
        assert!(!keep_size(1920, 300), "a banner");
        assert!(keep_size(900, 300));
    }

    #[test]
    fn saves_only_real_big_pictures() {
        let dir =
            std::env::temp_dir().join(format!("creator-scout-profiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut net = Canned::default();
        let body = |b: Vec<u8>| String::from_utf8(b).unwrap();
        net.on("GET", "https://cdn/a.gif", 200, &body(gif(344, 600)));
        net.on("GET", "https://cdn/small.gif", 200, &body(gif(100, 100)));
        net.on("GET", "https://cdn/banner.gif", 200, &body(gif(1919, 300)));
        net.on("GET", "https://cdn/page.html", 200, "<html></html>");
        let a = save_picture(&net, "https://cdn/a.gif", &dir)
            .unwrap()
            .unwrap();
        assert!(a.starts_with("web-") && a.ends_with(".gif"));
        assert!(dir.join(&a).is_file());
        for bad in [
            "https://cdn/small.gif",
            "https://cdn/banner.gif",
            "https://cdn/page.html",
        ] {
            assert_eq!(save_picture(&net, bad, &dir).unwrap(), None, "{bad}");
        }
        // Saved once: no second download.
        let seen = net.seen.borrow().len();
        assert_eq!(
            save_picture(&net, "https://cdn/a.gif", &dir).unwrap(),
            Some(a)
        );
        assert_eq!(net.seen.borrow().len(), seen);
        let files: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(files.len(), 1, "{files:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verifies_profiles_and_keeps_pictures_from_pages_that_passed() {
        let mut net = Canned::default();
        net.on(
            "GET",
            "https://onlyfans.com/candylove",
            200,
            r#"<meta property="og:title" content="Candy Love"><meta property="og:image" content="https://cdn/of.jpg">"#,
        );
        net.on(
            "GET",
            "https://x.com/janedoe_x",
            200,
            r#"<meta property="og:title" content="Jane Doe (@janedoe_x)">"#,
        );
        net.on(
            "GET",
            "https://www.iafd.com/person.rme/id=1/candy-love.htm",
            404,
            "",
        );
        let h = Handles {
            name: "Candy Love".into(),
            profiles: vec![
                "https://onlyfans.com/candylove".into(),
                "https://x.com/janedoe_x".into(),
                "https://www.iafd.com/person.rme/id=1/candy-love.htm".into(),
                "https://www.instagram.com/candylove/".into(),
            ],
            images: vec![
                judge::Image {
                    url: "https://cdn/j1.jpg".into(),
                    page: "https://onlyfans.com/candylove/".into(),
                },
                judge::Image {
                    url: "https://cdn/j2.jpg".into(),
                    page: "https://x.com/janedoe_x".into(),
                },
            ],
            ..Handles::default()
        };
        let mut p = Pacers::new(true);
        let f = verify(&net, &mut p, &names("Candy Love"), &h, false, false, "NOW").unwrap();
        assert!(f.any() && f.instagram.is_none() && f.twitch.is_none());
        assert_eq!(f.profiles.len(), 1);
        assert_eq!(
            (
                f.profiles[0].site.as_str(),
                f.profiles[0].handle.as_str(),
                f.profiles[0].match_.as_deref()
            ),
            ("onlyfans", "candylove", Some("high"))
        );
        let pics: Vec<(&str, &str)> = f
            .pictures
            .iter()
            .map(|x| (x.image.as_str(), x.source.as_str()))
            .collect();
        // The judge's picture on the page that failed is dropped.
        assert_eq!(
            pics,
            [
                ("https://cdn/of.jpg", "onlyfans"),
                ("https://cdn/j1.jpg", "web")
            ]
        );
        assert_eq!(f.describe(), "onlyfans candylove");
        // The OnlyFans page went as a link-preview bot.
        let seen = net.seen.borrow();
        let of = seen.iter().find(|r| r.url.contains("onlyfans")).unwrap();
        assert!(
            of.headers
                .contains(&("User-Agent".into(), verify::PREVIEW_UA.into()))
        );
    }

    #[test]
    fn poster_frames_of_linked_video_pages() {
        let mut net = Canned::default();
        net.on(
            "GET",
            "https://www.pornhub.com/view_video.php?viewkey=a",
            200,
            r#"<meta property="og:image" content="https://ci.phncdn.com/a.jpg">"#,
        );
        net.on(
            "GET",
            "https://www.sexlikereal.com/scenes/b",
            200,
            r#"<meta property="og:image" content="https://slr/b.jpg">"#,
        );
        let c = Creator {
            posts: vec![
                store::Post {
                    created_at: Some("2026-11-01T00:00:00Z".into()),
                    video_pages: vec!["https://www.pornhub.com/view_video.php?viewkey=a".into()],
                    ..store::Post::default()
                },
                store::Post {
                    created_at: Some("2026-11-05T00:00:00Z".into()),
                    video_pages: vec!["https://www.sexlikereal.com/scenes/b".into()],
                    ..store::Post::default()
                },
            ],
            pictures: vec![],
            ..Creator::default()
        };
        let got = video_pictures(&net, &mut Pacer::new(Duration::ZERO), &c).unwrap();
        let v: Vec<(&str, &str, Option<&str>)> = got
            .iter()
            .map(|x| (x.image.as_str(), x.source.as_str(), x.taken_at.as_deref()))
            .collect();
        assert_eq!(
            v,
            [
                ("https://slr/b.jpg", "web", Some("2026-11-05T00:00:00Z")),
                (
                    "https://ci.phncdn.com/a.jpg",
                    "pornhub",
                    Some("2026-11-01T00:00:00Z")
                )
            ]
        );
    }

    #[test]
    fn who_is_due() {
        let mut file = CreatorsFile::empty();
        let pic = || Picture {
            file: "x.jpg".into(),
            source: "web".into(),
            url: "u".into(),
            taken_at: None,
            likes: None,
        };
        let mk = |name: &str, ig: bool, pics: usize| Creator {
            name: name.into(),
            instagram: ig.then(Instagram::default),
            pictures: (0..pics).map(|_| pic()).collect(),
            ..Creator::default()
        };
        file.creators.insert("full".into(), mk("Full", false, 3));
        file.creators.insert("seed".into(), mk("Seed", true, 0));
        file.creators
            .insert("fetched".into(), mk("Fetched", true, 1));
        file.creators.insert("bare".into(), mk("Bare", false, 0));
        file.creators.insert("nope".into(), mk("Nope", false, 0));
        file.creators
            .insert("missed".into(), mk("Missed", false, 1));
        let mut state = State::default();
        state
            .pictures
            .insert("fetched".into(), "2026-11-10T00:00:00Z".into());
        state
            .profile_misses
            .insert("missed".into(), "2026-11-10T00:00:00Z".into());
        state
            .profile_misses
            .insert("oldmiss".into(), "2026-09-01T00:00:00Z".into());
        let mut swipes = Swipes::default();
        swipes.creators.insert("nope".into(), "no".into());
        let now = crate::instagram::parse_iso("2026-11-18T00:00:00Z").unwrap();
        let person = |name: &str, slug: &str| Person {
            name: name.into(),
            aliases: vec![],
            sources: vec!["favourite"],
            slug: slug.into(),
        };
        let people = [
            person("Old Miss", "oldmiss"),
            person("Full", "full"),
            person("New One", "newone"),
        ];
        let got: Vec<String> = due(&people, &file, &state, &swipes, now)
            .into_iter()
            .map(|p| p.slug)
            .collect();
        // People first; Seed waits for her Instagram pictures, Fetched got
        // them and is still short, Missed was looked up 8 days ago.
        assert_eq!(got, ["oldmiss", "newone", "bare", "fetched"]);
        assert_eq!(
            standing("nope", &file, &state, &swipes, now),
            Standing::SwipedNo
        );
        assert_eq!(
            standing("missed", &file, &state, &swipes, now),
            Standing::Missing
        );
        assert_eq!(
            standing("full", &file, &state, &swipes, now),
            Standing::Done
        );
    }
}
