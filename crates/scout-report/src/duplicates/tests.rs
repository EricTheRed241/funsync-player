use super::*;

fn t() -> Thresholds {
    Thresholds {
        min_confidence: 70.0,
        floor_confidence: 40.0,
        target: 3,
        compilation_words: vec!["pmv".into(), "compilation".into()],
    }
}

fn w(words: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = words.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

#[test]
fn title_words_drop_encode_projection_and_artist() {
    let (words, copy) = title_words("Drills3D - Judy Alvarez 4K60fps VR 180 SBS (2)", "Drills3D");
    assert_eq!(words, w(&["alvarez", "judy"]));
    assert_eq!(copy, Some(2));
    // Fap hero is one (genre) word; a decomposed é equals a composed one.
    let (words, _) = title_words("Ne\u{301}mesis Fap Hero 1080p", "");
    assert_eq!(words, w(&["faphero", "nemesis"]));
    assert!(!is_distinctive("faphero"));
    assert!(is_distinctive("触手"));
    assert!(!is_distinctive("2b"));
}

#[test]
fn same_title_and_length_is_a_likely_duplicate() {
    let (title, shared) = title_similarity(&w(&["judy", "alvarez"]), &w(&["judy", "alvarez"]));
    assert_eq!(title, 1.0);
    let v = judge(&Evidence {
        title,
        shared,
        lengths: Some((0.4, 600.0)),
        ..Evidence::default()
    });
    assert_eq!(v.kind, Kind::Duplicate);
    assert_eq!(v.confidence, 90.0);
    assert!(v.reasons.contains(&"same title".to_string()));
}

#[test]
fn identical_script_or_matching_frames_beat_other_titles() {
    let base = Evidence {
        lengths: Some((0.2, 300.0)),
        ..Evidence::default()
    };
    assert_eq!(judge(&base).confidence, 35.0);
    let script = judge(&Evidence {
        same_script: true,
        ..base.clone()
    });
    assert!(script.confidence >= 85.0, "{script:?}");
    let frames = judge(&Evidence {
        frames: Some((5, 5)),
        ..base.clone()
    });
    assert_eq!(frames.confidence, 100.0);
}

#[test]
fn frames_that_differ_and_part_numbers_cut_confidence() {
    let same = Evidence {
        title: 1.0,
        lengths: Some((0.5, 600.0)),
        ..Evidence::default()
    };
    let differ = judge(&Evidence {
        frames: Some((0, 5)),
        ..same.clone()
    });
    assert!(differ.confidence < 60.0, "{differ:?}");
    let parts = judge(&Evidence {
        series: true,
        ..same.clone()
    });
    assert!(parts.confidence < 60.0, "{parts:?}");
    assert!(numbered_apart(
        &w(&["part", "1", "x"]),
        &w(&["part", "2", "x"]),
        None,
        None,
        true
    ));
    assert!(numbered_apart(
        &w(&["x"]),
        &w(&["x"]),
        Some(1),
        Some(2),
        true
    ));
    assert!(!numbered_apart(&w(&["x"]), &w(&["x"]), None, Some(2), true));
    // `X` / `X 2`: a re-encode when as long, a sequel when not.
    assert!(!numbered_apart(
        &w(&["x", "y"]),
        &w(&["2", "x", "y"]),
        None,
        None,
        true
    ));
    assert!(numbered_apart(
        &w(&["x", "y"]),
        &w(&["2", "x", "y"]),
        None,
        None,
        false
    ));
}

#[test]
fn one_word_apart_is_capped_unless_frames_match_everywhere() {
    let a = w(&["heads", "roll", "tess", "will"]);
    let b = w(&["haley", "heads", "roll", "will"]);
    assert!(character_swap(&a, &b));
    assert!(!character_swap(&a, &a));
    let (title, shared) = title_similarity(&a, &b);
    let swap = Evidence {
        title,
        shared,
        lengths: Some((0.0, 200.0)),
        same_script: true,
        same_post: true,
        swap: true,
        ..Evidence::default()
    };
    let v = judge(&swap);
    assert_eq!(v.confidence, SWAP_CAP, "{v:?}");
    let confirmed = judge(&Evidence {
        frames: Some((5, 5)),
        ..swap
    });
    assert_eq!(confirmed.confidence, 100.0);
}

#[test]
fn compilations_of_other_length_are_capped_vr_next_to_flat_too() {
    let comp = judge(&Evidence {
        title: 1.0,
        lengths: Some((400.0, 900.0)),
        compilation: true,
        same_artist: true,
        ..Evidence::default()
    });
    assert_eq!(comp.kind, Kind::Compilation);
    assert!(comp.confidence <= COMPILATION_CAP);
    // Two copies of one PMV (same length) are a plain duplicate.
    let pmv = judge(&Evidence {
        title: 1.0,
        lengths: Some((0.1, 900.0)),
        compilation: true,
        ..Evidence::default()
    });
    assert_eq!(pmv.kind, Kind::Duplicate);
    let versions = judge(&Evidence {
        title: 1.0,
        lengths: Some((0.1, 900.0)),
        same_post: true,
        vr_differs: true,
        ..Evidence::default()
    });
    assert_eq!(versions.kind, Kind::Versions);
    assert_eq!(versions.confidence, VERSIONS_CAP);
}

#[test]
fn bar_lowers_to_reach_the_target_but_not_under_the_floor() {
    let t = t();
    assert_eq!(bar(&[95.0, 90.0, 80.0, 50.0], &t), 70.0);
    assert_eq!(bar(&[95.0, 60.0, 50.0, 45.0], &t), 50.0);
    assert_eq!(bar(&[95.0, 30.0], &t), 40.0);
    assert_eq!(bar(&[], &t), 40.0);
}

#[test]
fn frames_compare_only_samples_with_pictures() {
    let burst = |h: &str, d: u32| -> Burst { vec![None, Some(format!("{h}:{d}")), None] };
    let (ones, zeros) = ("f".repeat(36), "0".repeat(36));
    let near = format!("{}1", "f".repeat(35));
    let a: HashMap<String, Burst> = [
        ("F16.00".to_string(), burst(&ones, 200)),
        ("F32.00".to_string(), burst(&ones, 200)),
        ("F48.00".to_string(), burst(&zeros, 5)),
    ]
    .into();
    let b: HashMap<String, Burst> = [
        ("F16.00".to_string(), burst(&near, 200)),
        ("F32.00".to_string(), burst(&zeros, 200)),
        ("F48.00".to_string(), burst(&zeros, 5)),
        ("F64.00".to_string(), burst(&ones, 200)),
    ]
    .into();
    assert_eq!(compare_frames(&a, &b, false), Some((1, 2)));
    // One word apart: a match needs the fine grids too, which these frames lack.
    assert_eq!(compare_frames(&a, &b, true), Some((0, 2)));
    let grid = |cells: &str| -> Burst { vec![None, Some(format!("{ones}:200:{cells}")), None] };
    let x: HashMap<String, Burst> = [("F1".to_string(), grid("AAAA"))].into();
    let y: HashMap<String, Burst> = [("F1".to_string(), grid("gICA"))].into();
    // Brightness evened out: an all-0 and an all-128 grid are the same picture.
    assert_eq!(compare_frames(&x, &y, true), Some((1, 1)));
    assert_eq!(base64("gICA"), Some(vec![128, 128, 128]));
    assert_eq!(hamming("ff00", "00ff"), Some(16));
}

#[test]
fn pair_ids_put_the_smaller_id_first() {
    assert_eq!(pair_id("b2", "a1"), "a1-b2");
    assert_eq!(parse_pair("a1-b2"), Some(("a1", "b2")));
    assert_eq!(parse_pair("b2-a1"), None);
    assert_eq!(parse_pair("a1-a1"), None);
    assert_eq!(parse_pair("a1-b-2"), None);
}

// ── build / plan with a fake disk ────────────────────────────────────────

#[derive(Default)]
struct Fake {
    sizes: HashMap<String, u64>,
    posts: HashMap<String, Post>,
    scripts: HashMap<String, Vec<(i64, i64)>>,
}

impl Disk for Fake {
    fn size(&self, path: &str) -> Option<u64> {
        self.sizes.get(path).copied()
    }
    fn post(&self, path: &str) -> Option<Post> {
        self.posts.get(path).cloned()
    }
    fn bursts(&self, _: &str, _: u64) -> Option<&HashMap<String, Burst>> {
        None
    }
    fn actions(&self, id: &str) -> Option<Vec<(i64, i64)>> {
        self.scripts.get(id).cloned()
    }
    fn quality(&self, _: &str) -> Option<Quality> {
        Some(Quality {
            width: 1920,
            height: 1080,
            fps: Some(60.0),
        })
    }
    fn posted(&self, post: &Post) -> Option<(String, &'static str)> {
        (post.kind == "eroscripts").then(|| ("2025-01-02T00:00:00.000Z".to_string(), "posted"))
    }
}

fn video(id: &str, path: &str, duration: f64, speeds: (f64, f64)) -> Value {
    json!({"id": id, "path": path, "name": Path::new(path).file_stem().unwrap().to_str().unwrap(),
           "duration": duration, "avgSpeed": speeds.0, "maxSpeed": speeds.1, "isVR": false,
           "score": 5.0, "rating": 0, "favorite": false, "thumbUrl": format!("/api/media/thumb/{id}"),
           "dateAdded": 1_700_000_000_000u64, "tags": []})
}

fn setup() -> (Library, Fake, Vec<PathBuf>) {
    let videos = vec![
        // A 4K and a 1080p encode of one scene.
        video(
            "a1",
            "/lib/EroScripts/Art/Judy/Art - Judy Alvarez 4K.mp4",
            300.0,
            (200.0, 400.0),
        ),
        video(
            "b2",
            "/lib/FapTap/Art/Judy/Art - Judy Alvarez 1080p.mp4",
            300.4,
            (200.0, 400.0),
        ),
        // A re-upload under another name with the identical script.
        video(
            "c3",
            "/lib/Other/X/Y/Something Else.mp4",
            300.9,
            (200.0, 400.0),
        ),
        // Unrelated.
        video(
            "d4",
            "/lib/Other/Z/W/Unrelated Thing.mp4",
            50.0,
            (90.0, 100.0),
        ),
        // Not on disk: never paired.
        video(
            "e5",
            "/lib/Other/Art/Judy/Judy Alvarez.mp4",
            300.0,
            (200.0, 400.0),
        ),
    ];
    let lib = Library {
        axes: videos
            .iter()
            .map(|v| {
                (
                    v["path"].as_str().unwrap().to_string(),
                    "single".to_string(),
                )
            })
            .collect(),
        videos,
        ..Library::default()
    };
    let mut fake = Fake::default();
    for (i, v) in lib.videos.iter().take(4).enumerate() {
        fake.sizes.insert(
            v["path"].as_str().unwrap().into(),
            (i as u64 + 1) * 1_000_000_000,
        );
    }
    fake.posts.insert(
        "/lib/EroScripts/Art/Judy/Art - Judy Alvarez 4K.mp4".into(),
        Post {
            kind: "eroscripts",
            id: "7".into(),
            url: Some("https://discuss.eroscripts.com/t/7".into()),
        },
    );
    let script = vec![(0, 0), (500, 100)];
    for id in ["a1", "b2", "c3"] {
        fake.scripts.insert(id.into(), script.clone());
    }
    let sources = ["EroScripts", "FapTap", "Other"]
        .iter()
        .map(|s| PathBuf::from(format!("/lib/{s}")))
        .collect();
    (lib, fake, sources)
}

#[test]
fn build_deals_likeliest_pairs_with_both_sides() {
    let (lib, fake, sources) = setup();
    let deck = build(
        &lib,
        &Marks::default(),
        &t(),
        &sources,
        &fake,
        10,
        &HashSet::new(),
    );
    let ids: Vec<&str> = deck.cards.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["a1-b2", "b2-c3", "a1-c3"], "{:#?}", deck.cards);
    let top = &deck.cards[0];
    assert!(top.confidence >= 90.0, "{top:?}");
    assert_eq!(top.left.id, "a1");
    assert_eq!(top.left.posted.as_deref(), Some("2025-01-02T00:00:00.000Z"));
    assert_eq!(top.left.posted_from, "posted");
    assert_eq!(top.right.posted_from, "added");
    assert_eq!(top.left.library.as_deref(), Some("EroScripts"));
    assert_eq!(top.left.author.as_deref(), Some("Art"));
    assert_eq!(top.right.size, 2_000_000_000);
    assert!(
        deck.cards[1]
            .reasons
            .contains(&"identical script".to_string())
    );
    assert_eq!(deck.remaining, 3);
}

