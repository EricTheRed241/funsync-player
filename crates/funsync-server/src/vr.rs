//! The VR decision (`backend/routes/vr_api.py`, `detect_vr_format_for_video`):
//! the VR Format panel's projection first, then Mark as VR / Flat, then the
//! filename (`detect_vr_format`), then the frames (`crate::stereo`: two eyes
//! side by side or one picture), then the frame shape (cached dimensions).

use std::sync::LazyLock;

use regex::Regex;

use crate::data::Library;
use crate::scan::Video;

/// `(screenType, stereoMode, is3d)` as DeoVR names them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Format {
    pub screen: &'static str,
    pub stereo: &'static str,
    pub is_3d: bool,
}

const fn f(screen: &'static str, stereo: &'static str) -> Format {
    Format {
        screen,
        stereo,
        is_3d: !matches!(stereo.as_bytes(), b"off"),
    }
}

pub const FLAT: Format = f("flat", "off");
/// Layout for a video known to be VR whose filename doesn't say which.
const DEFAULT_VR: Format = f("mkx200", "sbs");

/// Any common filename separator. Python's lookahead `(?=SEP|$)` becomes a
/// consuming group: only whether a pattern matches counts.
const SEP: &str = r"[_\-. /\\\[\]()]";

fn re(p: &str) -> Regex {
    Regex::new(&format!("(?i){}", p.replace("SEP", SEP))).unwrap()
}

static VR_PATTERNS: LazyLock<Vec<(Regex, Format)>> = LazyLock::new(|| {
    [
        (
            r"(?:^|SEP)360(?:x180)?SEP(?:3dh|sbs|LR)(?:SEP|$)",
            f("sphere", "sbs"),
        ),
        (
            r"(?:^|SEP)360(?:x180)?SEP(?:3dv|TB)(?:SEP|$)",
            f("sphere", "tb"),
        ),
        (
            r"(?:^|SEP)(?:mono360|360SEPmono)(?:SEP|$)",
            f("sphere", "off"),
        ),
        (
            r"(?:^|SEP)(?:180(?:x180)?SEP(?:3dv|TB)|TBSEP180|3dv)(?:SEP|$)",
            f("dome", "tb"),
        ),
        (
            r"(?:^|SEP)(?:180(?:x180)?(?:SEP3dh)?(?:SEPLR)?|LRSEP180|3dh)(?:SEP|$)",
            f("dome", "sbs"),
        ),
        (
            r"(?:^|SEP)(?:mono180|180SEPmono)(?:SEP|$)",
            f("dome", "off"),
        ),
        (r"(?:^|SEP)MKX200(?:SEP|$)", f("mkx200", "sbs")),
        (r"(?:^|SEP)MKX220(?:SEP|$)", f("mkx220", "sbs")),
        (r"(?:^|SEP)RF52(?:SEP|$)", f("rf52", "sbs")),
        (r"(?:^|SEP)FISHEYE190(?:SEP|$)", f("fisheye", "sbs")),
        (r"(?:^|SEP)VRCA220(?:SEP|$)", f("fisheye", "sbs")),
        (r"(?:^|SEP)FB360(?:SEP|$)", f("sphere", "sbs")),
        (r"(?:^|SEP)EAC360(?:SEP|$)", f("sphere", "sbs")),
        (r"(?:^|SEP)sbs(?:SEP|$)", f("dome", "sbs")),
        (r"(?:^|SEP)(?:tb|ou)(?:SEP|$)", f("dome", "tb")),
        (r"(?:^|SEP)lr(?:SEP|$)", f("dome", "sbs")),
        (r"(?:^|SEP)3dh(?:SEP|$)", f("dome", "sbs")),
        (r"(?:^|SEP)3dv(?:SEP|$)", f("dome", "tb")),
    ]
    .into_iter()
    .map(|(p, fmt)| (re(p), fmt))
    .collect()
});

