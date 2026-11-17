//! What a video is, from its list entry: why it is dropped, whether it is
//! VR (and the file-name suffix the headset reads its layout from), PMV,
//! compilation. Words (`pmvhavenScout` in content-preferences.json) match
//! whole words of the folded title and tags (`slr_scout::text::normalise`),
//! so the tag `Pmv，music` says `pmv` and `Cock hero` says `cock hero`.

use crate::api::Video;
use crate::prefs::PmvhavenScout;
use slr_scout::text;

/// Why a video is dropped before its details are read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drop {
    /// PMVHaven's own content flags (`hasGayContent`, `hasTransContent`,
    /// `hasExtremeContent`).
    Gay,
    Trans,
    Extreme,
    /// An audio piece (an `audioWords` word): voice and sound over a still or
    /// a loop. `hasVoiceOver` alone isn't one: 156 of the 1,258 scripted
    /// videos (2026-11-17) are PMVs with JOI or hypno voice over real video.
    Audio(String),
    /// No video file to download.
    NoVideo,
}

impl Drop {
    pub fn key(&self) -> &'static str {
        match self {
            Drop::Gay => "gay",
            Drop::Trans => "trans",
            Drop::Extreme => "extreme",
            Drop::Audio(_) => "audio",
            Drop::NoVideo => "no-video",
        }
    }
}

/// The title and tags as one folded text, padded for whole-word search.
pub fn words_text(v: &Video) -> String {
    text::normalise(&format!("{} {}", v.title, v.tags.join(" ")))
}

/// The first of `words` in `folded` (whole words).
pub fn find<'a>(folded: &str, words: &'a [String]) -> Option<&'a String> {
    words
        .iter()
        .find(|w| text::contains_words(folded, &text::normalise(w)))
}

pub fn drop_reason(v: &Video, p: &PmvhavenScout) -> Option<Drop> {
    if v.gay {
        return Some(Drop::Gay);
    }
    if v.trans {
        return Some(Drop::Trans);
    }
    if v.extreme {
        return Some(Drop::Extreme);
    }
    if let Some(w) = find(&words_text(v), &p.audio_words) {
        return Some(Drop::Audio(w.clone()));
    }
    if v.video_url.is_none() {
        return Some(Drop::NoVideo);
    }
    None
}

/// The VR layout's file-name suffix (HereSphere / DeoVR tokens, read by the
/// app's `vr-detect.js` too), or `None` for a flat video:
///
/// - a frame 32:9 or wider (aspect ≥ 3.4) is two 16:9 eyes side by side
///   (the "VR 3D SBS" conversions): `_SBS_FULL`, VR with or without a word;
/// - 2:1 (1.9–2.1) is one 180° side-by-side frame, `_180_LR`, and 1:1
///   (0.95–1.05) a top-bottom one, `_180_TB`, but only when a `vrWords` word
///   (VR, SBS, VR180) is in the title or tags: PMVHaven has flat cinematic
///   2:1 edits too (2160×1080 "BBC RAW", 3840×1920 "Deviant Edge");
/// - anything else (16:9, vertical) is flat, even with a VR tag.
pub fn vr_suffix(v: &Video, p: &PmvhavenScout) -> Option<&'static str> {
    let aspect = v.aspect?;
    if aspect >= 3.4 {
        return Some("_SBS_FULL");
    }
    let said = find(&words_text(v), &p.vr_words).is_some();
    if said && (1.9..=2.1).contains(&aspect) {
        return Some("_180_LR");
    }
    if said && (0.95..=1.05).contains(&aspect) {
        return Some("_180_TB");
    }
    None
}

/// A PMV, HMV, Cock Hero or Fap Hero by its title or tags.
pub fn is_pmv(v: &Video, p: &PmvhavenScout) -> bool {
    find(&words_text(v), &p.pmv_words).is_some()
}

