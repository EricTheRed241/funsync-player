//! `npm run video:tags`: keep `<userData>/video-tags.json`, the per-video
//! tags (e.g. vision-classified `vr`, `pov`, `creampie`, `riding`) the app
//! merges into `library.scriptTags` on every library scan
//! (renderer/js/video-tags.js). The library scan rebuilds scriptTags from
//! the scene sidecars and EroScripts links, so this file is the durable
//! store. The CLI never writes config.json or the library.

pub mod cli;
pub mod config;
pub mod plan;
pub mod proposals;
pub mod rescan;
pub mod spelling;
pub mod store;
pub mod timefmt;

use plan::{Change, Mode, Outcome, Valid};
use spelling::Resolver;
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Run the CLI; returns the exit code.
pub fn run(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match run_inner(args, out, err) {
        Ok(code) => code,
        Err(msg) => {
            let _ = writeln!(err, "video-tags: {msg}");
            1
        }
    }
}

/// Electron's userData folder for the app (`app.getPath('userData')`).
pub fn default_user_data() -> Option<PathBuf> {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home()?.join("Library/Application Support")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| Some(home()?.join(".config")))?
    };
    Some(base.join("funsync-player"))
}

fn run_inner(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> Result<i32, String> {
    let opts = cli::parse(args)?;
    if opts.help {
        write!(out, "{}", cli::USAGE).map_err(io)?;
        return Ok(0);
    }
    let user_data = match opts.user_data.clone() {
        Some(dir) => dir,
        None => default_user_data().ok_or("can't find the userData folder; pass --user-data")?,
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

    let mut next = current.clone().unwrap_or_else(store::TagFile::empty);
    let now = timefmt::now_iso();
    let mut outcomes: Vec<Outcome> = Vec::new();
    let mut notes: HashMap<String, Vec<String>> = HashMap::new();
    let mut app_config = None;

    if let Some(proposals_path) = &opts.proposals {
        let raw = fs::read_to_string(proposals_path)
            .map_err(|e| format!("{}: {e}", proposals_path.display()))?;
        let proposals =
            proposals::parse(&raw).map_err(|e| format!("{}: {e}", proposals_path.display()))?;
        let cfg = config::read(&user_data)?;
        let (valid, invalid) = validate(&proposals, &cfg.sources, &mut notes);
        app_config = Some(cfg);
        if !invalid.is_empty() {
            for (path, reason) in &invalid {
                writeln!(err, "  invalid    {path}: {reason}").map_err(io)?;
            }
            writeln!(
                err,
                "{} invalid of {} proposed; nothing written.",
                invalid.len(),
                proposals.len()
            )
            .map_err(io)?;
            return Ok(1);
        }
        let mode = if opts.replace {
            Mode::Replace
        } else {
            Mode::Union
        };
        writeln!(
            out,
            "Proposals: {} video(s) from {} ({})",
            valid.len(),
            proposals_path.display(),
            if mode == Mode::Replace {
                "replace"
            } else {
                "add to current tags"
            }
        )
        .map_err(io)?;
        outcomes.extend(plan::apply_proposals(&mut next, &valid, mode, &now));
    }

    if !opts.remove.is_empty() {
        match plan::remove_paths(&mut next, &opts.remove) {
            Ok(removed) => outcomes.extend(removed),
            Err(missing) => {
                for path in &missing {
                    writeln!(err, "  invalid    {path}: not in {}", file_path.display())
                        .map_err(io)?;
                }
                writeln!(
                    err,
                    "{} path(s) to remove not found; nothing written.",
                    missing.len()
                )
                .map_err(io)?;
                return Ok(1);
            }
        }
    }

    report(&outcomes, &notes, next.videos.len(), out).map_err(io)?;
    if !outcomes.iter().any(Outcome::is_change) {
        writeln!(out, "Nothing to change.").map_err(io)?;
        return Ok(0);
    }
    if !opts.apply {
        writeln!(
            out,
            "Dry run: nothing written. Re-run with --apply to write {}.",
            file_path.display()
        )
        .map_err(io)?;
        return Ok(0);
    }

    next.version = store::VERSION;
    next.updated_at = Some(now);
    let backed_up = store::write_atomic(&user_data, &next)?;
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

    if opts.no_rescan {
        writeln!(
            out,
            "No rescan requested (--no-rescan); the app applies the file on its next library scan."
        )
        .map_err(io)?;
    } else {
        let port = match app_config {
            Some(cfg) => cfg.backend_port,
            None => config::read(&user_data)
                .map(|c| c.backend_port)
                .unwrap_or(config::DEFAULT_BACKEND_PORT),
        };
        match rescan::request_rescan(port, rescan::TIMEOUT) {
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

/// Check every proposal; returns the valid ones (on-disk path, normalised
/// tags) and `(path, reason)` for the rest. Spelling changes go to `notes`.
pub fn validate(
    proposals: &[proposals::Proposal],
    sources: &[PathBuf],
    notes: &mut HashMap<String, Vec<String>>,
) -> (Vec<Valid>, Vec<(String, String)>) {
    let mut resolver = Resolver::new();
    let roots: Vec<PathBuf> = sources
        .iter()
        .filter_map(|s| resolver.resolve(s).ok().map(|r| r.path))
        .collect();
    let mut valid: Vec<Valid> = Vec::new();
    let mut invalid = Vec::new();
    let mut seen: HashMap<String, String> = HashMap::new();
    for p in proposals {
        let mut reasons = Vec::new();
        if let Some(problem) = &p.problem {
            reasons.push(problem.clone());
        }
        let tags = match plan::normalize_tags(&p.tags) {
            Ok(tags) => tags,
            Err(e) => {
                reasons.push(e);
                Vec::new()
            }
        };
        let resolved = match check_path(&mut resolver, &p.path, &roots) {
            Ok(resolved) => Some(resolved),
            Err(e) => {
                reasons.push(e);
                None
            }
        };
        if let Some((path, respelled)) = &resolved {
            if let Some(first) = seen.get(path) {
                reasons.push(format!("same video as \"{first}\""));
            } else {
                seen.insert(path.clone(), p.path.clone());
                if !respelled.is_empty() {
                    notes.insert(
                        path.clone(),
                        respelled
                            .iter()
                            .map(spelling::Respelled::describe)
                            .collect(),
                    );
                }
            }
        }
        match resolved {
            Some((path, _)) if reasons.is_empty() => valid.push(Valid {
                path,
                tags,
                by: p.by.clone(),
            }),
            _ => invalid.push((p.path.clone(), reasons.join("; "))),
        }
    }
    (valid, invalid)
}

/// The on-disk spelling of a proposed video path, or why it can't be tagged.
fn check_path(
    resolver: &mut Resolver,
    proposed: &str,
    roots: &[PathBuf],
) -> Result<(String, Vec<spelling::Respelled>), String> {
    let path = Path::new(proposed);
    if !path.is_absolute() {
        return Err("not an absolute path".into());
    }
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("._"))
    {
        return Err("AppleDouble '._' file, not a video".into());
    }
    let resolved = resolver
        .resolve(path)
        .map_err(|e| format!("doesn't exist ({e})"))?;
    let meta = fs::metadata(&resolved.path).map_err(|e| format!("can't read it ({e})"))?;
    if !meta.is_file() {
        return Err("not a regular file".into());
    }
    if !roots.iter().any(|root| resolved.path.starts_with(root)) {
        return Err("not inside a library source (settings.library.sources in config.json)".into());
    }
    let stored = resolved
        .path
        .to_str()
        .ok_or("the on-disk name isn't valid UTF-8")?
        .to_string();
    Ok((stored, resolved.respelled))
}

fn report(
    outcomes: &[Outcome],
    notes: &HashMap<String, Vec<String>>,
    total: usize,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    let (mut new, mut changed, mut unchanged, mut removed) = (0, 0, 0, 0);
    for o in outcomes {
        let line = match &o.change {
            Change::New { tags } => {
                new += 1;
                format!("  new        {}: {}", o.path, marks("+", tags))
            }
            Change::Changed {
                added,
                dropped,
                kept,
            } => {
                changed += 1;
                let mut parts = Vec::new();
                if !added.is_empty() {
                    parts.push(marks("+", added));
                }
                if !dropped.is_empty() {
                    parts.push(marks("-", dropped));
                }
                if !kept.is_empty() {
                    parts.push(format!("(kept {})", kept.join(", ")));
                }
                format!("  changed    {}: {}", o.path, parts.join(" "))
            }
            Change::Unchanged { tags } => {
                unchanged += 1;
                let tags = if tags.is_empty() {
                    "(no tags)".to_string()
                } else {
                    tags.join(", ")
                };
                format!("  unchanged  {}: {tags}", o.path)
            }
            Change::Removed { tags } => {
                removed += 1;
                format!("  removed    {}: {}", o.path, marks("-", tags))
            }
        };
        writeln!(out, "{line}")?;
        for note in notes.get(&o.path).into_iter().flatten() {
            writeln!(out, "             spelling: {note}")?;
        }
    }
    writeln!(
        out,
        "Totals: {new} new, {changed} changed, {unchanged} unchanged, {removed} removed; {total} video(s) in the file after this."
    )
}

fn marks(sign: &str, tags: &[String]) -> String {
    tags.iter()
        .map(|t| format!("{sign} {t}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn list(
    file_path: &Path,
    file: Option<&store::TagFile>,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    let Some(file) = file else {
        return writeln!(
            out,
            "No video tags yet ({} doesn't exist).",
            file_path.display()
        );
    };
    writeln!(
        out,
        "{}: {} video(s), updated {}",
        file_path.display(),
        file.videos.len(),
        file.updated_at.as_deref().unwrap_or("never")
    )?;
    for (path, entry) in &file.videos {
        let by = entry
            .by
            .as_deref()
            .map(|b| format!(" by {b}"))
            .unwrap_or_default();
        writeln!(
            out,
            "  {path}\n      {}  (added {}{by})",
            entry.tags.join(", "),
            entry.added_at
        )?;
    }
    Ok(())
}