static VR_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?:^|SEP)(?:VR\d{2,4}|\d{2,4}VR|VR\d+K|\d+KVR)(?:SEP|$)"));
static VR_STUDIO_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?:^|SEP)(?:[A-Z]{1,8}VR[A-Z]{0,3}|VR[A-Z]{1,4})SEP*\d{2,5}"));
static BARE_VR_RE: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^|SEP)VR(?:SEP|$)"));
static SEP_SPLIT_RE: LazyLock<Regex> = LazyLock::new(|| re("SEP+"));
static STUDIO_COMBO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-Z]{2,8})\d{2,5}$").unwrap());
static EXT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.[^./\\]+$").unwrap());

/// Japanese and Western VR studio codes with their usual layout
/// (`VR_STUDIO_FORMATS`).
const FISHEYE_STUDIOS: &[&str] = &[
    "SIVR",
    "KAVR",
    "SAVR",
    "DSVR",
    "PRVR",
    "IPVR",
    "MDVR",
    "WAVR",
    "NHVR",
    "EBVR",
    "HNVR",
    "MTVR",
    "ATVR",
    "EXVR",
    "WPVR",
    "PXVR",
    "TMAVR",
    "FSVR",
    "UNVR",
    "DOVR",
    "JUVR",
    "MXVR",
    "KMVR",
    "VRKM",
    "BIKMVR",
    "CBIKMVR",
    "KIWVR",
    "VRSP",
    "URVRSP",
    "AVOPVR",
    "GOPJ",
    "VRBANGERS",
    "VRB",
    "VRCONK",
];
const DOME_STUDIOS: &[&str] = &[
    "CJVR",
    "WANKZVR",
    "NAVR",
    "NAUGHTYAMERICAVR",
    "BADOINKVR",
    "BAVR",
    "MILFVR",
    "POVR",
    "SINSVR",
    "REALJAMVR",
    "RJVR",
    "CZECHVR",
    "CZECHVRCASTING",
    "CZECHVRFETISH",
    "CZECHVRNETWORK",
    "TMWVRNET",
    "VIRTUALREALPORN",
    "VRP",
    "LETHALHARDCOREVR",
    "LHVR",
    "SLRORIGINALS",
    "DARKROOMVR",
    "SWEETLIFEVR",
    "HOLOGIRLSVR",
    "STASYQVR",
    "VRALLURE",
    "VRHUSH",
    "GROOBYVR",
    "GROVR",
    "KINKVR",
    "18VR",
    "EVILANGELVR",
    "EAVR",
    "METAVERSEVR",
    "ZEXYVR",
    "REALHOTVR",
    "VRLATINA",
    "PORNHATVR",
    "COSPLAYBABESVR",
    "VRTRANSTASTY",
    "VRBTS",
    "VRBTNS",
    "VRBS",
    "VRBANS",
];

fn studio(code: &str) -> Option<Format> {
    if FISHEYE_STUDIOS.contains(&code) {
        Some(f("mkx200", "sbs"))
    } else if DOME_STUDIOS.contains(&code) {
        Some(f("dome", "sbs"))
    } else {
        None
    }
}

/// VR layout from a filename (`detect_vr_format`): explicit projection /
/// stereo tags, a known studio code, a `XXVR-123` code, a `VR180`-style
/// tag, then a bare `VR` token.
pub fn detect_vr_format(name: &str) -> Format {
    let stem = EXT_RE.replace(name, "");
    if let Some((_, fmt)) = VR_PATTERNS.iter().find(|(r, _)| r.is_match(&stem)) {
        return *fmt;
    }
    for tok in SEP_SPLIT_RE.split(&stem).filter(|t| !t.is_empty()) {
        let upper = tok.to_uppercase();
        if let Some(fmt) = studio(&upper) {
            return fmt;
        }
        if let Some(fmt) = STUDIO_COMBO_RE.captures(&upper).and_then(|c| studio(&c[1])) {
            return fmt;
        }
    }
    if VR_STUDIO_RE.is_match(&stem) || VR_TAG_RE.is_match(&stem) || BARE_VR_RE.is_match(&stem) {
        return DEFAULT_VR;
    }
    FLAT
}

