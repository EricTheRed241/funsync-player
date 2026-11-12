//! `npm run age:screen`: the weekly underage screener. Every library video
//! is screened once (again when its file size changes): a contact sheet of
//! 12 frames goes to a judge (an omp run, one vision subagent per batch of
//! 15) that gives `p_minor`, the probability the video depicts someone under
//! 18. The thresholds (`ageScreen` in renderer/js/content-preferences.json)
//! decide: purge (1★ through the running app, which deletes it), the review
//! playlist, or clear. Every judged video goes into
//! `<userData>/age-screen.json`, so a video the user keeps is never flagged
//! again. The CLI never writes config.json or the library itself.

pub mod backend;
pub mod cli;
pub mod config;
pub mod judge;
pub mod plan;
pub mod prefs;
pub mod sheets;
pub mod state;
pub mod timefmt;

use backend::{Backend, Error as BackendError};
use cli::Mode;
use state::{Action, Entry, StateFile};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The rating that purges: the app deletes videos rated at or below
/// `ratings.deleteAtOrBelow` (favourites excepted).
pub const PURGE_RATING: f64 = 1.0;
pub const WORK_DIR: &str = "age-screen-work";

macro_rules! say {
    ($w:expr, $($t:tt)*) => {
        writeln!($w, $($t)*).map_err(io)?
    };
}

/// Run the CLI; returns the exit code.
pub fn run(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match run_inner(args, out, err) {
        Ok(code) => code,
        Err(msg) => {
            let _ = writeln!(err, "age-screen: {msg}");
            1
        }
    }
}

fn io(e: std::io::Error) -> String {
    format!("output: {e}")
}