#[test]
fn swiped_pairs_and_doomed_videos_leave_the_deck() {
    let (lib, fake, sources) = setup();
    let mut marks = Marks::default();
    let at = "2026-11-19T00:00:00.000Z".to_string();
    marks.pairs.insert(
        "a1-b2".into(),
        Mark {
            dir: Dir::Left,
            keep: Some("a1".into()),
            delete: Some("b2".into()),
            at: at.clone(),
        },
    );
    let deck = build(&lib, &marks, &t(), &sources, &fake, 10, &HashSet::new());
    let ids: Vec<&str> = deck.cards.iter().map(|c| c.id.as_str()).collect();
    // b2 is going: only a1 / c3 is left.
    assert_eq!(ids, ["a1-c3"]);
    assert_eq!(deck.wanted, 1);
    assert_eq!(deck.marked_bytes, 2_000_000_000);
    marks.pairs.insert(
        "a1-c3".into(),
        Mark {
            dir: Dir::Up,
            keep: None,
            delete: None,
            at,
        },
    );
    let deck = build(&lib, &marks, &t(), &sources, &fake, 10, &HashSet::new());
    assert!(deck.cards.is_empty());
    assert_eq!(deck.distinct, 1);
    // Cards the page holds are left out but still counted.
    let deck = build(
        &lib,
        &Marks::default(),
        &t(),
        &sources,
        &fake,
        10,
        &["dupes:a1-b2".to_string()].into(),
    );
    assert_eq!(deck.cards.len(), 2);
    assert_eq!(deck.remaining, 3);
}

