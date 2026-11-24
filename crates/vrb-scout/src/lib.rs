//! `npm run vrb:scout`: the VRBangers group's VR scenes (vrbangers.com,
//! vrconk.com; `sites`) that come with the studio's free toy script,
//! screened and scored the way the EroScripts deck scores its candidates,
//! with `inLibrary` marking the ones the library already has. Writes
//! `<userData>/vrb-scout.json`, read by the `scout:report` swipe page;
//! `--fetch` downloads the scenes swiped right there
//! (`<userData>/vrb-scout-swipes.json`). Never writes config.json.

pub mod api;
pub mod browser;
pub mod cli;
pub mod fetch;
pub mod library;
pub mod nuxt;
pub mod prefs;
pub mod scene;
pub mod sites;
pub mod store;

use api::Listed;
use sites::{SITES, Site};
use slr_scout::http::{self, Http, NetError, Pacer};
use slr_scout::score::Scorer;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};
use store::{Scene, ScoutFile};
use video_tags::host::{self, LockError};
use video_tags::timefmt;

/// Point the CLI at canned responses instead of the network (tests); the
/// file format is `slr_scout::http::Fake`'s. Member pages come from it too
/// (`browser::Canned`).
pub const FAKE_ENV: &str = "VRB_SCOUT_FAKE_HTTP";
/// Between requests to one host.
pub const GAP: Duration = Duration::from_secs(1);
pub const LOCK_NAME: &str = "vrb-scout.lock";
/// Scenes listed in the run's summary.
const TOP: usize = 10;

