//! Library scan: walk the source folders and pair every video with its
//! funscript, variants, axis companions, subtitle and scout sidecar.
//!
//! A port of `electron/library-scan.js` (and `electron/script-variant.js`);
//! the old app is the reference, so keep the two in step. Paths come from
//! `read_dir`, so they keep the drive's own spelling (NFC / NFD).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// Axis suffixes paired as companions (`Title.twist.funscript`).
pub const AXIS_SUFFIXES: [&str; 10] = [
    "surge", "sway", "twist", "roll", "pitch", "vib", "lube", "pump", "suction", "valve",
];
pub const VIDEO_EXTS: [&str; 11] = [
    ".mp4", ".m4v", ".mkv", ".webm", ".avi", ".mov", ".mp3", ".wav", ".ogg", ".flac", ".m4a",
];
pub const SUBTITLE_EXTS: [&str; 2] = [".srt", ".vtt"];
const SCOUT_SIDECAR: &str = "eroscripts.json";
const FAPTAP_SIDECAR: &str = "faptap.json";

/// macOS AppleDouble sidecars (`._Video.mp4`): skipped in every walk.
pub fn is_apple_double(name: &str) -> bool {
    name.starts_with("._")
}

/// The old backend's video id: `md5(path)[:12]`.
pub fn video_id(path: &str) -> String {
    format!("{:x}", md5::compute(path.as_bytes()))[..12].to_string()
}

#[derive(Debug, Clone)]
pub struct Source {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Variant {
    pub label: String,
    pub path: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Axis {
    pub axis: String,
    pub path: String,
    pub generated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EroscriptsRef {
    pub topic_id: serde_json::Value,
    pub url: Option<String>,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poster_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FaptapRef {
    pub faptap_id: String,
    pub url: Option<String>,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Video {
    pub id: String,
    pub name: String,
    pub path: String,
    pub ext: String,
    pub funscript_path: Option<String>,
    pub fuzzy_matched: bool,
    pub subtitle_path: Option<String>,
    pub variants: Vec<Variant>,
    /// Axis companions of the video's main script (or of the video's own
    /// name when the scene ships axes only), hand-made and generated.
    pub axes: Vec<Axis>,
    /// Axis files a manual `multi` association plays instead (devices only;
    /// the axis kind, categories and scores keep following `axes`, as the
    /// old backend registered the scan's pairing).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub manual_axes: Vec<Axis>,
    pub source_name: String,
    /// mtime in ms, the old app's "date added".
    pub date_added: f64,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eroscripts: Option<EroscriptsRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub faptap: Option<FaptapRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NamePath {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub videos: Vec<Video>,
    pub unmatched_funscripts: Vec<NamePath>,
    pub unmatched_subtitles: Vec<NamePath>,
    pub all_funscripts: Vec<NamePath>,
    pub failed_paths: Vec<String>,
}

struct Entry {
    name: String,
    dir: String,
}

impl Entry {
    fn path(&self) -> String {
        join(&self.dir, &self.name)
    }
}

fn join(dir: &str, name: &str) -> String {
    Path::new(dir).join(name).to_string_lossy().into_owned()
}

/// `path.extname`: the last dot of the base name, never a leading one.
fn extname(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[i..],
        _ => "",
    }
}

fn stem<'a>(name: &'a str, ext: &str) -> &'a str {
    &name[..name.len() - ext.len()]
}

/// Lowercase, `_ . -` as spaces, whitespace collapsed.
pub fn normalize_name(name: &str) -> String {
    let swapped: String = name
        .to_lowercase()
        .chars()
        .map(|c| if matches!(c, '_' | '.' | '-') { ' ' } else { c })
        .collect();
    swapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `/^(.+?)\s*\(([^)]+)\)\s*$/`: `Title (Soft)` → (`Title`, `Soft`).
fn paren_variant(name: &str) -> Option<(String, String)> {
    let t = name.trim_end();
    let body = t.strip_suffix(')')?;
    let after_close = body.rfind(')').map_or(0, |i| i + 1);
    let open = body[after_close..]
        .char_indices()
        .map(|(i, _)| i + after_close)
        .find(|&i| i >= 1 && body[i..].starts_with('('))?;
    let label = &body[open + 1..];
    if label.is_empty() {
        return None;
    }
    let head = body[..open].trim_end();
    let head = if head.is_empty() { &body[..1] } else { head };
    Some((head.to_string(), label.trim().to_string()))
}

/// `Title_hardcore` next to `Title.mp4` → (`Title`, `hardcore`); the
/// longest matching video stem wins, and a funscript named exactly after a
/// video is that video's primary.
pub fn underscore_variant(name: &str, video_stems: &[String]) -> Option<(String, String)> {
    if name.is_empty() || video_stems.is_empty() {
        return None;
    }
    let lower = name.to_lowercase();
    let mut best: Option<&String> = None;
    for s in video_stems {
        let sl = s.to_lowercase();
        if sl == lower {
            return None;
        }
        if lower.chars().count() > sl.chars().count() + 1
            && lower.starts_with(&format!("{sl}_"))
            && best.is_none_or(|b| s.chars().count() > b.chars().count())
        {
            best = Some(s);
        }
    }
    let best = best?;
    let label: String = name.chars().skip(best.chars().count() + 1).collect();
    Some((best.clone(), label))
}

fn label_cmp(a: &Variant, b: &Variant) -> std::cmp::Ordering {
    match (a.label == "Default", b.label == "Default") {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.label.to_lowercase().cmp(&b.label.to_lowercase()),
    }
}

fn name_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.cmp(b))
}

struct Funscript {
    name: String,
    path: String,
    dir: String,
    video_base: String,
    variant_label: Option<String>,
    axis: Option<String>,
    generated: bool,
    ambiguous_dot: bool,
    full_base: String,
    used: bool,
}

/// Walk one source. Any unreadable folder fails the whole source, as the
/// old scan's recursive `readdir` does.
fn walk(root: &Path, out: &mut Vec<Entry>, dirs: &mut Vec<Entry>) -> std::io::Result<()> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let dir_s = dir.to_string_lossy().into_owned();
        for e in fs::read_dir(&dir)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            if is_apple_double(&name) {
                continue;
            }
            let ft = e.file_type()?;
            if ft.is_dir() {
                stack.push(dir.join(&name));
                dirs.push(Entry {
                    name,
                    dir: dir_s.clone(),
                });
            } else if ft.is_file() {
                out.push(Entry {
                    name,
                    dir: dir_s.clone(),
                });
            }
        }
    }
    Ok(())
}