#[test]
fn plan_deletes_each_copy_once_and_never_both_sides() {
    let (lib, fake, sources) = setup();
    let mark = |keep: &str, del: &str| Mark {
        dir: Dir::Left,
        keep: Some(keep.into()),
        delete: Some(del.into()),
        at: "2026-11-19T00:00:00.000Z".into(),
    };
    let mut marks = Marks::default();
    marks.pairs.insert("a1-b2".into(), mark("a1", "b2"));
    marks.pairs.insert("b2-c3".into(), mark("c3", "b2"));
    // Gone from the library: stale.
    marks.pairs.insert("a1-zz".into(), mark("a1", "zz"));
    let p = plan(&lib, &marks, &sources, |path| fake.size(path));
    assert_eq!(
        p.delete,
        [("b2".to_string(), "a1".to_string(), 2_000_000_000)]
    );
    assert_eq!(p.stale, ["a1-zz"]);
    // Crossed choices (keep a over c, keep c over a) delete neither.
    let mut crossed = Marks::default();
    crossed.pairs.insert("a1-c3".into(), mark("a1", "c3"));
    crossed.pairs.insert("x".into(), mark("c3", "a1"));
    let p = plan(&lib, &crossed, &sources, |path| fake.size(path));
    assert!(p.delete.is_empty(), "{p:?}");
    assert_eq!(p.skipped.len(), 2);
}