/// A ~2:1 frame at least 3200 wide: a stereo VR master (`_dims_suggest_vr`).
pub fn dims_suggest_vr(w: u32, h: u32) -> bool {
    w >= 3200 && h > 0 && (1.9..=2.1).contains(&(w as f64 / h as f64))
}

/// VR Format panel projections that name a playable geometry.
fn projection_format(p: &str) -> Option<Format> {
    Some(match p {
        "sbs-full" => f("flat", "sbs"),
        "tb-half" | "tb-full" => f("dome", "tb"),
        "equirect-180" => f("dome", "sbs"),
        "equirect-360" => f("sphere", "sbs"),
        "fisheye-180" | "fisheye-190" => f("fisheye", "sbs"),
        "fisheye-200" | "mkx200" => f("mkx200", "sbs"),
        "rf52" => f("rf52", "sbs"),
        _ => return None,
    })
}

/// The full decision for one video; `dims` are cached `(width, height)`.
pub fn detect(lib: &Library, v: &Video, dims: Option<(u32, u32)>) -> Format {
    let l = lib.lib();
    let projection = l["vrFormat"][&v.path]["projection"].as_str();
    if projection == Some("flat") {
        return FLAT;
    }
    if let Some(fmt) = projection.and_then(projection_format) {
        return fmt;
    }
    let manual = if projection.is_some() {
        None
    } else {
        l["manualVRType"][&v.path].as_str()
    };
    if manual == Some("flat") {
        return FLAT;
    }
    let by_name = detect_vr_format(&v.name);
    if by_name.is_3d {
        return by_name;
    }
    if projection.is_some() || manual == Some("vr") {
        return DEFAULT_VR;
    }
    if manual.is_none() {
        match crate::stereo::layout(&v.path) {
            // Each eye wider than 1.5:1 is flat 3D, else a 180° render.
            Some("sbs") if dims.is_some_and(|(w, h)| h > 0 && w as f64 / 2.0 / h as f64 > 1.5) => {
                return f("flat", "sbs");
            }
            Some("sbs") => return f("dome", "sbs"),
            Some(_) => return by_name,
            None => {}
        }
        if dims.is_some_and(|(w, h)| dims_suggest_vr(w, h)) {
            return DEFAULT_VR;
        }
    }
    by_name
}

/// Neither the user (VR Format, Mark as VR / Flat) nor the file name decides:
/// the frames are looked at (`queue_dims_warmup`).
pub fn needs_frames(lib: &Library, v: &Video) -> bool {
    let l = lib.lib();
    l["vrFormat"][&v.path]["projection"].is_null()
        && l["manualVRType"][&v.path].is_null()
        && !detect_vr_format(&v.name).is_3d
}

pub fn is_vr(lib: &Library, v: &Video) -> bool {
    detect(lib, v, None).is_3d
}