/// A compilation by its title.
pub fn is_compilation(v: &Video, p: &PmvhavenScout) -> bool {
    find(&text::normalise(&v.title), &p.compilation_words).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs() -> PmvhavenScout {
        crate::prefs::load().unwrap()
    }

    fn video(title: &str, tags: &[&str], aspect: f64) -> Video {
        Video {
            id: "a".into(),
            title: title.into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            aspect: Some(aspect),
            video_url: Some("https://s3/v.mp4".into()),
            ..Video::default()
        }
    }

    #[test]
    fn drops_flagged_and_audio_videos() {
        let p = prefs();
        let ok = video("Neurotica", &["Blowjob"], 1.7778);
        assert_eq!(drop_reason(&ok, &p), None);
        let mut v = ok.clone();
        v.gay = true;
        v.trans = true;
        assert_eq!(drop_reason(&v, &p), Some(Drop::Gay));
        v.gay = false;
        assert_eq!(drop_reason(&v, &p).unwrap().key(), "trans");
        let mut v = ok.clone();
        v.extreme = true;
        assert_eq!(drop_reason(&v, &p), Some(Drop::Extreme));
        let v = video(
            "Corrupted by your Girlfriend's Sister - Binaural Story Audio",
            &["Binaural", "Gonewildaudio"],
            1.7778,
        );
        assert_eq!(drop_reason(&v, &p), Some(Drop::Audio("binaural".into())));
        // Voice over real video stays.
        let mut v = video("Hypno JOI PMV", &["Hypno", "Joi", "Voice-over"], 1.7778);
        v.voice_over = true;
        assert_eq!(drop_reason(&v, &p), None);
        let mut v = ok;
        v.video_url = None;
        assert_eq!(drop_reason(&v, &p), Some(Drop::NoVideo));
    }

    #[test]
    fn vr_by_frame_and_words() {
        let p = prefs();
        let s =
            |title: &str, tags: &[&str], aspect: f64| vr_suffix(&video(title, tags, aspect), &p);
        assert_eq!(
            s("Ecstasy - VR 3D SBS Conversion", &["Vr"], 3.5556),
            Some("_SBS_FULL")
        );
        assert_eq!(s("i fckn love gym girls 2", &[], 3.5556), Some("_SBS_FULL"));
        assert_eq!(
            s("[VR PMV] World PMV Games 2025", &[], 2.0),
            Some("_180_LR")
        );
        assert_eq!(
            s("Try not to cum", &["Vr", "Blowjob"], 2.0),
            Some("_180_LR")
        );
        assert_eq!(s("Dome", &["VR180"], 1.0), Some("_180_TB"));
        // 2:1 without a word: a cinematic flat edit.
        assert_eq!(s("BBC RAW", &["Bbc"], 2.0), None);
        // A VR tag on a 16:9 frame: flat.
        assert_eq!(s("Noodledude VR", &["Vr"], 1.7778), None);
        assert_eq!(s("Vertical", &[], 0.5625), None);
        // "Vroom" isn't VR.
        assert_eq!(s("Vroom", &[], 2.0), None);
        let mut v = video("x", &["Vr"], 2.0);
        v.aspect = None;
        assert_eq!(vr_suffix(&v, &p), None);
    }

    #[test]
    fn pmv_and_compilation() {
        let p = prefs();
        assert!(is_pmv(&video("Neurotica", &["Pmv，music"], 1.7), &p));
        assert!(is_pmv(&video("Neurotica", &["Cock hero"], 1.7), &p));
        assert!(is_pmv(&video("Overwatch HMV", &[], 1.7), &p));
        assert!(!is_pmv(&video("Neurotica", &["Goon"], 1.7), &p));
        assert!(!is_pmv(&video("PMVHaven special", &[], 1.7), &p));
        assert!(is_compilation(&video("Riding compilation", &[], 1.7), &p));
        assert!(is_compilation(&video("Best of 2025", &[], 1.7), &p));
        assert!(!is_compilation(&video("x", &["Compilation"], 1.7), &p));
    }
}