fn run_inner(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> Result<i32, String> {
    let opts = cli::parse(args)?;
    if opts.help {
        write!(out, "{}", cli::USAGE).map_err(io)?;
        return Ok(0);
    }
    let mode = opts.mode.ok_or("no mode")?;
    let prefs = prefs::load()?;
    let user_data = match opts.user_data.clone() {
        Some(dir) => dir,
        None => video_tags::host::default_user_data()
            .ok_or("can't find the userData folder; pass --user-data")?,
    };
    if !user_data.is_dir() {
        return Err(format!("{} is not a folder", user_data.display()));
    }
    let state_path = state::file_path(&user_data);

    if mode == Mode::List {
        let current = state::read(&state_path)?;
        list(&state_path, current.as_ref(), out).map_err(io)?;
        return Ok(0);
    }

    let _lock = state::lock(&user_data)?;
    let mut st = state::read(&state_path)?.unwrap_or_else(StateFile::empty);
    let now = timefmt::now_ms();
    let interval = prefs.age_screen.interval_days;
    if mode == Mode::Weekly && !state::is_due(st.last_run.as_deref(), now, interval) {
        let last = st.last_run.as_deref().unwrap_or("?");
        let next = timefmt::parse_iso(last)
            .map(|ms| timefmt::iso_utc(ms + (interval * 86_400_000.0) as i64))
            .unwrap_or_default();
        say!(
            out,
            "Not due: last complete run {last}; the next is due from {next}."
        );
        return Ok(0);
    }

    let backend = Backend::new(&backend::url(opts.backend.as_deref()))?;
    let videos = match backend.videos() {
        Ok(v) => v,
        Err(BackendError::Down(m)) => {
            say!(err, "age-screen: {m}");
            return Ok(2);
        }
        Err(BackendError::Bad(m)) => return Err(m),
    };
    let (mut candidates, skipped) = plan::select(&videos, &st, &prefs, |p| {
        fs::metadata(p)
            .ok()
            .filter(|m| m.is_file())
            .map(|m| m.len())
    });
    say!(
        out,
        "{} video(s) in the library: {} to screen; skipped {} screened before, {} rated {} or lower, {} AppleDouble, {} not on disk.",
        videos.len(),
        candidates.len(),
        skipped.screened,
        skipped.rated_out,
        prefs.ratings.delete_at_or_below,
        skipped.apple_double,
        skipped.missing
    );
    let waiting = match opts.limit {
        Some(n) if candidates.len() > n => {
            let rest = candidates.len() - n;
            candidates.truncate(n);
            say!(
                out,
                "--limit {n}: screening {n}, {rest} wait for a later run."
            );
            rest
        }
        _ => 0,
    };
    let dry = mode == Mode::DryRun;

    if candidates.is_empty() {
        say!(out, "Nothing to screen.");
        if !dry {
            st.last_run = Some(timefmt::iso_utc(now));
            state::write_atomic(&user_data, &st)?;
        }
        return Ok(0);
    }

    let vr = match backend.vr_flags() {
        Ok(flags) => flags,
        Err(BackendError::Down(m)) => {
            say!(err, "age-screen: {m}");
            return Ok(2);
        }
        Err(BackendError::Bad(m)) => {
            say!(
                err,
                "age-screen: VR flags unavailable ({m}); VR sheets show both eyes"
            );
            HashMap::new()
        }
    };
    let sources = config::sources(&user_data)?;
    let titles: Vec<String> = candidates
        .iter()
        .map(|c| config::title(&c.video.path, &sources))
        .collect();

    let work = user_data.join(WORK_DIR);
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;

    // 1. Contact sheets and batch manifests.
    let mut used = HashSet::new();
    let jobs: Vec<sheets::Job> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut file_id = sheets::file_id(&c.video.id_text());
            if !used.insert(file_id.clone()) {
                file_id = format!("{file_id}-{i}");
                used.insert(file_id.clone());
            }
            sheets::Job {
                file_id,
                path: c.video.path.clone(),
                duration: c.video.duration,
                vr: vr.get(&c.video.path).copied().unwrap_or(false),
            }
        })
        .collect();
    say!(
        out,
        "Making contact sheets for {} video(s) in {} …",
        jobs.len(),
        work.display()
    );
    out.flush().map_err(io)?;
    let mut sheet_of: Vec<Option<PathBuf>> = vec![None; jobs.len()];
    let mut left = 0usize;
    let mut finished = 0usize;
    sheets::make_sheets(&work, &jobs, |i, result| {
        finished += 1;
        match result {
            Ok(sheet) => sheet_of[i] = Some(sheet),
            Err(e) => {
                left += 1;
                let _ = writeln!(err, "  no sheet   {}: {e}; stays unscreened", titles[i]);
            }
        }
        if finished.is_multiple_of(25) && finished < jobs.len() {
            let _ = writeln!(out, "  {finished}/{} sheets", jobs.len());
            let _ = out.flush();
        }
    });
    let entries: Vec<sheets::ManifestEntry> = candidates
        .iter()
        .zip(&sheet_of)
        .zip(&titles)
        .filter_map(|((c, sheet), title)| {
            Some(sheets::ManifestEntry {
                id: c.video.id_text(),
                title: title.clone(),
                sheet: sheet.as_ref()?.to_string_lossy().into_owned(),
            })
        })
        .collect();

    // 2. Judge.
    let mut judged: Vec<(usize, judge::Verdict)> = Vec::new();
    if !entries.is_empty() {
        let batches = sheets::write_batches(&work, entries)?;
        let command = opts.judge_cmd.clone().unwrap_or_else(|| {
            judge::default_command(opts.max_cost.unwrap_or(judge::DEFAULT_MAX_COST))
        });
        say!(
            out,
            "Judging {} sheet(s) in {} batch(es): {command} …",
            batches.iter().map(|b| b.entries.len()).sum::<usize>(),
            batches.len()
        );
        out.flush().map_err(io)?;
        if let Err(e) = judge::run(&command, &judge::prompt(&work, &batches), &work) {
            say!(
                err,
                "age-screen: {e}; using the results files it wrote, the rest stay unscreened"
            );
        }
        let index: HashMap<String, usize> = candidates
            .iter()
            .enumerate()
            .map(|(i, c)| (c.video.id_text(), i))
            .collect();
        for b in &batches {
            match judge::read_results(b) {
                Ok(verdicts) => judged.extend(
                    verdicts
                        .into_iter()
                        .filter_map(|v| Some((*index.get(&v.id)?, v))),
                ),
                Err(e) => {
                    left += b.entries.len();
                    say!(
                        err,
                        "  batch {}: {e} ({}); its {} video(s) stay unscreened",
                        b.name,
                        b.results.display(),
                        b.entries.len()
                    );
                }
            }
        }
    }

    // 3. Apply.
    let screened_at = timefmt::iso_utc(timefmt::now_ms());
    let playlist = &prefs.age_screen.playlist;
    let mut rows: Vec<(Action, f64, &str, String)> = Vec::new();
    for (i, verdict) in judged {
        let c = &candidates[i];
        let action = plan::decide(verdict.p_minor, c.video.favorite, &prefs);
        if !dry {
            let applied = match action {
                Action::Purged => backend.rate(&c.video.id, PURGE_RATING),
                Action::Review | Action::ReviewFavourite => {
                    backend.add_to_playlist(&c.video.id, playlist)
                }
                Action::Clear => Ok(()),
            };
            if let Err(e) = applied {
                left += 1;
                say!(
                    err,
                    "  failed     {}: {} not applied ({e}); stays unscreened",
                    titles[i],
                    action.label()
                );
                continue;
            }
            st.videos.insert(
                c.video.path.clone(),
                Entry {
                    size: c.size,
                    screened_at: screened_at.clone(),
                    p_minor: verdict.p_minor,
                    reason: verdict.reason.clone(),
                    action,
                    extra: Default::default(),
                },
            );
        }
        rows.push((action, verdict.p_minor, &titles[i], verdict.reason));
    }

    report(&rows, left, waiting, dry, playlist, out).map_err(io)?;
    if dry {
        say!(
            out,
            "Dry run: no ratings, no playlist adds, {} not written. Sheets and results stay in {}.",
            state_path.display(),
            work.display()
        );
    } else {
        if left == 0 && waiting == 0 {
            st.last_run = Some(timefmt::iso_utc(timefmt::now_ms()));
        }
        state::write_atomic(&user_data, &st)?;
        let _ = fs::remove_dir_all(&work);
        say!(out, "Wrote {}.", state_path.display());
    }
    Ok(if left > 0 { 2 } else { 0 })
}