fn canonical(p: &str) -> String {
    let mut out = p.replace('\\', "/");
    while out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    out
}

pub fn scan_library(sources: &[Source]) -> ScanResult {
    let mut entries: Vec<Entry> = Vec::new();
    let mut seen = HashSet::new();
    let mut failed_paths = Vec::new();
    for src in sources {
        if src.path.is_empty() {
            continue;
        }
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        if walk(Path::new(&src.path), &mut files, &mut dirs).is_err() {
            failed_paths.push(src.path.clone());
            continue;
        }
        for f in files {
            if seen.insert(canonical(&f.path())) {
                entries.push(f);
            }
        }
    }

    let ext_of = |e: &Entry| extname(&e.name).to_lowercase();

    let mut video_base_local = HashSet::new();
    let mut video_base_global = HashSet::new();
    let mut video_stems_by_dir: HashMap<&str, Vec<String>> = HashMap::new();
    for e in &entries {
        let ext = ext_of(e);
        if !VIDEO_EXTS.contains(&ext.as_str()) {
            continue;
        }
        let vstem = stem(&e.name, &ext);
        let vbase = normalize_name(vstem);
        video_base_local.insert(format!("{}\0{}", e.dir, vbase));
        video_base_global.insert(vbase);
        video_stems_by_dir
            .entry(&e.dir)
            .or_default()
            .push(vstem.to_string());
    }

    let mut list: Vec<Funscript> = Vec::new();
    for e in &entries {
        let ext = ext_of(e);
        if ext != ".funscript" {
            continue;
        }
        let name_no_ext = stem(&e.name, &ext);
        let dot_idx = name_no_ext.rfind('.');
        let dot_suffix = dot_idx
            .map(|i| name_no_ext[i + 1..].to_lowercase())
            .filter(|s| !s.is_empty());
        let is_axis = dot_suffix
            .as_deref()
            .is_some_and(|s| AXIS_SUFFIXES.contains(&s));
        let paren = paren_variant(name_no_ext);
        let uvar = if is_axis || paren.is_some() {
            None
        } else {
            underscore_variant(
                name_no_ext,
                video_stems_by_dir
                    .get(e.dir.as_str())
                    .map_or(&[][..], |v| v),
            )
        };
        let full_base = normalize_name(name_no_ext);
        let mut generated = false;
        let mut ambiguous_dot = false;
        let (video_base, variant_label) = if is_axis {
            let mut st = &name_no_ext[..dot_idx.unwrap_or(0)];
            if st.to_lowercase().ends_with(".generated") {
                st = &st[..st.len() - ".generated".len()];
                generated = true;
            }
            (normalize_name(st), None)
        } else if let Some((head, label)) = paren {
            if video_base_local.contains(&format!("{}\0{}", e.dir, full_base))
                || video_base_global.contains(&full_base)
            {
                (full_base.clone(), None)
            } else {
                (normalize_name(&head), Some(label))
            }
        } else if let Some((st, label)) = uvar {
            (normalize_name(&st), Some(label))
        } else if let (Some(sfx), Some(i)) = (&dot_suffix, dot_idx.filter(|&i| i > 0)) {
            ambiguous_dot = true;
            (normalize_name(&name_no_ext[..i]), Some(sfx.clone()))
        } else {
            (full_base.clone(), None)
        };
        list.push(Funscript {
            name: e.name.clone(),
            path: e.path(),
            dir: e.dir.clone(),
            video_base,
            variant_label,
            axis: if is_axis { dot_suffix.clone() } else { None },
            generated,
            ambiguous_dot,
            full_base,
            used: false,
        });
    }

    // Demote ambiguous dot variants (`S01.E03`, `Title.2024`) with no
    // primary sibling to primaries of their full name.
    let mut sib_local = HashSet::new();
    let mut sib_global = HashSet::new();
    for f in &list {
        if f.axis.is_none() && !f.ambiguous_dot {
            sib_local.insert(format!("{}\0{}", f.dir, f.video_base));
            sib_global.insert(f.video_base.clone());
        }
    }
    for f in list.iter_mut().filter(|f| f.ambiguous_dot) {
        f.ambiguous_dot = false;
        if sib_local.contains(&format!("{}\0{}", f.dir, f.video_base))
            || sib_global.contains(&f.video_base)
        {
            continue;
        }
        f.video_base = f.full_base.clone();
        f.variant_label = None;
    }

    // Primary maps: defaults first, then any non-axis script.
    let mut map_local: HashMap<String, usize> = HashMap::new();
    let mut map_global: HashMap<String, usize> = HashMap::new();
    for pass_defaults in [true, false] {
        for (i, f) in list.iter().enumerate() {
            if f.axis.is_some() || (pass_defaults && f.variant_label.is_some()) {
                continue;
            }
            map_local
                .entry(format!("{}\0{}", f.dir, f.video_base))
                .or_insert(i);
            map_global.entry(f.video_base.clone()).or_insert(i);
        }
    }

    struct Sub {
        name: String,
        path: String,
        used: bool,
    }
    let mut subs: Vec<Sub> = Vec::new();
    let mut sub_local: HashMap<String, usize> = HashMap::new();
    let mut sub_global: HashMap<String, usize> = HashMap::new();
    for e in &entries {
        let ext = ext_of(e);
        if !SUBTITLE_EXTS.contains(&ext.as_str()) {
            continue;
        }
        let base = normalize_name(stem(&e.name, &ext));
        let key = format!("{}\0{}", e.dir, base);
        if sub_local.contains_key(&key) && sub_global.contains_key(&base) {
            continue;
        }
        subs.push(Sub {
            name: e.name.clone(),
            path: e.path(),
            used: false,
        });
        let i = subs.len() - 1;
        sub_local.entry(key).or_insert(i);
        sub_global.entry(base).or_insert(i);
    }

    let mut videos = Vec::new();
    for e in &entries {
        let ext = ext_of(e);
        if !VIDEO_EXTS.contains(&ext.as_str()) {
            continue;
        }
        let base = normalize_name(stem(&e.name, &ext));
        let key = format!("{}\0{}", e.dir, base);
        let fs_i = map_local
            .get(&key)
            .or_else(|| map_global.get(&base))
            .copied();
        if let Some(i) = fs_i {
            list[i].used = true;
        }
        let sub_i = sub_local
            .get(&key)
            .or_else(|| sub_global.get(&base))
            .copied();
        if let Some(i) = sub_i {
            subs[i].used = true;
        }
        let mut variants = Vec::new();
        for f in list.iter_mut() {
            if f.video_base == base && f.axis.is_none() && f.dir == e.dir {
                f.used = true;
                variants.push(variant_of(f));
            }
        }
        if variants.is_empty() {
            for f in list.iter_mut() {
                if f.video_base == base && f.axis.is_none() {
                    f.used = true;
                    variants.push(variant_of(f));
                }
            }
        }
        variants.sort_by(label_cmp);
        let path = e.path();
        let source_name = sources
            .iter()
            .filter(|s| path.starts_with(&s.path))
            .max_by_key(|s| s.path.len())
            .map(|s| s.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| {
                Path::new(&e.dir)
                    .file_name()
                    .map_or("Library".into(), |n| n.to_string_lossy().into_owned())
            });
        let (date_added, size) = fs::metadata(&path)
            .map(|m| {
                let ms = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0.0, |d| d.as_secs_f64() * 1000.0);
                (ms, m.len())
            })
            .unwrap_or((0.0, 0));
        videos.push(Video {
            id: video_id(&path),
            name: e.name.clone(),
            path,
            ext,
            funscript_path: fs_i.map(|i| list[i].path.clone()),
            fuzzy_matched: false,
            subtitle_path: sub_i.map(|i| subs[i].path.clone()),
            variants,
            axes: Vec::new(),
            manual_axes: Vec::new(),
            source_name,
            date_added,
            size,
            eroscripts: None,
            faptap: None,
        });
    }

    attach_sidecars(&entries, &mut videos);

    // Fuzzy pass: token Jaccard ≥ 98 for videos still without a script.
    for v in videos.iter_mut().filter(|v| v.funscript_path.is_none()) {
        let vbase = stem(&v.name, extname(&v.name)).to_string();
        let mut best: Option<usize> = None;
        let mut best_score = 0;
        for (i, f) in list.iter().enumerate() {
            if f.used || f.axis.is_some() {
                continue;
            }
            let s = fuzzy_score(&vbase, stem(&f.name, ".funscript"));
            if s > best_score {
                best_score = s;
                best = Some(i);
            }
        }
        let Some(bi) = best.filter(|_| best_score >= 98) else {
            continue;
        };
        v.funscript_path = Some(list[bi].path.clone());
        v.fuzzy_matched = true;
        list[bi].used = true;
        let mut fv: Vec<Variant> = Vec::new();
        for f in &list {
            if f.axis.is_none()
                && fuzzy_score(&vbase, stem(&f.name, ".funscript")) >= 98
                && !fv.iter().any(|x| x.path == f.path)
            {
                fv.push(variant_of(f));
            }
        }
        if fv.len() > 1 {
            fv.sort_by(label_cmp);
            v.variants = fv;
        }
    }

    // Axis companions belong to the stroke script named like them, or to a
    // video of that name when the scene ships axes only.
    let mut paired = HashSet::new();
    for f in &list {
        if f.used && f.axis.is_none() {
            paired.insert(format!("{}\0{}", f.dir, f.full_base));
        }
    }
    for f in list.iter_mut().filter(|f| f.axis.is_some()) {
        let key = format!("{}\0{}", f.dir, f.video_base);
        if paired.contains(&key) || video_base_local.contains(&key) {
            f.used = true;
        }
    }
    let mut axes_by_key: HashMap<String, Vec<Axis>> = HashMap::new();
    for f in list.iter().filter(|f| f.axis.is_some()) {
        axes_by_key
            .entry(format!("{}\0{}", f.dir, f.video_base))
            .or_default()
            .push(Axis {
                axis: f.axis.clone().unwrap_or_default(),
                path: f.path.clone(),
                generated: f.generated,
            });
    }
    for v in &mut videos {
        let key = match &v.funscript_path {
            Some(p) => {
                let p = Path::new(p);
                let dir = p
                    .parent()
                    .map_or(String::new(), |d| d.to_string_lossy().into_owned());
                let name = p
                    .file_name()
                    .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                format!("{}\0{}", dir, normalize_name(stem(&name, extname(&name))))
            }
            None => {
                let dir = Path::new(&v.path)
                    .parent()
                    .map_or(String::new(), |d| d.to_string_lossy().into_owned());
                format!("{}\0{}", dir, normalize_name(stem(&v.name, &v.ext)))
            }
        };
        if let Some(axes) = axes_by_key.get(&key) {
            let mut axes = axes.clone();
            axes.sort_by(|a, b| a.axis.cmp(&b.axis).then(a.generated.cmp(&b.generated)));
            v.axes = axes;
        }
    }

    let mut unmatched_funscripts: Vec<NamePath> = list
        .iter()
        .filter(|f| !f.used)
        .map(|f| NamePath {
            name: f.name.clone(),
            path: f.path.clone(),
        })
        .collect();
    unmatched_funscripts.sort_by(|a, b| name_cmp(&a.name, &b.name));
    let mut unmatched_subtitles: Vec<NamePath> = sub_global
        .values()
        .filter(|&&i| !subs[i].used)
        .map(|&i| NamePath {
            name: subs[i].name.clone(),
            path: subs[i].path.clone(),
        })
        .collect();
    unmatched_subtitles.sort_by(|a, b| name_cmp(&a.name, &b.name));
    let mut all_funscripts: Vec<NamePath> = list
        .iter()
        .map(|f| NamePath {
            name: f.name.clone(),
            path: f.path.clone(),
        })
        .collect();
    all_funscripts.sort_by(|a, b| name_cmp(&a.name, &b.name));
    videos.sort_by(|a, b| name_cmp(&a.name, &b.name));

    ScanResult {
        videos,
        unmatched_funscripts,
        unmatched_subtitles,
        all_funscripts,
        failed_paths,
    }
}

