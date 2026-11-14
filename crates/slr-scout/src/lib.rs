//! `npm run slr:scout`: find SexLikeReal VR scenes of the wanted performers
//! (`slrScout` in renderer/js/content-preferences.json: its actors, the
//! favourite artists and the listed pornstars), score them the way the
//! EroScripts deck scores its candidates plus SLR-only extras, mark where a
//! download would take each script from, and record the account's remaining
//! download quota. Writes `<userData>/slr-scout.json`, read by the
//! `scout:report` swipe page; `--fetch` downloads the scenes swiped right.
//! Never writes config.json; the forum is read-only.

pub mod cdp;
pub mod cli;
pub mod fetch;
pub mod forum;
pub mod http;
pub mod library;
pub mod login;
pub mod performers;
pub mod prefs;
pub mod quota;
pub mod score;
pub mod slr;
pub mod store;
pub mod text;

use http::{Http, NetError, Pacer};
use quota::QuotaError;
use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};
use store::{Actor, Handmade, Quota, Scene, ScoutFile};
use video_tags::host::{self, LockError};
use video_tags::timefmt;

/// Between SLR API requests: its limit is 120 requests a minute
/// (`x-ratelimit-limit`), so a long walk never runs into it.
pub const SLR_GAP: Duration = Duration::from_millis(500);
/// Between forum requests.
pub const FORUM_GAP: Duration = Duration::from_secs(4);
pub const LOCK_NAME: &str = "slr-scout.lock";

/// Run the CLI; returns the exit code.
pub fn run(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match run_inner(args, out, err) {
        Ok(code) => code,
        Err(msg) => {
            let _ = writeln!(err, "slr-scout: {msg}");
            1
        }
    }
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
        LockError::Held(holder) => {
            format!(
                "another slr-scout run ({holder}) holds {}",
                lock_path.display()
            )
        }
        LockError::Io(m) => m,
    })?;

    let (client, slr_gap, forum_gap): (Box<dyn Http>, _, _) =
        match std::env::var_os(http::FAKE_ENV).filter(|v| !v.is_empty()) {
            Some(path) => (
                Box::new(http::Fake::load(Path::new(&path))?),
                Duration::ZERO,
                Duration::ZERO,
            ),
            None => (Box::new(http::Curl), SLR_GAP, FORUM_GAP),
        };
    let http = client.as_ref();
    let file_path = store::file_path(&user_data);
    let previous = store::read(&file_path)?;

    if opts.login {
        let path = login::credentials_path(
            opts.credentials.as_deref(),
            std::env::var(login::CREDENTIALS_ENV).ok(),
        );
        let creds = login::read_credentials(&path)?;
        let session = login::login(&user_data, &creds, &timefmt::now_iso(), err)?;
        quota::write_session(&user_data, &session)?;
        writeln!(
            out,
            "Signed in; session saved to {} (expires {}).",
            quota::session_path(&user_data).display(),
            session.expires.as_deref().unwrap_or("unknown")
        )
        .map_err(io)?;
    }
    if opts.quota || opts.login {
        return refresh_quota(http, &user_data, previous, out, err);
    }

    let prefs = prefs::load()?;
    let config = video_tags::config::read_json(&user_data).ok();
    let user = config.as_ref().map(library::from_json).unwrap_or_default();
    if opts.fetch {
        let mut f = fetch::Fetch {
            http,
            user_data: &user_data,
            prefs: &prefs,
            dry_run: opts.dry_run,
            slr: Pacer::new(slr_gap),
            forum: Pacer::new(forum_gap),
            fake: slr_gap.is_zero(),
        };
        return f.run(out);
    }
    let started = Instant::now();
    let scorer = score::Scorer::new(&prefs, &user, library::creators(&user.sources));
    let favourites: Vec<String> = scorer.favourites().map(str::to_string).collect();
    let wanted = performers::assemble(&prefs, &favourites, &opts.actors);
    writeln!(err, "Looking up {} performer(s) on SLR...", wanted.len()).map_err(io)?;
    let catalogue = match slr::walk(
        http,
        &mut Pacer::new(slr_gap),
        &wanted,
        &prefs.slr_scout,
        err,
    ) {
        Ok(c) => c,
        Err(NetError::RateLimited(m)) => {
            writeln!(
                err,
                "slr-scout: SLR rate limited us ({m}); nothing written, try later."
            )
            .map_err(io)?;
            return Ok(2);
        }
        Err(NetError::Failed(m)) => return Err(m),
    };
    let mut scenes = catalogue.scenes;
    let mut code = 0;

    let skipped = if opts.no_forum {
        Some("--no-forum".to_string())
    } else {
        match config.as_ref().and_then(forum::session_cookie) {
            None => Some("no EroScripts session in config.json".to_string()),
            Some(cookie) => {
                // Only performers with a kept scene: a post matches a scene.
                let names: Vec<String> = catalogue
                    .actors
                    .iter()
                    .filter(|a| kept(a, &scenes) > 0)
                    .map(|a| a.name.clone())
                    .collect();
                match forum::search_all(http, &mut Pacer::new(forum_gap), &cookie, &names, err) {
                    Ok(topics) => {
                        forum::attach(&mut scenes, &topics);
                        None
                    }
                    Err(e) => {
                        if matches!(e, NetError::RateLimited(_)) {
                            code = 2;
                        }
                        Some(format!("EroScripts search failed: {e}"))
                    }
                }
            }
        }
    };
    if let Some(why) = skipped {
        let kept = previous
            .as_ref()
            .map_or(0, |p| keep_forum_matches(p, &mut scenes));
        writeln!(
            err,
            "Warning: EroScripts not searched ({why}); kept {kept} match(es) from the previous run."
        )
        .map_err(io)?;
    }
    for s in &mut scenes {
        scorer.score(s);
        s.script_source = store::script_source(s);
    }
    fetch::keep_fetched(previous.as_ref(), &mut scenes);
    score::sort(&mut scenes);

    let now = timefmt::now_iso();
    let result = quota::fetch(http, &user_data, &now);
    let (quota, quota_error) = quota::merge(previous.as_ref(), &result);
    let file = ScoutFile {
        generated_at: now,
        actors: catalogue.actors,
        quota,
        quota_error,
        scenes,
    };
    store::write(&user_data, &file)?;
    summary(&file, &catalogue.unknown, &prefs.slr_scout, out).map_err(io)?;
    quota_line(&file, &result, out).map_err(io)?;
    writeln!(
        out,
        "Wrote {} in {}.",
        file_path.display(),
        elapsed(started.elapsed())
    )
    .map_err(io)?;
    Ok(quota_code(&result, code))
}

