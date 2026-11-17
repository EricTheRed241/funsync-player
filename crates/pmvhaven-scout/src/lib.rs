//! `npm run pmvhaven:scout`: PMVHaven videos that have a funscript, screened
//! and scored the way the other scouts do (shared screening, candidateScore)
//! and written to `<userData>/pmvhaven-scout.json` for the `scout:report`
//! swipe deck (Multi / VR / Flat tabs by `script.axes` and `vr`); `--fetch`
//! downloads the ones swiped right into the library source Other. No login;
//! never writes config.json.

pub mod api;
pub mod classify;
pub mod cli;
pub mod fetch;
pub mod library;
pub mod prefs;
pub mod scan;
pub mod script;
pub mod store;

use slr_scout::http::{self, Http, Pacer};
use slr_scout::score::Scorer;
use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};
use store::{Scene, ScoutFile};
use video_tags::host::{self, LockError};
use video_tags::timefmt;

/// Point the CLI at canned responses instead of the network (tests); the
/// fixture format of `slr_scout::http::Fake`.
pub const FAKE_ENV: &str = "PMVHAVEN_SCOUT_FAKE_HTTP";
/// Between requests to one host (pmvhaven.com, its file bucket).
pub const GAP: Duration = Duration::from_secs(1);
pub const LOCK_NAME: &str = "pmvhaven-scout.lock";

/// Run the CLI; returns the exit code.
pub fn run(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match run_inner(args, out, err) {
        Ok(code) => code,
        Err(msg) => {
            let _ = writeln!(err, "pmvhaven-scout: {msg}");
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
    let user_data = match opts.user_data.clone() {
        Some(dir) => dir,
        None => {
            host::default_user_data().ok_or("can't find the userData folder; pass --user-data")?
        }
    };
    if !user_data.is_dir() {
        return Err(format!("{} is not a folder", user_data.display()));
    }
    let lock_path = user_data.join(LOCK_NAME);
    let _lock = host::take_lock(&lock_path).map_err(|e| match e {
        LockError::Held(holder) => format!(
            "another pmvhaven-scout run ({holder}) holds {}",
            lock_path.display()
        ),
        LockError::Io(m) => m,
    })?;
    let (client, gap): (Box<dyn Http>, Duration) =
        match std::env::var_os(FAKE_ENV).filter(|v| !v.is_empty()) {
            Some(path) => (
                Box::new(http::Fake::load(Path::new(&path))?),
                Duration::ZERO,
            ),
            None => (Box::new(http::Curl), GAP),
        };
    let http = client.as_ref();
    let shared = slr_scout::prefs::load()?;
    let own = prefs::load()?;

    if opts.fetch {
        return fetch::Fetch {
            http,
            user_data: &user_data,
            library_max_videos: shared.library_max_videos,
            dry_run: opts.dry_run,
            limit: opts.limit,
            files: Pacer::new(gap),
            fake: gap.is_zero(),
        }
        .run(out);
    }

    let started = Instant::now();
    let previous = store::read(&user_data)?;
    let mut cache = store::read_cache(&user_data)?;
    let config = video_tags::config::read_json(&user_data).ok();
    let user = config
        .as_ref()
        .map(slr_scout::library::from_json)
        .unwrap_or_default();
    let scorer = Scorer::new(&shared, &user, slr_scout::library::creators(&user.sources));
    let known = config
        .as_ref()
        .map(|c| library::known(c, &user.sources))
        .unwrap_or_default();
    let now = timefmt::now_iso();
    let outcome = scan::Scan {
        http,
        api: Pacer::new(gap),
        prefs: &shared,
        own: &own,
        scorer: &scorer,
        known: &known,
        now: now.clone(),
        max_pages: opts.pages,
        user_data: Some(&user_data),
    }
    .run(&mut cache, err);
    store::write_cache(&user_data, &cache)?;
    if outcome.listed == 0 && outcome.partial {
        writeln!(
            err,
            "pmvhaven-scout: PMVHaven's list couldn't be read; nothing written."
        )
        .map_err(io)?;
        return Ok(if outcome.rate_limited { 2 } else { 1 });
    }
    let mut scenes = outcome.scenes.clone();
    fetch::keep_fetched(previous.as_ref(), &mut scenes);
    let file = ScoutFile {
        version: store::VERSION,
        generated_at: now,
        scenes,
    };
    store::write(&user_data, &file)?;
    summary(&file, &outcome, out).map_err(io)?;
    writeln!(
        out,
        "Wrote {} in {}.",
        store::file_path(&user_data).display(),
        elapsed(started.elapsed())
    )
    .map_err(io)?;
    Ok(
        if outcome.rate_limited || outcome.left > 0 || outcome.partial {
            2
        } else {
            0
        },
    )
}

fn elapsed(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}m {:02}s", s / 60, s % 60)
}

/// Scenes listed in the run's summary (the file has them all).
const TOP: usize = 20;

fn summary(file: &ScoutFile, o: &scan::Outcome, out: &mut dyn Write) -> std::io::Result<()> {
    writeln!(
        out,
        "{} scripted video(s) listed of {} on PMVHaven ({} page(s){}); {} detail(s) read.",
        o.listed,
        o.total,
        o.pages,
        if o.partial { ", list cut short" } else { "" },
        o.details_read
    )?;
    let count = |f: &dyn Fn(&Scene) -> bool| file.scenes.iter().filter(|s| f(s)).count();
    writeln!(
        out,
        "Kept {}: {} VR, {} multi-axis, {} in the library, {} with a generated script.",
        file.scenes.len(),
        count(&|s| s.vr),
        count(&|s| s.script.as_ref().is_some_and(|x| !x.axes.is_empty())),
        count(&|s| s.in_library),
        count(&|s| s.script.as_ref().is_some_and(|x| x.generated)),
    )?;
    let dropped: Vec<String> = o.dropped.iter().map(|(k, n)| format!("{k} {n}")).collect();
    writeln!(
        out,
        "Dropped {}: {}.",
        o.dropped.values().sum::<usize>(),
        if dropped.is_empty() {
            "none".to_string()
        } else {
            dropped.join(", ")
        }
    )?;
    if o.left > 0 {
        writeln!(
            out,
            "{} video(s) left for the next run (no details yet).",
            o.left
        )?;
    }
    let top: Vec<&Scene> = file.scenes.iter().take(TOP).collect();
    writeln!(out, "Top {}:", top.len())?;
    for s in top {
        let parts: Vec<String> = s
            .score_parts
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect();
        writeln!(
            out,
            "  {:>5.2}  {}{} ({}) — {}",
            s.score,
            s.title,
            if s.vr { " [VR]" } else { "" },
            s.studio.as_deref().unwrap_or("?"),
            parts.join(", ")
        )?;
    }
    Ok(())
}