fn variant_of(f: &Funscript) -> Variant {
    Variant {
        label: f.variant_label.clone().unwrap_or_else(|| "Default".into()),
        path: f.path.clone(),
        name: f.name.clone(),
    }
}

fn fuzzy_tokens(s: &str) -> HashSet<String> {
    s.to_lowercase()
        .chars()
        .map(|c| if "_.-()[]{}".contains(c) { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

pub fn fuzzy_score(a: &str, b: &str) -> u32 {
    let ta = fuzzy_tokens(a);
    let tb = fuzzy_tokens(b);
    if ta.is_empty() || tb.is_empty() {
        return 0;
    }
    let inter = ta.intersection(&tb).count();
    let union = ta.union(&tb).count();
    ((inter as f64 / union as f64) * 100.0).round() as u32
}

fn fold(s: &str) -> String {
    s.nfc().collect::<String>().to_lowercase()
}

fn clean_tags(v: Option<&serde_json::Value>) -> Vec<String> {
    v.and_then(|t| t.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|t| t.as_str())
                .filter(|t| !t.trim().is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// `https://<host>/…` with no user info or port; the host part.
fn https_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let auth = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    if auth.is_empty() || auth.contains('@') || auth.contains(':') {
        return None;
    }
    Some(auth)
}

pub fn is_faptap_thumbnail_url(url: &str) -> bool {
    https_host(url).is_some_and(|h| {
        h == "faptap.net"
            || h.strip_prefix("vz-")
                .and_then(|r| r.strip_suffix(".b-cdn.net"))
                .is_some_and(|m| {
                    !m.is_empty()
                        && m.chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                })
    })
}

pub fn is_rule34video_poster_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://rule34video.com/contents/videos_screenshots/")
    else {
        return false;
    };
    let Some(rest) = rest.strip_suffix("/preview.jpg") else {
        return false;
    };
    let Some((group, id)) = rest.split_once('/') else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.len() <= 12 && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(group) || !digits(id) {
        return false;
    }
    let (Ok(g), Ok(i)) = (group.parse::<u64>(), id.parse::<u64>()) else {
        return false;
    };
    i > 0 && g == i / 1000 * 1000 && i.to_string() == id && g.to_string() == group
}

fn attach_sidecars(entries: &[Entry], videos: &mut [Video]) {
    let mut sidecar_dirs: HashMap<PathBuf, Vec<&str>> = HashMap::new();
    for e in entries {
        if e.name == SCOUT_SIDECAR || e.name == FAPTAP_SIDECAR {
            sidecar_dirs
                .entry(PathBuf::from(&e.dir))
                .or_default()
                .push(&e.name);
        }
    }
    for v in videos.iter_mut() {
        let dir = Path::new(&v.path)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let Some(names) = sidecar_dirs.get(&dir) else {
            continue;
        };
        for name in names {
            let Ok(text) = fs::read_to_string(dir.join(name)) else {
                continue;
            };
            let Ok(rec) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            if let Some(file) = rec.get("file").and_then(|f| f.as_str())
                && !file.is_empty()
                && fold(file) != fold(&v.name)
            {
                continue;
            }
            let url = rec.get("url").and_then(|u| u.as_str()).map(str::to_string);
            if *name == SCOUT_SIDECAR {
                if let Some(id) = rec.get("topicId").filter(|i| !i.is_null()) {
                    v.eroscripts = Some(EroscriptsRef {
                        topic_id: id.clone(),
                        url,
                        tags: clean_tags(rec.get("tags")),
                        poster_url: rec
                            .get("posterUrl")
                            .and_then(|p| p.as_str())
                            .filter(|p| is_rule34video_poster_url(p))
                            .map(str::to_string),
                    });
                }
            } else {
                let id = match rec.get("faptapId") {
                    Some(serde_json::Value::String(s)) if !s.is_empty() => s.clone(),
                    Some(serde_json::Value::Number(n)) => n.to_string(),
                    _ => continue,
                };
                v.faptap = Some(FaptapRef {
                    faptap_id: id,
                    url,
                    tags: clean_tags(rec.get("tags")),
                    thumbnail_url: rec
                        .get("thumbnailUrl")
                        .and_then(|p| p.as_str())
                        .filter(|p| is_faptap_thumbnail_url(p))
                        .map(str::to_string),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    fn scan(root: &Path) -> ScanResult {
        scan_library(&[Source {
            name: "Lib".into(),
            path: root.to_string_lossy().into_owned(),
        }])
    }

    fn by_name<'a>(r: &'a ScanResult, name: &str) -> &'a Video {
        r.videos.iter().find(|v| v.name == name).expect(name)
    }

    #[test]
    fn paren_title_vs_variant() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), "a/Title.mp4", "");
        touch(t.path(), "a/Title (Soft).funscript", "");
        touch(t.path(), "a/Title.funscript", "");
        touch(t.path(), "b/Show (Nude).mp4", "");
        touch(t.path(), "b/Show (Nude).funscript", "");
        let r = scan(t.path());
        let v = by_name(&r, "Title.mp4");
        assert!(
            v.funscript_path
                .as_deref()
                .unwrap()
                .ends_with("Title.funscript")
        );
        let labels: Vec<_> = v.variants.iter().map(|x| x.label.as_str()).collect();
        assert_eq!(labels, ["Default", "Soft"]);
        assert!(
            by_name(&r, "Show (Nude).mp4")
                .funscript_path
                .as_deref()
                .unwrap()
                .ends_with("Show (Nude).funscript")
        );
    }

    #[test]
    fn underscore_and_dot_variants() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), "Title.mp4", "");
        touch(t.path(), "Title.funscript", "");
        touch(t.path(), "Title_hardcore.funscript", "");
        touch(t.path(), "Title.intense.funscript", "");
        touch(t.path(), "S01.E03.mp4", "");
        touch(t.path(), "S01.E03.funscript", "");
        let r = scan(t.path());
        let labels: Vec<_> = by_name(&r, "Title.mp4")
            .variants
            .iter()
            .map(|x| x.label.clone())
            .collect();
        assert_eq!(labels, ["Default", "hardcore", "intense"]);
        // No `S01` primary exists: the dotted name is demoted to a primary.
        assert!(by_name(&r, "S01.E03.mp4").funscript_path.is_some());
        assert!(r.unmatched_funscripts.is_empty());
    }

    #[test]
    fn axes_hand_made_and_generated_pair_with_their_script() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), "s/Scene.mp4", "");
        touch(t.path(), "s/Scene.funscript", "");
        touch(t.path(), "s/Scene.twist.funscript", "");
        touch(t.path(), "s/Scene.generated.roll.funscript", "");
        touch(t.path(), "s/._Scene.mp4", "");
        touch(t.path(), "s/._Scene.funscript", "");
        let r = scan(t.path());
        assert_eq!(r.videos.len(), 1);
        let v = &r.videos[0];
        assert_eq!(
            v.axes
                .iter()
                .map(|a| (a.axis.as_str(), a.generated))
                .collect::<Vec<_>>(),
            [("roll", true), ("twist", false)]
        );
        assert!(r.unmatched_funscripts.is_empty());
        assert_eq!(r.all_funscripts.len(), 3);
    }

    #[test]
    fn same_dir_beats_global_and_subtitles_pair() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), "x/Clip.mp4", "");
        touch(t.path(), "x/Clip.srt", "");
        touch(t.path(), "y/Clip.mp4", "");
        touch(t.path(), "y/Clip.funscript", "");
        touch(t.path(), "z/Orphan.srt", "");
        let r = scan(t.path());
        for v in &r.videos {
            // y's script is global fallback for x too.
            assert!(v.funscript_path.as_deref().unwrap().contains("/y/"));
            assert!(v.subtitle_path.as_deref().unwrap().contains("/x/"));
        }
        assert_eq!(r.unmatched_subtitles.len(), 1);
    }

    #[test]
    fn fuzzy_pass_and_sidecars() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), "p/Hero [Clip].mp4", "");
        touch(t.path(), "q/hero clip.funscript", "");
        touch(
            t.path(),
            "p/eroscripts.json",
            r#"{"topicId": 42, "url": "u", "tags": ["pov", " ", 3],
                "posterUrl": "https://rule34video.com/contents/videos_screenshots/3000/3456/preview.jpg"}"#,
        );
        touch(t.path(), "q/clip.mp4", "");
        touch(
            t.path(),
            "q/faptap.json",
            r#"{"faptapId": 7, "file": "other.mp4"}"#,
        );
        let r = scan(t.path());
        let v = by_name(&r, "Hero [Clip].mp4");
        assert!(v.fuzzy_matched);
        let e = v.eroscripts.as_ref().unwrap();
        assert_eq!(e.tags, ["pov"]);
        assert!(e.poster_url.is_some());
        assert!(by_name(&r, "clip.mp4").faptap.is_none());
    }

    #[test]
    fn missing_source_is_failed_not_empty() {
        let r = scan_library(&[Source {
            name: "Gone".into(),
            path: "/nonexistent/funsync-rust-test".into(),
        }]);
        assert_eq!(r.failed_paths.len(), 1);
    }

    #[test]
    fn url_guards() {
        assert!(is_faptap_thumbnail_url("https://vz-ab1.b-cdn.net/x.jpg"));
        assert!(!is_faptap_thumbnail_url("https://evil.net/x.jpg"));
        assert!(!is_faptap_thumbnail_url("https://faptap.net:8443/x"));
        assert!(!is_rule34video_poster_url(
            "https://rule34video.com/contents/videos_screenshots/2000/3456/preview.jpg"
        ));
    }

    #[test]
    fn ids_match_the_old_backend() {
        // python3: hashlib.md5(b"/a/b.mp4").hexdigest()[:12]
        assert_eq!(video_id("/a/b.mp4"), "34122fba6f93");
    }
}
