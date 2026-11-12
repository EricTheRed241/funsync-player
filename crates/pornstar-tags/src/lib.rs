//! `npm run pornstar:tags`: tag library videos with the pornstars they show.
//! The list and threshold are `pornstarTags` in
//! renderer/js/content-preferences.json; performers the VRBangers scout names
//! in its scene folders join it. Each run recomputes
//! `<userData>/pornstar-tags.json` from the library (applied tags per video,
//! every performer's scene count), which the app merges into
//! `library.scriptTags` on every scan, next to video-tags.json
//! (renderer/js/video-tags.js). The CLI never writes config.json or the
//! library.

pub mod cli;
pub mod library;
pub mod performers;
pub mod prefs;
pub mod store;
pub mod tally;
pub mod text;

use library::Video;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use store::PornstarFile;
use video_tags::{config, rescan, timefmt};

/// Run the CLI; returns the exit code.
pub fn run(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match run_inner(args, out) {
        Ok(code) => code,
        Err(msg) => {
            let _ = writeln!(err, "pornstar-tags: {msg}");
            1
        }
    }
}

fn run_inner(args: &[OsString], out: &mut dyn Write) -> Result<i32, String> {
    let opts = cli::parse(args)?;
    if opts.help {
        write!(out, "{}", cli::USAGE).map_err(io)?;
        return Ok(0);
    }
    let user_data = match opts.user_data.clone() {
        Some(dir) => dir,
        None => video_tags::host::default_user_data()
            .ok_or("can't find the userData folder; pass --user-data")?,
    };
    if !user_data.is_dir() {
        return Err(format!("{} is not a folder", user_data.display()));
    }
    let file_path = store::file_path(&user_data);
    let current = store::read(&file_path)?;

    if opts.list {
        list(&file_path, current.as_ref(), out).map_err(io)?;
        return Ok(0);
    }

    let now = timefmt::now_iso();
    if opts.daily {
        let last_run = store::last_run(&user_data);
        let updated = current.as_ref().and_then(|f| f.updated_at.as_deref());
        if store::ran_today(&[updated, last_run.as_deref()], &now) {
            writeln!(
                out,
                "Already ran today (last run {}); nothing to do.",
                last_run.as_deref().or(updated).unwrap_or("?")
            )
            .map_err(io)?;
            return Ok(0);
        }
    }

    let prefs = prefs::load()?;
    let config_json = config::read_json(&user_data)?;
    let app = config::from_json(&user_data, &config_json)?;
    let videos = library::scan(&app.sources, &library::topic_slugs(&config_json))?;
    let everyone = performers::all(&prefs.pornstars, &discovered(&videos));
    let result = tally::tally(&everyone, &videos, prefs.min_scenes);
    let mut next = result.file;

    report(
        &everyone,
        current.as_ref(),
        &next,
        &result.candidates,
        videos.len(),
        out,
    )
    .map_err(io)?;
    let changed = current.as_ref().is_none_or(|c| !c.same_content(&next));

    if !(opts.apply || opts.daily) {
        let what = if changed {
            "Re-run with --apply to write"
        } else {
            "Nothing to change in"
        };
        writeln!(
            out,
            "Dry run: nothing written. {what} {}.",
            file_path.display()
        )
        .map_err(io)?;
        return Ok(0);
    }

    if changed {
        next.updated_at = Some(now.clone());
        let backed_up = store::write(&user_data, &next)?;
        writeln!(
            out,
            "Wrote {}{}.",
            file_path.display(),
            if backed_up {
                format!(" (previous file kept as {})", store::BACKUP_NAME)
            } else {
                String::new()
            }
        )
        .map_err(io)?;
    } else {
        writeln!(out, "Nothing to change in {}.", file_path.display()).map_err(io)?;
    }
    store::record_run(&user_data, &now)?;
    if !changed {
        return Ok(0);
    }

    if opts.no_rescan {
        writeln!(
            out,
            "No rescan requested (--no-rescan); the app applies the file on its next library scan."
        )
        .map_err(io)?;
    } else {
        let backend = video_tags::host::backend_url(app.backend_port);
        match rescan::request_rescan(&backend, rescan::TIMEOUT) {
            Ok(status) => writeln!(out, "Rescan requested from the app ({status}).").map_err(io)?,
            Err(e) => writeln!(
                out,
                "Rescan not requested ({e}); the app applies the file on its next library scan."
            )
            .map_err(io)?,
        }
    }
    Ok(0)
}

fn io(e: std::io::Error) -> String {
    format!("output: {e}")
}