/// The thumbnail crop (`thumb_vr_crop`): one eye of a stereo frame, and its
/// middle for anything but a flat screen.
pub fn thumb_crop(fmt: Format) -> Option<&'static str> {
    if !fmt.is_3d {
        return None;
    }
    Some(match (fmt.stereo, fmt.screen) {
        ("sbs", "flat") => "sbs",
        ("sbs", _) => "sbsc",
        ("tb", "flat") => "tb",
        ("tb", _) => "tbc",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Names and expectations from `backend/tests/test_vr_api.py`.
    #[test]
    fn filenames() {
        let cases: &[(&str, &str, &str, bool)] = &[
            ("Scene_180_sbs.mp4", "dome", "sbs", true),
            ("Video_180x180_3dh_LR.mp4", "dome", "sbs", true),
            ("Scene_360x180_3dh.mp4", "sphere", "sbs", true),
            ("Scene_180_3dv.mp4", "dome", "tb", true),
            ("Scene_mono360.mp4", "sphere", "off", false),
            ("Scene_MKX200.mp4", "mkx200", "sbs", true),
            ("Scene_MKX220.mp4", "mkx220", "sbs", true),
            ("Scene_RF52.mp4", "rf52", "sbs", true),
            ("Scene_FISHEYE190.mp4", "fisheye", "sbs", true),
            ("Regular Video.mp4", "flat", "off", false),
            ("Scene_sbs.mp4", "dome", "sbs", true),
            ("scene_180_SBS.mp4", "dome", "sbs", true),
            ("", "flat", "off", false),
            ("SIVR-178-A.mp4", "mkx200", "sbs", true),
            ("KAVR-483-A.mp4", "mkx200", "sbs", true),
            ("SAVR-278c-4k60fps_P2_.mp4", "mkx200", "sbs", true),
            ("CJVR-043-B-Decensored.mp4", "dome", "sbs", true),
            ("IPVR-215.mp4", "mkx200", "sbs", true),
            ("VRKM-912.mp4", "mkx200", "sbs", true),
            ("KIWVR-382.mp4", "mkx200", "sbs", true),
            ("D:\\VR\\SIVR-178-A.mp4", "mkx200", "sbs", true),
            ("KAVR-428-2-Mayuki Ito_8K_180_3DH.mp4", "dome", "sbs", true),
            ("ZZVR-999.mp4", "mkx200", "sbs", true),
            ("ABP-123.mp4", "flat", "off", false),
            ("Overview-Report.mp4", "flat", "off", false),
            ("Movie.Title.SBS.mp4", "dome", "sbs", true),
            ("Scene-MKX200-7K.mp4", "mkx200", "sbs", true),
            ("Movie Title SBS 180.mp4", "dome", "sbs", true),
            ("[VR] My Movie.mp4", "mkx200", "sbs", true),
            ("My Movie (VR).mp4", "mkx200", "sbs", true),
            ("Studio.Title.4K.VR.180.SBS.mkv", "dome", "sbs", true),
            ("VR180-title.mp4", "mkx200", "sbs", true),
            ("8KVR_scene.mp4", "mkx200", "sbs", true),
            ("SIVR178.mp4", "mkx200", "sbs", true),
            ("SIVR.178.Title.mkv", "mkx200", "sbs", true),
            ("Movie_FB360_8K.mp4", "sphere", "sbs", true),
            ("C:\\Downloads\\VR\\movie.mp4", "mkx200", "sbs", true),
            ("/home/me/Videos/VR/title.mp4", "mkx200", "sbs", true),
            ("hhb3d-sivr-178.mp4", "mkx200", "sbs", true),
            ("Movie.2023.8K.HDR.mp4", "flat", "off", false),
            ("server-backup.mp4", "flat", "off", false),
            ("carving_wood.mp4", "flat", "off", false),
            (
                "Documentary.2023.1080p.BluRay.x264.mkv",
                "flat",
                "off",
                false,
            ),
            ("WankzVR - Scene Title.mp4", "dome", "sbs", true),
            ("NaughtyAmericaVR.Scene.Title.mp4", "dome", "sbs", true),
            ("BadoinkVR - Scene.mp4", "dome", "sbs", true),
            ("MilfVR scene title.mp4", "dome", "sbs", true),
            ("CzechVR_0123_title.mp4", "dome", "sbs", true),
            (
                "GroobyVR - Daisy Taylor - Roommate Wanted VR.mp4",
                "dome",
                "sbs",
                true,
            ),
            ("2.GroVR_30 35_title_TMAL.mp4", "dome", "sbs", true),
            ("RealJamVR.scene.mp4", "dome", "sbs", true),
            ("SinsVR-title.mp4", "dome", "sbs", true),
            ("VRBangers - Scene Title.mp4", "mkx200", "sbs", true),
            ("VRConk.scene.mp4", "mkx200", "sbs", true),
            (
                "9.VRBTS_46 20_Naie Mars_the_nutcracker_tmal.mp4",
                "dome",
                "sbs",
                true,
            ),
            (
                "8.vrbans_33 19_slty_receptionist_TMAL.mp4",
                "dome",
                "sbs",
                true,
            ),
            (
                "6.VRBS_42 15_Alise_Game Over TraR Pn_tmal.mp4",
                "dome",
                "sbs",
                true,
            ),
            (
                "5.VRBTNS_34 04_AIA RAE_watch_and_learn_TMAL.mp4",
                "dome",
                "sbs",
                true,
            ),
        ];
        for &(name, screen, stereo, is_3d) in cases {
            let got = detect_vr_format(name);
            assert_eq!(
                (got.screen, got.stereo, got.is_3d),
                (screen, stereo, is_3d),
                "{name}"
            );
        }
        assert!(detect_vr_format("VR360_movie.mp4").is_3d);
    }

    fn video(name: &str) -> Video {
        serde_json::from_value(
            json!({"id": "x", "name": name, "path": format!("/v/{name}"),
            "ext": ".mp4", "funscriptPath": null, "fuzzyMatched": false, "subtitlePath": null,
            "variants": [], "axes": [], "sourceName": "", "dateAdded": 0, "size": 0}),
        )
        .unwrap()
    }

    #[test]
    fn dims_and_overrides() {
        assert!(dims_suggest_vr(3840, 1920));
        assert!(dims_suggest_vr(5760, 2880));
        assert!(!dims_suggest_vr(3840, 2160)); // 16:9
        assert!(!dims_suggest_vr(1920, 960)); // 2:1 but low-res
        assert!(!dims_suggest_vr(4096, 0));

        let lib = |cfg| Library {
            config: json!({"settings": {"library": cfg}}),
            ..Default::default()
        };
        let plain = video("Clip.mp4");
        let empty = lib(json!({}));
        assert_eq!(detect(&empty, &plain, Some((3840, 1920))), DEFAULT_VR);
        assert_eq!(detect(&empty, &plain, Some((1920, 1080))), FLAT);
        assert_eq!(detect(&empty, &plain, None), FLAT);
        let manual_flat = lib(json!({"manualVRType": {"/v/Clip.mp4": "flat"}}));
        assert_eq!(detect(&manual_flat, &plain, Some((3840, 1920))), FLAT);
        let manual_vr = lib(json!({"manualVRType": {"/v/Clip.mp4": "vr"}}));
        assert_eq!(detect(&manual_vr, &plain, None), DEFAULT_VR);
        let tb = video("Clip_180_TB.mp4");
        assert_eq!(detect(&manual_vr, &tb, None), f("dome", "tb"));
        let panel = lib(
            json!({"vrFormat": {"/v/Clip_180_TB.mp4": {"projection": "sbs-full"}},
            "manualVRType": {"/v/Clip_180_TB.mp4": "flat"}}),
        );
        assert_eq!(detect(&panel, &tb, None), f("flat", "sbs"));
        assert_eq!(thumb_crop(f("flat", "sbs")), Some("sbs"));
        assert_eq!(thumb_crop(f("dome", "tb")), Some("tbc"));
        assert_eq!(thumb_crop(FLAT), None);
    }

    #[test]
    fn the_frames_decide_what_nothing_else_does() {
        let lib = |cfg| Library {
            config: json!({"settings": {"library": cfg}}),
            ..Default::default()
        };
        let empty = lib(json!({}));
        let render = video("Frames render.mp4");
        crate::stereo::set_layout(&render.path, Some("sbs"));
        assert!(needs_frames(&empty, &render));
        assert_eq!(detect(&empty, &render, None), f("dome", "sbs"));
        assert_eq!(
            detect(&empty, &render, Some((7680, 2160))),
            f("flat", "sbs"),
            "wide eyes: flat 3D"
        );
        let manual = lib(json!({"manualVRType": {"/v/Frames render.mp4": "flat"}}));
        assert_eq!(detect(&manual, &render, None), FLAT);
        assert!(!needs_frames(&manual, &render));
        // One picture beats a VR-shaped frame.
        let collage = video("Frames collage.mp4");
        crate::stereo::set_layout(&collage.path, Some("mono"));
        assert_eq!(detect(&empty, &collage, Some((4320, 2160))), FLAT);
        assert!(!needs_frames(&empty, &video("Frames_180_sbs.mp4")));
    }
}
