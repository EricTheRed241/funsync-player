//! Which script a download takes. PMVHaven has two kinds: the uploader's
//! own (`funScriptUrl`, made with the edit) and community scripts
//! (`userFunScripts[]`, anyone's, with an optional description).
//!
//! The uploader's script comes first. Without one, the best community
//! script: by tier (hand-made, then generated: a `generatedScriptWords`
//! word such as "Quick Audio Based FunScript" in its description, then one
//! for another device: an `otherDeviceScriptWords` word such as "FM Hero"
//! or "oscillation"), then most likes, then the newest. Axis companions
//! (`<name>.twist.funscript` …, by `slr_scout::forum::script_axis`) aren't
//! picked as the stroke script; the same user's companions of the picked
//! script come along, so `axes` lists hand-made axes known from file names
//! alone (no script is read during a scan).

use crate::api::{CommunityScript, Detail};
use crate::prefs::PmvhavenScout;
use serde::{Deserialize, Serialize};
use slr_scout::forum::script_axis;
use slr_scout::text;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Companion {
    pub axis: String,
    pub url: String,
}

/// The picked script (the scene file's `script`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Script {
    pub url: String,
    /// Axes besides stroke from companion files (`[]`: single-axis).
    pub axes: Vec<String>,
    /// `uploader` or `community`.
    pub source: String,
    /// Who made it (the uploader or the community user).
    pub by: String,
    /// Described as generated (audio / beat based).
    #[serde(default)]
    pub generated: bool,
    #[serde(default)]
    pub companions: Vec<Companion>,
}

/// A script URL's file name without PMVHaven's upload prefix
/// (`<user id>_<epoch ms>_`), percent-escapes left as they are.
pub fn file_name(url: &str) -> &str {
    let name = url.rsplit('/').next().unwrap_or(url);
    let name = name.split(['?', '#']).next().unwrap_or(name);
    let mut parts = name.splitn(3, '_');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(user), Some(ms), Some(rest))
            if user.len() == 24
                && user.bytes().all(|b| b.is_ascii_hexdigit())
                && !ms.is_empty()
                && ms.bytes().all(|b| b.is_ascii_digit()) =>
        {
            rest
        }
        _ => name,
    }
}

/// 0 hand-made, 1 generated, 2 for another device.
fn tier(s: &CommunityScript, p: &PmvhavenScout) -> u8 {
    let d = text::normalise(&s.description);
    let has = |words: &[String]| {
        words
            .iter()
            .any(|w| text::contains_words(&d, &text::normalise(w)))
    };
    if has(&p.other_device_script_words) {
        2
    } else if has(&p.generated_script_words) {
        1
    } else {
        0
    }
}

pub fn choose(detail: &Detail, uploader: &str, p: &PmvhavenScout) -> Option<Script> {
    if let Some(url) = &detail.uploader_script {
        return Some(Script {
            url: url.clone(),
            axes: Vec::new(),
            source: "uploader".into(),
            by: uploader.to_string(),
            generated: false,
            companions: Vec::new(),
        });
    }
    let strokes = detail
        .community
        .iter()
        .filter(|s| matches!(script_axis(file_name(&s.url)), Some((_, None))));
    let best = strokes.min_by(|a, b| {
        tier(a, p)
            .cmp(&tier(b, p))
            .then(b.likes.cmp(&a.likes))
            .then(b.uploaded.cmp(&a.uploaded))
    })?;
    let (base, _) = script_axis(file_name(&best.url))?;
    let mut companions: Vec<Companion> = Vec::new();
    for s in detail
        .community
        .iter()
        .filter(|s| s.username == best.username)
    {
        if let Some((b, Some(axis))) = script_axis(file_name(&s.url))
            && b.eq_ignore_ascii_case(&base)
            && !companions.iter().any(|c| c.axis == axis)
        {
            companions.push(Companion {
                axis: axis.to_string(),
                url: s.url.clone(),
            });
        }
    }
    Some(Script {
        url: best.url.clone(),
        axes: companions.iter().map(|c| c.axis.clone()).collect(),
        source: "community".into(),
        by: best.username.clone(),
        generated: tier(best, p) == 1,
        companions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const U: &str = "68fdc784b99aaf24a4c0d42a";

    fn community(
        user: &str,
        name: &str,
        description: &str,
        likes: u64,
        day: u8,
    ) -> CommunityScript {
        CommunityScript {
            username: user.into(),
            url: format!("https://s3/funscripts/vid/{U}_17853383055{day:02}_{name}"),
            description: description.into(),
            likes,
            uploaded: Some(format!("2026-08-{day:02}T00:00:00Z")),
        }
    }

    #[test]
    fn strips_the_upload_prefix() {
        assert_eq!(
            file_name(&format!(
                "https://s3/f/v/{U}_1785338305589_RedDaddi_-_STUPID.funscript"
            )),
            "RedDaddi_-_STUPID.funscript"
        );
        assert_eq!(
            file_name("https://s3/funscripts/1794908845009_ncmhfiki07q_GRANDANA2.funscript"),
            "1794908845009_ncmhfiki07q_GRANDANA2.funscript"
        );
    }

    #[test]
    fn the_uploaders_script_comes_first() {
        let p = crate::prefs::load().unwrap();
        let d = Detail {
            uploader_script: Some("https://s3/own.funscript".into()),
            community: vec![community("a", "x.funscript", "", 9, 1)],
            ..Detail::default()
        };
        let s = choose(&d, "Cowgirlfan101", &p).unwrap();
        assert_eq!(
            (s.source.as_str(), s.by.as_str()),
            ("uploader", "Cowgirlfan101")
        );
        assert_eq!(s.url, "https://s3/own.funscript");
        assert!(s.axes.is_empty());
        assert_eq!(choose(&Detail::default(), "x", &p), None);
    }

    #[test]
    fn picks_the_best_community_script() {
        let p = crate::prefs::load().unwrap();
        let d = Detail {
            community: vec![
                community("auto", "A.funscript", "Quick Audio Based FunScript", 50, 1),
                community(
                    "fm",
                    "B.funscript",
                    "_Fuck-Machine Hero (FM Hero)_ A re-scripting",
                    90,
                    2,
                ),
                community("hand", "C.funscript", "Made for Handy", 1, 3),
                community("hand2", "D.funscript", "", 1, 4),
                community("hand2", "D.twist.funscript", "", 99, 4),
                community("other", "D.roll.funscript", "", 0, 4),
            ],
            ..Detail::default()
        };
        // Hand-made first; equal likes: the newest, with its own companions.
        let s = choose(&d, "up", &p).unwrap();
        assert_eq!((s.by.as_str(), s.source.as_str()), ("hand2", "community"));
        assert!(s.url.ends_with("_D.funscript") && !s.generated);
        assert_eq!(s.axes, ["twist"]);
        assert!(s.companions[0].url.ends_with("_D.twist.funscript"));
        // Only generated and other-device ones: generated wins over likes.
        let d = Detail {
            community: d.community[..2].to_vec(),
            ..Detail::default()
        };
        let s = choose(&d, "up", &p).unwrap();
        assert_eq!(s.by, "auto");
        assert!(s.generated);
        // Companions alone: no stroke script.
        let d = Detail {
            community: vec![community("a", "x.twist.funscript", "", 0, 1)],
            ..Detail::default()
        };
        assert_eq!(choose(&d, "up", &p), None);
    }
}