/// Run the CLI; returns the exit code.
pub fn run(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match run_inner(args, out, err) {
        Ok(code) => code,
        Err(msg) => {
            let _ = writeln!(err, "vrb-scout: {msg}");
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
    // Another run is at it: work left for a later run (exit 2), as every scout's lock.
    let _lock = match host::take_lock(&lock_path) {
        Ok(lock) => lock,
        Err(LockError::Held(holder)) => {
            writeln!(
                err,
                "vrb-scout: another vrb-scout run ({holder}) holds {}",
                lock_path.display()
            )
            .map_err(io)?;
            return Ok(2);
        }
        Err(LockError::Io(m)) => return Err(m),
    };
    let (client, gap): (Box<dyn Http>, _) =
        match std::env::var_os(FAKE_ENV).filter(|v| !v.is_empty()) {
            Some(path) => (
                Box::new(http::Fake::load(Path::new(&path))?),
                Duration::ZERO,
            ),
            None => (Box::new(http::Curl), GAP),
        };
    let http = client.as_ref();
    let prefs = slr_scout::prefs::load()?;
    let sites = opts.sites();

    if opts.fetch {
        let fake = gap.is_zero();
        let mut canned = browser::Canned { http };
        let mut chrome = browser::LazyChrome::new(
            user_data.clone(),
            browser::credentials_path(
                opts.credentials.as_deref(),
                std::env::var(browser::CREDENTIALS_ENV).ok(),
            ),
            opts.headful,
        );
        let members: &mut dyn browser::Members = if fake { &mut canned } else { &mut chrome };
        let mut f = fetch::Fetch {
            http,
            members,
            user_data: &user_data,
            prefs: &prefs,
            sites,
            dry_run: opts.dry_run,
            limit: opts.limit,
            fake,
        };
        return f.run(out);
    }
    scan(http, gap, &user_data, &sites, &prefs, out, err)
}

/// One site's walk, for the summary.
#[derive(Debug, Default)]
struct SiteStats {
    listed: usize,
    scripted: usize,
    details: usize,
    detail_failures: usize,
    /// Why the listing couldn't be read (the previous scenes were kept).
    failed: Option<String>,
}

/// A scene built from its listing entry when its detail couldn't be read:
/// the previous detail kept (asked again next scan).
fn without_detail(site: &Site, l: &Listed, prev: Option<&Scene>) -> Scene {
    let mut s = scene::build(site, l, None);
    if let Some(p) = prev {
        s.categories = p.categories.clone();
        s.images = p.images.clone();
        s.fps = p.fps;
        s.passthrough = p.passthrough;
    }
    s
}

/// Walk one site: listing, then details of new or changed scenes.
#[allow(clippy::too_many_arguments)]
fn walk_site(
    http: &dyn Http,
    gap: Duration,
    site: &Site,
    previous: Option<&ScoutFile>,
    stats: &mut SiteStats,
    scenes: &mut Vec<Scene>,
    code: &mut i32,
    err: &mut dyn Write,
) {
    let mut pacer = Pacer::new(gap);
    let fps = api::digest_fps(http, &mut pacer, site).unwrap_or_else(|e| {
        let _ = writeln!(err, "Warning: {} digest: {e}; fps unknown.", site.name);
        BTreeMap::new()
    });
    let listing = match api::walk(http, &mut pacer, site, err) {
        Ok(l) => l,
        Err(e) => {
            *code = (*code).max(if matches!(e, NetError::RateLimited(_)) {
                2
            } else {
                1
            });
            stats.failed = Some(e.to_string());
            if let Some(p) = previous {
                scenes.extend(
                    p.scenes
                        .iter()
                        .chain(&p.excluded)
                        .filter(|s| s.site == site.key)
                        .cloned(),
                );
            }
            return;
        }
    };
    stats.listed = listing.len();
    let scripted: Vec<&Listed> = listing.iter().filter(|l| l.has_script).collect();
    stats.scripted = scripted.len();
    let to_read = scripted
        .iter()
        .filter(|l| {
            previous
                .and_then(|p| p.find(site.key, &l.id))
                .is_none_or(|p| p.list_key != l.key || p.updated_at.is_none())
        })
        .count();
    if to_read > 0 {
        let _ = writeln!(
            err,
            "  {}: reading {to_read} scene detail(s) (about {} min)...",
            site.name,
            (to_read as u64 * gap.as_secs()).div_ceil(60)
        );
    }
    let mut stopped = false;
    for l in scripted {
        let prev = previous.and_then(|p| p.find(site.key, &l.id));
        if let Some(p) = prev.filter(|p| p.list_key == l.key && p.updated_at.is_some()) {
            let mut s = p.clone();
            scene::refresh(&mut s, l);
            scenes.push(s);
            continue;
        }
        if stopped {
            scenes.push(without_detail(site, l, prev));
            continue;
        }
        match api::detail(http, &mut pacer, site, &l.slug, &fps) {
            Ok(d) => {
                stats.details += 1;
                if stats.details.is_multiple_of(100) {
                    let _ = writeln!(err, "  {}: {}/{to_read} details", site.name, stats.details);
                }
                scenes.push(scene::build(site, l, Some(&d)));
            }
            Err(e) => {
                stats.detail_failures += 1;
                if matches!(e, NetError::RateLimited(_)) {
                    let _ = writeln!(
                        err,
                        "Warning: {} rate limited us ({e}); the other details wait for the next scan.",
                        site.name
                    );
                    *code = (*code).max(2);
                    stopped = true;
                } else if stats.detail_failures <= 5 {
                    let _ = writeln!(err, "Warning: {e}");
                }
                scenes.push(without_detail(site, l, prev));
            }
        }
    }
}

fn scan(
    http: &dyn Http,
    gap: Duration,
    user_data: &Path,
    sites: &[&'static Site],
    prefs: &slr_scout::prefs::Prefs,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<i32, String> {
    let started = Instant::now();
    let vrb = prefs::load()?;
    let file_path = store::file_path(user_data);
    let previous = store::read(&file_path)?;
    let config = video_tags::config::read_json(user_data).ok();
    let user = config
        .as_ref()
        .map(slr_scout::library::from_json)
        .unwrap_or_default();
    let scorer = Scorer::new(prefs, &user, slr_scout::library::creators(&user.sources));
    let mut all: Vec<Scene> = Vec::new();
    let mut stats: BTreeMap<&str, SiteStats> = BTreeMap::new();
    let mut code = 0;
    for site in SITES {
        if !sites.contains(&site) {
            // Not walked this time: its scenes stay as they were (rescored below).
            if let Some(p) = &previous {
                all.extend(
                    p.scenes
                        .iter()
                        .chain(&p.excluded)
                        .filter(|s| s.site == site.key)
                        .cloned(),
                );
            }
            continue;
        }
        writeln!(err, "Listing {}...", site.name).map_err(io)?;
        let st = stats.entry(site.key).or_default();
        walk_site(
            http,
            gap,
            site,
            previous.as_ref(),
            st,
            &mut all,
            &mut code,
            err,
        );
    }
    let index = library::index(&user.sources);
    let mut kept = Vec::new();
    let mut excluded = Vec::new();
    for mut s in all {
        if s.fetched_at.is_none() {
            s.fetched_at = previous
                .as_ref()
                .and_then(|p| p.find(&s.site, &s.id))
                .and_then(|p| p.fetched_at.clone());
        }
        let reason = scene::assess(&mut s, &scorer, prefs, &vrb);
        s.in_library = s.fetched_at.is_some() || index.contains(&s);
        if reason.is_some() {
            excluded.push(s);
        } else {
            kept.push(s);
        }
    }
    scene::sort(&mut kept);
    excluded.sort_by(|a, b| a.site.cmp(&b.site).then(a.id.cmp(&b.id)));
    let file = ScoutFile {
        version: store::VERSION,
        generated_at: timefmt::now_iso(),
        scenes: kept,
        excluded,
    };
    store::write(user_data, &file)?;
    summary(&file, &stats, out).map_err(io)?;
    let secs = started.elapsed().as_secs();
    writeln!(
        out,
        "Wrote {} in {}m {:02}s.",
        file_path.display(),
        secs / 60,
        secs % 60
    )
    .map_err(io)?;
    Ok(code)
}

fn summary(
    file: &ScoutFile,
    stats: &BTreeMap<&str, SiteStats>,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    for site in SITES {
        let kept = file.scenes.iter().filter(|s| s.site == site.key);
        let (n, in_library) = kept.fold((0, 0), |(n, l), s| (n + 1, l + usize::from(s.in_library)));
        let out_n = file.excluded.iter().filter(|s| s.site == site.key).count();
        match stats.get(site.key) {
            Some(st) if st.failed.is_some() => writeln!(
                out,
                "{}: listing failed ({}); kept the previous {n} scene(s).",
                site.name,
                st.failed.as_deref().unwrap_or_default()
            )?,
            Some(st) => writeln!(
                out,
                "{}: {} listed, {} with a toy script; {n} kept ({in_library} in the library), {out_n} excluded; {} detail(s) read{}.",
                site.name,
                st.listed,
                st.scripted,
                st.details,
                if st.detail_failures > 0 {
                    format!(", {} failed", st.detail_failures)
                } else {
                    String::new()
                }
            )?,
            None if n + out_n > 0 => writeln!(
                out,
                "{}: not scanned; kept the previous {n} scene(s).",
                site.name
            )?,
            None => {}
        }
    }
    let mut reasons: BTreeMap<&str, usize> = BTreeMap::new();
    for s in &file.excluded {
        let key = s
            .excluded
            .as_deref()
            .map_or("?", |r| r.split(" (").next().unwrap_or(r));
        *reasons.entry(key).or_default() += 1;
    }
    if !reasons.is_empty() {
        let list: Vec<String> = reasons.iter().map(|(k, n)| format!("{k} {n}")).collect();
        writeln!(out, "Excluded: {}.", list.join(", "))?;
    }
    let top: Vec<&Scene> = file
        .scenes
        .iter()
        .filter(|s| !s.in_library)
        .take(TOP)
        .collect();
    if !top.is_empty() {
        writeln!(out, "Top {} not in the library:", top.len())?;
        for s in top {
            let name = sites::by_key(&s.site).map_or(s.site.as_str(), |x| x.name);
            writeln!(
                out,
                "  {:>6.2}  {name:<9}  {} — {}",
                s.score,
                s.title,
                if s.actors.is_empty() {
                    "?".to_string()
                } else {
                    s.actors.join(", ")
                }
            )?;
        }
    }
    Ok(())
}