/// A performer's kept scenes.
fn kept(actor: &Actor, scenes: &[Scene]) -> usize {
    scenes
        .iter()
        .filter(|s| s.actors.contains(&actor.name))
        .count()
}

fn elapsed(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}m {:02}s", s / 60, s % 60)
}

/// Exit code for the quota result: 1 for a failure, else 2 / 3 when nothing
/// worse happened.
fn quota_code(result: &Result<Quota, QuotaError>, code: i32) -> i32 {
    match result {
        Ok(_) => code,
        Err(QuotaError::Failed(_)) => 1,
        Err(_) if code != 0 => code,
        Err(QuotaError::RateLimited(_)) => 2,
        Err(_) => 3,
    }
}

/// `--quota` / after `--login`: refresh the quota in the existing file (a
/// file without scenes when there is none).
fn refresh_quota(
    http: &dyn Http,
    user_data: &Path,
    previous: Option<ScoutFile>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<i32, String> {
    let now = timefmt::now_iso();
    let result = quota::fetch(http, user_data, &now);
    let (quota, quota_error) = quota::merge(previous.as_ref(), &result);
    let file = match previous {
        Some(mut p) => {
            p.quota = quota;
            p.quota_error = quota_error;
            p
        }
        None => ScoutFile {
            generated_at: now,
            actors: Vec::new(),
            quota,
            quota_error,
            scenes: Vec::new(),
        },
    };
    store::write(user_data, &file)?;
    quota_line(&file, &result, out).map_err(io)?;
    writeln!(out, "Wrote {}.", store::file_path(user_data).display()).map_err(io)?;
    if let Err(QuotaError::Failed(m)) = &result {
        writeln!(err, "slr-scout: quota: {m}").map_err(io)?;
    }
    Ok(quota_code(&result, 0))
}

/// Carry the previous run's EroScripts matches over to the same scenes;
/// returns how many.
pub fn keep_forum_matches(previous: &ScoutFile, scenes: &mut [Scene]) -> usize {
    let mut kept = 0;
    for s in scenes {
        for h in previous.forum_matches(s.id) {
            if !s.handmade.contains(&h) {
                s.handmade.push(h);
                kept += 1;
            }
        }
    }
    kept
}

fn io(e: std::io::Error) -> String {
    format!("output: {e}")
}

fn handmade_text(s: &Scene) -> String {
    let parts: Vec<String> = s
        .handmade
        .iter()
        .map(|h| match h {
            Handmade::Slr { author } => format!("slr:{author}"),
            Handmade::Eroscripts { topic_id, free, .. } => {
                format!("es#{topic_id}{}", if *free { " free" } else { " paid" })
            }
        })
        .collect();
    if parts.is_empty() {
        "-".into()
    } else {
        parts.join(", ")
    }
}

/// Scenes listed in the run's summary (the file has them all).
const TOP: usize = 20;

fn summary(
    file: &ScoutFile,
    unknown: &[String],
    prefs: &prefs::SlrScout,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    let mut wanted = Vec::new();
    if prefs.vr_only {
        wanted.push("VR");
    }
    if prefs.require_creampie {
        wanted.push("creampie");
    }
    writeln!(
        out,
        "{} scene(s){} from {} performer(s) on SLR:",
        file.scenes.len(),
        if wanted.is_empty() {
            String::new()
        } else {
            format!(" ({})", wanted.join(", "))
        },
        file.actors.len()
    )?;
    for a in &file.actors {
        writeln!(
            out,
            "  {}: {} on SLR, {} kept",
            a.name,
            a.scenes,
            kept(a, &file.scenes)
        )?;
    }
    if !unknown.is_empty() {
        writeln!(out, "Not on SLR (skipped): {}", unknown.join(", "))?;
    }
    let sources = |src: Option<store::ScriptSource>| {
        file.scenes
            .iter()
            .filter(|s| s.script_source == src)
            .count()
    };
    writeln!(
        out,
        "Scripts: {} from EroScripts, {} SLR hand-crafted, {} SLR AI, {} none.",
        sources(Some(store::ScriptSource::Eroscripts)),
        sources(Some(store::ScriptSource::SlrHandcrafted)),
        sources(Some(store::ScriptSource::SlrAi)),
        sources(None)
    )?;
    let top: Vec<&Scene> = file.scenes.iter().take(TOP).collect();
    let made: Vec<String> = top.iter().map(|s| handmade_text(s)).collect();
    let width = made.iter().map(String::len).max().unwrap_or(0).max(9);
    writeln!(out, "Top {}:", top.len())?;
    writeln!(out, "  score  {:<width$}  title — parts", "hand-made")?;
    for (s, made) in top.iter().zip(&made) {
        let parts: Vec<String> = s
            .score_parts
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect();
        writeln!(
            out,
            "  {:>5.2}  {made:<width$}  {} ({}) — {}",
            s.score,
            s.title,
            s.actors.join(", "),
            parts.join(", ")
        )?;
    }
    Ok(())
}

fn limit_text(l: &store::Limit) -> String {
    format!("{}/{}", l.remaining, l.total)
}

fn quota_line(
    file: &ScoutFile,
    result: &Result<Quota, QuotaError>,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    let describe = |q: &Quota| {
        format!(
            "hand-crafted scripts {}, AI scripts {}, videos {} left",
            limit_text(&q.scripts),
            limit_text(&q.ai_scripts),
            limit_text(&q.videos)
        )
    };
    match (result, &file.quota) {
        (Ok(q), _) => writeln!(out, "Quota: {} (checked {}).", describe(q), q.checked_at)?,
        (Err(e), kept) => {
            let kept = kept.as_ref().map_or("no earlier quota".to_string(), |q| {
                format!("kept the one checked {}: {}", q.checked_at, describe(q))
            });
            let hint = if e.needs_login() {
                "; sign in with npm run slr:scout -- --login"
            } else {
                ""
            };
            writeln!(out, "Quota: not read ({}){hint}; {kept}.", e.message())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_rank_errors() {
        let failed = Err(QuotaError::Failed("x".into()));
        assert_eq!(quota_code(&failed, 0), 1);
        assert_eq!(quota_code(&failed, 2), 1);
        assert_eq!(quota_code(&Err(QuotaError::Expired), 0), 3);
        assert_eq!(quota_code(&Err(QuotaError::NoSession), 2), 2);
        assert_eq!(quota_code(&Err(QuotaError::RateLimited("x".into())), 0), 2);
    }

    #[test]
    fn keeps_previous_forum_matches() {
        let detail = |id: u64, title: &str| {
            slr::scene(&serde_json::json!({"id": id, "label": format!("x-{id}"), "title": title}))
                .unwrap()
        };
        let mut old = detail(1, "Please Creampie Me");
        old.handmade = vec![
            Handmade::Slr {
                author: "kimera".into(),
            },
            Handmade::Eroscripts {
                topic_id: 273166,
                title: "Please Creampie Me".into(),
                free: false,
                multi_axis: false,
            },
        ];
        let previous = ScoutFile {
            generated_at: String::new(),
            actors: vec![],
            quota: None,
            quota_error: None,
            scenes: vec![old],
        };
        let mut scenes = vec![detail(1, "Please Creampie Me"), detail(2, "Other")];
        assert_eq!(keep_forum_matches(&previous, &mut scenes), 1);
        assert_eq!(keep_forum_matches(&previous, &mut scenes), 0);
        assert!(matches!(
            scenes[0].handmade[..],
            [Handmade::Eroscripts {
                topic_id: 273166,
                ..
            }]
        ));
        assert!(scenes[1].handmade.is_empty());
    }
}
