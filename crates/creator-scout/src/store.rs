//! `<userData>/creators.json` (written only by this CLI, read by the
//! Creators site), its picture folders, and the name rules both share.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "creators.json";
pub const BACKUP_NAME: &str = "creators.json.bak";
pub const VERSION: u64 = 1;
/// `<userData>/creator-pictures/<slug>/<file>`.
pub const PICTURES_DIR: &str = "creator-pictures";
pub const FORUM_SITE: &str = "https://discuss.eroscripts.com";
pub const FAPTAP_SITE: &str = "https://faptap.net";
pub const INSTAGRAM_SITE: &str = "https://www.instagram.com";
/// A creator's sources (`Creator::sources`), in this order.
pub const SOURCES: [&str; 3] = ["swiped", "favourite", "pornstar"];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EroscriptsChecked {
    pub newest_topic_id: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaptapChecked {
    pub newest_video_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Instagram {
    pub handle: String,
    #[serde(default)]
    pub followers: Option<String>,
    #[serde(default)]
    pub posts: Option<String>,
    #[serde(default)]
    pub full_name: Option<String>,
    #[serde(default)]
    pub bio: Option<String>,
    /// The newest timeline post (only known with a login).
    #[serde(default)]
    pub last_post_at: Option<String>,
    /// How well the account matches the creator's name: high / medium.
    #[serde(default, rename = "match")]
    pub match_: Option<String>,
    #[serde(default)]
    pub checked_at: Option<String>,
    /// Our Instagram account follows her: following / requested (`--follow`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub followed: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Twitch {
    pub handle: String,
    #[serde(default)]
    pub followers: Option<u64>,
    #[serde(default)]
    pub last_live: Option<String>,
    #[serde(default, rename = "match")]
    pub match_: Option<String>,
    #[serde(default)]
    pub checked_at: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A public profile page confirmed hers (`--discover`, `--profiles`): its
/// own name or handle matches hers (the verify rule) and it answered.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// onlyfans / fansly / pornhub / x / reddit / manyvids / linktree /
    /// iafd / babepedia / boobpedia / freeones / profile (another page).
    pub site: String,
    pub url: String,
    #[serde(default)]
    pub handle: String,
    /// high / medium.
    #[serde(default, rename = "match")]
    pub match_: Option<String>,
    #[serde(default)]
    pub checked_at: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Picture {
    /// A file name in the creator's picture folder.
    pub file: String,
    /// instagram / eroscripts / faptap (post images), pornhub / web (video
    /// thumbnails and pictures the judge found), onlyfans / fansly / x /
    /// reddit / profile (a confirmed profile's own picture).
    pub source: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taken_at: Option<String>,
    /// An Instagram timeline post's likes (`like_count`), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub likes: Option<u64>,
}

impl Picture {
    /// A post's own image (rebuilt from the posts every time).
    fn is_post_image(&self) -> bool {
        self.source == "eroscripts" || self.source == "faptap"
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Post {
    /// eroscripts / faptap.
    pub kind: String,
    pub id: String,
    pub title: String,
    pub url: String,
    pub free: bool,
    #[serde(default)]
    pub hosts: Vec<String>,
    pub downloadable: bool,
    #[serde(default)]
    pub creampie: bool,
    #[serde(default)]
    pub multi_axis: bool,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub likes: u64,
    #[serde(default)]
    pub tags: Vec<String>,
    /// A file name in the creator's picture folder.
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub duration_s: Option<u64>,
    #[serde(default)]
    pub in_library: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// The post's public video pages (Pornhub, rule34video, SexLikeReal):
    /// their poster frames are pictures of her.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub video_pages: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Post {
    /// `eroscripts:41415`: the key creator-swipes.json uses.
    pub fn key(&self) -> String {
        post_key(&self.kind, &self.id)
    }
}

pub fn post_key(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Creator {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub first_seen: Option<String>,
    /// The average of the posts' scores; null when none is scored.
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub instagram: Option<Instagram>,
    #[serde(default)]
    pub twitch: Option<Twitch>,
    /// Her other public profiles confirmed hers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<Profile>,
    /// Newest first.
    #[serde(default)]
    pub pictures: Vec<Picture>,
    #[serde(default)]
    pub posts: Vec<Post>,
    /// Why the user wants her, set by `--favourites`: `swiped` (yes on the
    /// Creators site), `favourite` (a favourite artist), `pornstar` (the
    /// pornstar list), in that order; empty for a creator only discovered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Creator {
    pub fn post(&self, kind: &str, id: &str) -> Option<&Post> {
        self.posts.iter().find(|p| p.kind == kind && p.id == id)
    }

    /// Add `post`, or refresh the one with its key (keeping its image when
    /// the new one has none). Returns whether it was new.
    pub fn upsert_post(&mut self, mut post: Post) -> bool {
        match self
            .posts
            .iter_mut()
            .find(|p| p.kind == post.kind && p.id == post.id)
        {
            Some(old) => {
                if post.image.is_none() {
                    post.image = old.image.clone();
                }
                if post.created_at.is_none() {
                    post.created_at = old.created_at.clone();
                }
                *old = post;
                false
            }
            None => {
                self.posts.push(post);
                true
            }
        }
    }

    /// `score` = the average of the scored posts, two decimals.
    pub fn update_score(&mut self) {
        let scores: Vec<f64> = self.posts.iter().filter_map(|p| p.score).collect();
        self.score = (!scores.is_empty()).then(|| {
            let avg = scores.iter().sum::<f64>() / scores.len() as f64;
            (avg * 100.0).round() / 100.0
        });
    }

    /// The name and aliases as slugs.
    pub fn slugs(&self) -> impl Iterator<Item = String> + '_ {
        std::iter::once(&self.name)
            .chain(&self.aliases)
            .map(|n| slug(n))
            .filter(|s| !s.is_empty())
    }

    /// Her first source's place in [`SOURCES`] (swiped 0, favourite 1,
    /// pornstar 2); `SOURCES.len()` for a creator only discovered.
    pub fn source_rank(&self) -> usize {
        self.sources
            .iter()
            .filter_map(|s| SOURCES.iter().position(|x| x == s))
            .min()
            .unwrap_or(SOURCES.len())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatorsFile {
    pub version: u64,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub eroscripts_checked: Option<EroscriptsChecked>,
    #[serde(default)]
    pub faptap_checked: Option<FaptapChecked>,
    #[serde(default)]
    pub creators: BTreeMap<String, Creator>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl CreatorsFile {
    pub fn empty() -> Self {
        CreatorsFile {
            version: VERSION,
            updated_at: None,
            eroscripts_checked: None,
            faptap_checked: None,
            creators: BTreeMap::new(),
            extra: Map::new(),
        }
    }

    /// The creator a name (or alias) belongs to, by slug.
    pub fn find(&self, name: &str) -> Option<String> {
        let s = slug(name);
        if s.is_empty() {
            return None;
        }
        if self.creators.contains_key(&s) {
            return Some(s);
        }
        self.creators
            .iter()
            .find(|(_, c)| c.slugs().any(|x| x == s))
            .map(|(k, _)| k.clone())
    }

    /// Every post key (`kind:id`) in the file.
    pub fn post_keys(&self) -> HashSet<String> {
        self.creators
            .values()
            .flat_map(|c| c.posts.iter().map(Post::key))
            .collect()
    }

    pub fn post_count(&self) -> usize {
        self.creators.values().map(|c| c.posts.len()).sum()
    }
}

/// A creator's key: the name in lower case with every non-alphanumeric
/// removed (accents folded: `Zoë Lynn` → `zoelynn`).
pub fn slug(name: &str) -> String {
    slr_scout::text::normalise(name).replace(' ', "")
}

/// Whether `name` is safe as a picture file or folder name: only
/// `[A-Za-z0-9._-]`, not starting with a dot.
pub fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

fn safe_id(id: &str) -> Option<&str> {
    (!id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)))
    .then_some(id)
}

/// `ig-<media id>.jpg`.
pub fn instagram_picture(id: &str) -> Option<String> {
    safe_id(id).map(|id| format!("ig-{id}.jpg"))
}

/// The profile photo saved without a login.
pub const INSTAGRAM_PROFILE_PICTURE: &str = "ig-profile.jpg";

/// `es-<topic>-<n>.jpg`: a post's n-th image.
pub fn eroscripts_picture(topic: u64, n: usize) -> String {
    format!("es-{topic}-{n}.jpg")
}

/// `ft-<video id>.jpg`.
pub fn faptap_picture(id: &str) -> Option<String> {
    safe_id(id).map(|id| format!("ft-{id}.jpg"))
}

/// `web-<hash of the URL>.<ext>`: a picture from a public page.
pub fn web_picture(url: &str, ext: &str) -> String {
    let h = url.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("web-{h:016x}.{ext}")
}

/// Pictures a creator keeps besides Instagram's, at most.
pub const MAX_OTHER_PICTURES: usize = 9;

pub fn file_path(user_data: &Path) -> PathBuf {
    user_data.join(FILE_NAME)
}

pub fn pictures_dir(user_data: &Path, slug: &str) -> PathBuf {
    user_data.join(PICTURES_DIR).join(slug)
}

/// `Ok(None)` when the file doesn't exist; an error when it isn't ours
/// (never overwrite what we don't understand).
pub fn read(user_data: &Path) -> Result<Option<CreatorsFile>, String> {
    let path = file_path(user_data);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let file: CreatorsFile = serde_json::from_str(&raw).map_err(|e| {
        format!(
            "{} is not a creators file ({e}); move it aside and re-run",
            path.display()
        )
    })?;
    if file.version != VERSION {
        return Err(format!(
            "{} is version {}, this CLI writes version {VERSION}",
            path.display(),
            file.version
        ));
    }
    Ok(Some(file))
}

/// Write atomically (temp + rename), the previous copy kept as `.bak`.
pub fn write(user_data: &Path, file: &mut CreatorsFile) -> Result<(), String> {
    file.updated_at = Some(video_tags::timefmt::now_iso());
    video_tags::store::write_json_atomic(user_data, FILE_NAME, BACKUP_NAME, file).map(|_| ())
}

/// A creator's pictures: Instagram's first (newest first), then her
/// confirmed profiles' own pictures (undated, as found), then the post
/// images (EroScripts images, FapTap thumbnails) and video poster frames,
/// newest first, a picture without a time last; at most
/// [`MAX_OTHER_PICTURES`] besides Instagram's.
pub fn rebuild_pictures(c: &mut Creator) {
    let mut kept: Vec<Picture> = c
        .pictures
        .iter()
        .filter(|p| !p.is_post_image())
        .cloned()
        .collect();
    for post in &c.posts {
        let Some(file) = &post.image else { continue };
        if kept.iter().any(|p| p.file == *file) {
            continue;
        }
        kept.push(Picture {
            file: file.clone(),
            source: post.kind.clone(),
            url: post.url.clone(),
            taken_at: post.created_at.clone(),
            likes: None,
        });
    }
    // ISO times sort as text; None last.
    let newest_first = |a: &Picture, b: &Picture| match (&a.taken_at, &b.taken_at) {
        (Some(x), Some(y)) => y.cmp(x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    };
    let (mut pictures, rest): (Vec<Picture>, Vec<Picture>) =
        kept.into_iter().partition(|p| p.source == "instagram");
    pictures.sort_by(newest_first);
    let (profile, mut dated): (Vec<Picture>, Vec<Picture>) = rest
        .into_iter()
        .partition(|p| p.taken_at.is_none() && !p.is_post_image());
    dated.sort_by(newest_first);
    pictures.extend(profile.into_iter().chain(dated).take(MAX_OTHER_PICTURES));
    c.pictures = pictures;
}

/// Delete the files in a creator's picture folder that neither a picture
/// nor a post names; returns how many.
pub fn prune(user_data: &Path, slug: &str, c: &Creator) -> usize {
    let keep: HashSet<&str> = c
        .pictures
        .iter()
        .map(|p| p.file.as_str())
        .chain(c.posts.iter().filter_map(|p| p.image.as_deref()))
        .collect();
    let Ok(entries) = fs::read_dir(pictures_dir(user_data, slug)) else {
        return 0;
    };
    let mut n = 0;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !keep.contains(name.as_str())
            && e.file_type().is_ok_and(|t| t.is_file())
            && fs::remove_file(e.path()).is_ok()
        {
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Sweetie Fox"), "sweetiefox");
        assert_eq!(slug("Reina_Ohara"), "reinaohara");
        assert_eq!(slug("Zoë Lynn!"), "zoelynn");
        assert_eq!(slug("  "), "");
    }

    #[test]
    fn picture_names() {
        assert_eq!(
            instagram_picture("3456789").as_deref(),
            Some("ig-3456789.jpg")
        );
        assert_eq!(instagram_picture("../x"), None);
        assert_eq!(eroscripts_picture(41415, 0), "es-41415-0.jpg");
        assert_eq!(
            faptap_picture("2122637633308856321").as_deref(),
            Some("ft-2122637633308856321.jpg")
        );
        assert_eq!(faptap_picture(""), None);
        assert!(safe_name("ig-1.jpg"));
        assert!(!safe_name("../a.jpg"));
        assert!(!safe_name(".hidden"));
        assert!(!safe_name("a b.jpg"));
    }

    #[test]
    fn reads_unknown_fields_and_finds_by_alias() {
        let raw = r#"{"version":1,"other":1,"creators":{"sweetiefox":{"name":"Sweetie Fox","aliases":["SweetieFoxx"],"x":2,
            "posts":[{"kind":"faptap","id":"1","title":"t","url":"u","free":true,"downloadable":true,"score":3.0},
                     {"kind":"eroscripts","id":"2","title":"t","url":"u","free":true,"downloadable":false,"score":4.0}]}}}"#;
        let mut f: CreatorsFile = serde_json::from_str(raw).unwrap();
        assert_eq!(f.find("sweetie-foxx").as_deref(), Some("sweetiefox"));
        assert_eq!(f.find("Sweetie Fox").as_deref(), Some("sweetiefox"));
        assert_eq!(f.find("Other"), None);
        let c = f.creators.get_mut("sweetiefox").unwrap();
        c.update_score();
        assert_eq!(c.score, Some(3.5));
        let back = serde_json::to_value(&f).unwrap();
        assert_eq!(back["other"], 1);
        assert_eq!(back["creators"]["sweetiefox"]["x"], 2);
        assert_eq!(back["creators"]["sweetiefox"]["instagram"], Value::Null);
    }

    #[test]
    fn pictures_newest_first_after_instagram_and_pruned() {
        let dir = std::env::temp_dir().join(format!("creator-scout-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut c = Creator {
            name: "A".into(),
            pictures: vec![Picture {
                file: "ig-1.jpg".into(),
                source: "instagram".into(),
                url: "i".into(),
                taken_at: Some("2026-11-10T00:00:00Z".into()),
                likes: Some(120),
            }],
            posts: vec![
                Post {
                    kind: "eroscripts".into(),
                    id: "5".into(),
                    url: "e".into(),
                    image: Some("es-5-0.jpg".into()),
                    created_at: Some("2026-11-12T00:00:00Z".into()),
                    ..Post::default()
                },
                Post {
                    kind: "faptap".into(),
                    id: "9".into(),
                    url: "f".into(),
                    image: Some("ft-9.jpg".into()),
                    ..Post::default()
                },
            ],
            ..Creator::default()
        };
        rebuild_pictures(&mut c);
        let files: Vec<&str> = c.pictures.iter().map(|p| p.file.as_str()).collect();
        assert_eq!(files, ["ig-1.jpg", "es-5-0.jpg", "ft-9.jpg"]);
        assert_eq!(c.pictures[1].source, "eroscripts");
        assert_eq!(c.pictures[0].likes, Some(120), "likes survive a rebuild");
        assert_eq!(c.pictures[1].likes, None);
        let pics = pictures_dir(&dir, "a");
        fs::create_dir_all(&pics).unwrap();
        for f in ["ig-1.jpg", "ig-old.jpg", "es-5-0.jpg", "ft-9.jpg"] {
            fs::write(pics.join(f), "x").unwrap();
        }
        assert_eq!(prune(&dir, "a", &c), 1);
        assert!(!pics.join("ig-old.jpg").exists());
        assert!(pics.join("ft-9.jpg").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn profile_pictures_before_post_ones_nine_at_most() {
        let pic = |file: &str, source: &str, at: Option<&str>| Picture {
            file: file.into(),
            source: source.into(),
            url: "u".into(),
            taken_at: at.map(str::to_string),
            likes: None,
        };
        let mut pictures = vec![
            pic("ig-1.jpg", "instagram", Some("2026-11-01T00:00:00Z")),
            pic("web-a.jpg", "onlyfans", None),
            pic("web-b.jpg", "pornhub", Some("2026-10-01T00:00:00Z")),
        ];
        for i in 0..10 {
            pictures.push(pic(
                &format!("web-{i}.jpg"),
                "web",
                Some(&format!("2026-09-0{}T00:00:00Z", i.min(8) + 1)),
            ));
        }
        let mut c = Creator {
            pictures,
            posts: vec![Post {
                kind: "eroscripts".into(),
                id: "5".into(),
                image: Some("es-5-0.jpg".into()),
                created_at: Some("2026-11-12T00:00:00Z".into()),
                ..Post::default()
            }],
            ..Creator::default()
        };
        rebuild_pictures(&mut c);
        let files: Vec<&str> = c.pictures.iter().map(|p| p.file.as_str()).collect();
        assert_eq!(files.len(), 1 + MAX_OTHER_PICTURES);
        assert_eq!(
            &files[..4],
            ["ig-1.jpg", "web-a.jpg", "es-5-0.jpg", "web-b.jpg"]
        );
        assert_eq!(files[4], "web-8.jpg");
        assert_eq!(
            web_picture("https://x/a.jpg", "png").len(),
            "web-".len() + 16 + 4
        );
        assert!(safe_name(&web_picture("https://x/a.jpg", "jpg")));
    }
}