#[test]
fn marks_round_trip_with_keep_and_delete() {
    let dir = std::env::temp_dir().join(format!("dupes-marks-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let m = Marks::set(&dir, "a1-b2", Dir::Right, "now".into()).unwrap();
    assert_eq!(
        (m.keep.as_deref(), m.delete.as_deref()),
        (Some("b2"), Some("a1"))
    );
    Marks::set(&dir, "a1-c3", Dir::Up, "now".into()).unwrap();
    let read = Marks::read(&dir).unwrap();
    assert_eq!(read.pairs.len(), 2);
    assert_eq!(read.doomed().get("a1"), Some(&"b2"));
    assert!(Marks::undo(&dir, "a1-b2").unwrap());
    assert!(!Marks::undo(&dir, "a1-b2").unwrap());
    assert!(Marks::set(&dir, "b2-a1", Dir::Left, "now".into()).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn frame_paths_name_a_video_and_a_share() {
    assert_eq!(
        frame_request("/dupes-frames/a1b2/40.jpg"),
        Some(("a1b2", 40))
    );
    for bad in [
        "/dupes-frames/a1b2/45.jpg",
        "/dupes-frames/../40.jpg",
        "/dupes-frames/a1b2/040.jpg",
        "/dupes-frames/a1b2/40.png",
        "/dupes-frames/a1b2/x/40.jpg",
    ] {
        assert_eq!(frame_request(bad), None, "{bad}");
    }
}