fn report(
    rows: &[(Action, f64, &str, String)],
    left: usize,
    waiting: usize,
    dry: bool,
    playlist: &str,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    let count = |a: Action| rows.iter().filter(|r| r.0 == a).count();
    writeln!(
        out,
        "{}: {} purged (1★), {} to \"{playlist}\", {} favourite(s) to \"{playlist}\", {} clear; {} unscreened{}.",
        if dry { "Planned (dry run)" } else { "Done" },
        count(Action::Purged),
        count(Action::Review),
        count(Action::ReviewFavourite),
        count(Action::Clear),
        left,
        if waiting > 0 {
            format!(", {waiting} waiting (--limit)")
        } else {
            String::new()
        }
    )?;
    // A real run lists what it flagged; a dry run every planned action.
    let mut shown: Vec<&(Action, f64, &str, String)> = rows
        .iter()
        .filter(|r| dry || r.0 != Action::Clear)
        .collect();
    shown.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)));
    for (action, p, title, reason) in shown {
        let what = match (action, dry) {
            (Action::Purged, false) => "purged",
            (Action::Purged, true) => "would purge",
            (Action::ReviewFavourite, false) => "review (fav)",
            (Action::ReviewFavourite, true) => "would review (fav)",
            (Action::Review, false) => "review",
            (Action::Review, true) => "would review",
            (Action::Clear, _) => "clear",
        };
        writeln!(out, "  {what:<18} p={p:.2}  {title}")?;
        if !reason.is_empty() {
            writeln!(out, "  {:<18}         {reason}", "")?;
        }
    }
    Ok(())
}

fn list(path: &Path, file: Option<&StateFile>, out: &mut dyn Write) -> std::io::Result<()> {
    let Some(file) = file else {
        return writeln!(
            out,
            "No videos screened yet ({} doesn't exist).",
            path.display()
        );
    };
    writeln!(
        out,
        "{}: {} video(s) screened, last complete run {}",
        path.display(),
        file.videos.len(),
        file.last_run.as_deref().unwrap_or("never")
    )?;
    let mut flagged: Vec<(&String, &Entry)> = file
        .videos
        .iter()
        .filter(|(_, e)| e.action != Action::Clear)
        .collect();
    if flagged.is_empty() {
        return writeln!(out, "No flagged videos.");
    }
    flagged.sort_by(|a, b| {
        a.1.action
            .cmp(&b.1.action)
            .then(b.1.p_minor.total_cmp(&a.1.p_minor))
    });
    for (path, e) in flagged {
        writeln!(
            out,
            "  {:<16} p={:.2}  {path}\n  {:<16}         {} (screened {})",
            e.action.label(),
            e.p_minor,
            "",
            e.reason,
            e.screened_at
        )?;
    }
    Ok(())
}