/// Performer names from the VRBangers scout's scene folders, first seen
/// first.
pub fn discovered(videos: &[Video]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for v in videos {
        for name in performers::vrbangers_performers(&v.folder) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

fn scenes(n: usize) -> String {
    format!("{n} scene{}", if n == 1 { "" } else { "s" })
}

fn status(applied: bool) -> &'static str {
    if applied { "applied" } else { "pending" }
}

fn report(
    everyone: &[performers::Performer],
    current: Option<&PornstarFile>,
    next: &PornstarFile,
    candidates: &BTreeMap<String, usize>,
    video_count: usize,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    writeln!(
        out,
        "Pornstars ({} video(s) with scripts read; a tag applies at {}+):",
        video_count,
        scenes(next.min_scenes)
    )?;
    let width = everyone.iter().map(|p| p.tag.len()).max().unwrap_or(0);
    for p in everyone {
        let Some(s) = next.pornstars.get(&p.tag) else {
            continue;
        };
        let mut notes = Vec::new();
        if p.discovered {
            notes.push("found in VRBangers scenes".to_string());
        }
        match current.and_then(|c| c.pornstars.get(&p.tag)) {
            Some(was) if was.applied != s.applied => {
                notes.push(format!("was {}", status(was.applied)));
            }
            None if current.is_some() => notes.push("new".to_string()),
            _ => {}
        }
        let notes = if notes.is_empty() {
            String::new()
        } else {
            format!("  ({})", notes.join(", "))
        };
        writeln!(
            out,
            "  {}  {:<width$}  {:>9}  {}{notes}",
            status(s.applied),
            p.tag,
            scenes(s.scenes),
            s.name
        )?;
    }
    if let Some(c) = current {
        for (tag, s) in &c.pornstars {
            if !next.pornstars.contains_key(tag) {
                writeln!(out, "  dropped  {tag}  ({}, no longer listed)", s.name)?;
            }
        }
    }

    let empty = BTreeMap::new();
    let before = current.map_or(&empty, |c| &c.videos);
    let paths: BTreeSet<&String> = before.keys().chain(next.videos.keys()).collect();
    let (mut gaining, mut losing) = (0, 0);
    let mut lines = Vec::new();
    for path in paths {
        let old: &[String] = before.get(path).map_or(&[], Vec::as_slice);
        let new: &[String] = next.videos.get(path).map_or(&[], Vec::as_slice);
        let added: Vec<&String> = new.iter().filter(|t| !old.contains(t)).collect();
        let dropped: Vec<&String> = old.iter().filter(|t| !new.contains(t)).collect();
        if added.is_empty() && dropped.is_empty() {
            continue;
        }
        gaining += usize::from(!added.is_empty());
        losing += usize::from(!dropped.is_empty());
        let marks = added
            .iter()
            .map(|t| format!("+ {t}"))
            .chain(dropped.iter().map(|t| format!("- {t}")))
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(format!("  {path}: {marks}"));
    }
    if !lines.is_empty() {
        writeln!(out, "Videos:")?;
        for line in &lines {
            writeln!(out, "{line}")?;
        }
    }

    if !candidates.is_empty() {
        writeln!(
            out,
            "Unconfirmed FapTap performers (not in pornstarTags; never applied):"
        )?;
        for (name, n) in candidates {
            writeln!(out, "  {name}  ({})", scenes(*n))?;
        }
    }

    let applied = next.pornstars.values().filter(|s| s.applied).count();
    writeln!(
        out,
        "Totals: {applied} tag(s) applied, {} pending; {} video(s) tagged; {gaining} gaining, {losing} losing tags.",
        next.pornstars.len() - applied,
        next.videos.len()
    )
}

fn list(file_path: &Path, file: Option<&PornstarFile>, out: &mut dyn Write) -> std::io::Result<()> {
    let Some(file) = file else {
        return writeln!(
            out,
            "No pornstar tags yet ({} doesn't exist; run with --apply).",
            file_path.display()
        );
    };
    writeln!(
        out,
        "{}: updated {}, a tag applies at {}+",
        file_path.display(),
        file.updated_at.as_deref().unwrap_or("never"),
        scenes(file.min_scenes)
    )?;
    let mut tags: Vec<(&String, &store::Pornstar)> = file.pornstars.iter().collect();
    tags.sort_by(|a, b| {
        b.1.applied
            .cmp(&a.1.applied)
            .then(b.1.scenes.cmp(&a.1.scenes))
            .then(a.0.cmp(b.0))
    });
    let width = tags.iter().map(|(t, _)| t.len()).max().unwrap_or(0);
    for (tag, s) in tags {
        writeln!(
            out,
            "  {}  {tag:<width$}  {:>9}  {}",
            status(s.applied),
            scenes(s.scenes),
            s.name
        )?;
    }
    writeln!(out, "Videos ({}):", file.videos.len())?;
    for (path, tags) in &file.videos {
        writeln!(out, "  {path}: {}", tags.join(", "))?;
    }
    Ok(())
}
