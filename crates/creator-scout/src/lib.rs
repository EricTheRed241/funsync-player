//! `npm run creators:scout`: the Creators site's data. Independent creators
//! (OnlyFans / Fansly / camgirls / cosplayers / influencers) behind new
//! EroScripts script posts (`real` + `non-vr`) and FapTap videos, named by a
//! judge (an omp run) and kept when an Instagram or Twitch account of theirs
//! exists; the people the user likes (favourite artists, the pornstar list,
//! creators swiped yes) looked up on Instagram and Twitch; their posts
//! (scored like the EroScripts deck), Instagram pictures, and the fetch of
//! the posts swiped in. Writes `<userData>/creators.json`,
//! `creator-pictures/`, `creator-scout-state.json` and
//! `instagram-session.json`; reads `creator-swipes.json` (the site's) and
//! config.json, which it never writes. The forum is read-only and paced.

pub mod cli;
pub mod discover;
pub mod eroscripts;
pub mod faptap;
pub mod fetch;
pub mod follow;
pub mod html;
pub mod instagram;
pub mod judge;
pub mod library;
pub mod people;
pub mod post_images;
pub mod profiles;
pub mod runtimes;
pub mod seed;
pub mod state;
pub mod store;
pub mod verify;

use cli::Mode;
use discover::{Discover, combine};
use library::{LibraryIds, Scoring};
use serde_json::Value;
use slr_scout::http::{Curl, Fake, Http};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use video_tags::host::{self, LockError};

pub const LOCK_NAME: &str = "creator-scout.lock";
/// Point the CLI at canned responses instead of the network (tests): a JSON
/// file `{"GET <url>": {"status": 200, "headers": [["name", "value"]],
/// "body": <JSON or string>}}` (slr-scout's fake); unlisted requests get a
/// 404. Pacing is off with it.
pub const FAKE_ENV: &str = "CREATOR_SCOUT_FAKE_HTTP";

/// Run the CLI; returns the exit code.
pub fn run(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match run_inner(args, out, err) {
        Ok(code) => code,
        Err(msg) => {
            let _ = writeln!(err, "creator-scout: {msg}");
            1
        }
    }
}

fn io(e: std::io::Error) -> String {
    format!("output: {e}")
}

/// What every networked mode needs.
struct Env {
    user_data: PathBuf,
    config: Value,
    user: slr_scout::library::UserConfig,
    http: Box<dyn Http>,
    fake: bool,
}

impl Env {
    fn library(&self) -> LibraryIds {
        LibraryIds::scan(&self.user_data, &self.user.sources)
    }

    fn scoring(&self) -> Result<Scoring, String> {
        Ok(Scoring::new(&slr_scout::prefs::load()?, &self.user))
    }
}

/// The client of the modes that fetch nothing (`--import-seed`).
struct Offline;

impl Http for Offline {
    fn send(&self, req: &slr_scout::http::Request) -> Result<slr_scout::http::Response, String> {
        Err(format!("{}: this mode doesn't use the network", req.url))
    }
}

fn run_inner(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> Result<i32, String> {
    let opts = cli::parse(args)?;
    if opts.help {
        write!(out, "{}", cli::USAGE).map_err(io)?;
        return Ok(0);
    }
    let mode = opts.mode.clone().ok_or("no mode")?;
    let user_data = match opts.user_data.clone() {
        Some(dir) => dir,
        None => {
            host::default_user_data().ok_or("can't find the userData folder; pass --user-data")?
        }
    };
    if !user_data.is_dir() {
        return Err(format!("{} is not a folder", user_data.display()));
    }
    if mode == Mode::Status {
        status(&user_data, out).map_err(io)?;
        return Ok(0);
    }
    let _lock = match host::take_lock(&user_data.join(LOCK_NAME)) {
        Ok(lock) => lock,
        Err(LockError::Held(h)) => {
            writeln!(
                err,
                "creator-scout: another run ({h}) holds {LOCK_NAME}; try later."
            )
            .map_err(io)?;
            return Ok(2);
        }
        Err(LockError::Io(m)) => return Err(m),
    };
    if mode == Mode::Login {
        let path = slr_scout::login::credentials_path_for(
            opts.credentials.as_deref(),
            std::env::var(instagram::CREDENTIALS_ENV).ok(),
            instagram::CREDENTIALS_FILE,
        );
        // A missing default file means the user signs in by hand.
        let creds = if opts.credentials.is_none() && !path.is_file() {
            None
        } else {
            Some(slr_scout::login::read_credentials(&path)?)
        };
        let session = instagram::login(&user_data, creds.as_ref(), err)?;
        instagram::write_session(&user_data, &session)?;
        writeln!(
            out,
            "Instagram session saved to {} ({} cookie(s)).",
            user_data.join(instagram::SESSION_FILE).display(),
            session.cookies.len()
        )
        .map_err(io)?;
        return Ok(0);
    }

    let config = if user_data.join("config.json").is_file() {
        video_tags::config::read_json(&user_data)?
    } else {
        writeln!(
            err,
            "creator-scout: no config.json in {}: no library, no EroScripts session.",
            user_data.display()
        )
        .map_err(io)?;
        Value::Null
    };
    let (http, fake): (Box<dyn Http>, bool) = if matches!(mode, Mode::ImportSeed(_)) {
        (Box::new(Offline), false)
    } else {
        match std::env::var_os(FAKE_ENV).filter(|v| !v.is_empty()) {
            Some(path) => (Box::new(Fake::load(Path::new(&path))?), true),
            None => (Box::new(Curl), false),
        }
    };
    let env = Env {
        user: slr_scout::library::from_json(&config),
        config,
        user_data,
        http,
        fake,
    };
    let mut file = store::read(&env.user_data)?.unwrap_or_else(store::CreatorsFile::empty);
    let mut st = state::read(&env.user_data)?;

    let code = match mode {
        Mode::ImportSeed(path) => import_seed(&env, &path, &mut file, &mut st, out)?,
        Mode::Discover => step("discover", &env, &mut st, err, |st| {
            run_discover(&env, &opts, &mut file, st, out)
        })?,
        Mode::Pictures(n) => step("pictures", &env, &mut st, err, |st| {
            run_pictures(&env, n, &mut file, st, out)
        })?,
        Mode::PostImages(n) => step("post-images", &env, &mut st, err, |st| {
            run_post_images(&env, n, &mut file, st, out)
        })?,
        Mode::Runtimes(n) => step("runtimes", &env, &mut st, err, |st| {
            run_runtimes(&env, n, &mut file, st, out)
        })?,
        Mode::Profiles(n) => step("profiles", &env, &mut st, err, |st| {
            run_profiles(&env, &opts, n, &mut file, st, out)
        })?,
        Mode::Follow(n) => step("follow", &env, &mut st, err, |st| {
            run_follow(&env, n, &mut file, st, out)
        })?,
        Mode::Fetch => step("fetch", &env, &mut st, err, |st| {
            run_fetch(&env, &opts, &mut file, st, out)
        })?,
        Mode::Daily => {
            let mut codes = Vec::new();
            writeln!(out, "== discover").map_err(io)?;
            codes.push((
                "discover",
                step("discover", &env, &mut st, err, |st| {
                    run_discover(&env, &opts, &mut file, st, out)
                })?,
            ));
            writeln!(out, "== profiles").map_err(io)?;
            codes.push((
                "profiles",
                step("profiles", &env, &mut st, err, |st| {
                    run_profiles(&env, &opts, cli::DEFAULT_PROFILES, &mut file, st, out)
                })?,
            ));
            writeln!(out, "== pictures").map_err(io)?;
            codes.push((
                "pictures",
                step("pictures", &env, &mut st, err, |st| {
                    run_pictures(&env, cli::DEFAULT_PICTURES, &mut file, st, out)
                })?,
            ));
            writeln!(out, "== follow").map_err(io)?;
            codes.push((
                "follow",
                step("follow", &env, &mut st, err, |st| {
                    run_follow(&env, cli::DEFAULT_FOLLOWS, &mut file, st, out)
                })?,
            ));
            let code = codes.iter().fold(0, |a, (_, c)| combine(a, *c));
            writeln!(
                out,
                "\nRun summary: {}; {} creator(s), {} post(s); exit {code}.",
                codes
                    .iter()
                    .map(|(s, c)| format!("{s} {}", code_text(*c)))
                    .collect::<Vec<_>>()
                    .join(", "),
                file.creators.len(),
                file.post_count()
            )
            .map_err(io)?;
            code
        }
        Mode::Login | Mode::Status => 0,
    };
    Ok(code)
}

fn code_text(code: i32) -> &'static str {
    match code {
        0 => "ok",
        2 => "work left",
        3 => "not logged in",
        _ => "failed",
    }
}

/// Run one step: an error is reported and gives exit 1 (a daily run goes
/// on with the next step); the step's code goes into the state.
fn step(
    name: &str,
    env: &Env,
    st: &mut state::State,
    err: &mut dyn Write,
    f: impl FnOnce(&mut state::State) -> Result<i32, String>,
) -> Result<i32, String> {
    let code = match f(st) {
        Ok(code) => code,
        Err(msg) => {
            writeln!(err, "creator-scout: {name}: {msg}").map_err(io)?;
            1
        }
    };
    st.last_runs.insert(
        name.to_string(),
        state::StepRun {
            at: video_tags::timefmt::now_iso(),
            code,
        },
    );
    state::write(&env.user_data, st)?;
    Ok(code)
}

fn import_seed(
    env: &Env,
    path: &Path,
    file: &mut store::CreatorsFile,
    st: &mut state::State,
    out: &mut dyn Write,
) -> Result<i32, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let r = seed::import(&raw, file, &video_tags::timefmt::now_iso())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    env.library().mark(file);
    // The snapshot's scores stay (its FapTap videos have none: no tags or
    // likes to score them by); a creator's score is her scored posts' average.
    for c in file.creators.values_mut() {
        c.update_score();
        store::rebuild_pictures(c);
    }
    // The snapshot has no pictures: its EroScripts posts wait for the next
    // --discover to read their details (first image, hosts) like new ones.
    let mut queued = 0;
    for (slug, c) in &file.creators {
        for p in c
            .posts
            .iter()
            .filter(|p| p.kind == "eroscripts" && p.image.is_none())
        {
            if st
                .pending_details
                .insert(p.id.clone(), slug.clone())
                .is_none()
            {
                queued += 1;
            }
        }
    }
    store::write(&env.user_data, file)?;
    state::write(&env.user_data, st)?;
    writeln!(
        out,
        "Imported {}: {} new creator(s), {} merged, {} post(s) added; {} creator(s), {} post(s) in {}.",
        path.display(),
        r.creators_added,
        r.creators_merged,
        r.posts_added,
        file.creators.len(),
        file.post_count(),
        store::file_path(&env.user_data).display()
    )
    .map_err(io)?;
    if !r.skipped.is_empty() {
        writeln!(out, "Skipped (no usable name): {}", r.skipped.join(", ")).map_err(io)?;
    }
    if queued > 0 {
        writeln!(out, "{queued} EroScripts post(s) wait for their details (pictures, hosts) in the next --discover.").map_err(io)?;
    }
    Ok(0)
}

fn run_discover(
    env: &Env,
    opts: &cli::Options,
    file: &mut store::CreatorsFile,
    st: &mut state::State,
    out: &mut dyn Write,
) -> Result<i32, String> {
    let scoring = env.scoring()?;
    let library = env.library();
    let d = Discover {
        http: env.http.as_ref(),
        user_data: &env.user_data,
        cookie: slr_scout::forum::session_cookie(&env.config),
        judge_cmd: opts.judge_cmd.clone().unwrap_or_else(|| {
            judge::default_command(opts.max_cost.unwrap_or(judge::DEFAULT_MAX_COST))
        }),
        fake: env.fake,
        scoring: &scoring,
        library: &library,
    };
    discover::discover(&d, file, st, out)
}

fn run_pictures(
    env: &Env,
    n: usize,
    file: &mut store::CreatorsFile,
    st: &mut state::State,
    out: &mut dyn Write,
) -> Result<i32, String> {
    let swipes = state::read_swipes(&env.user_data)?;
    let r = instagram::pictures(
        env.http.as_ref(),
        verify::INSTAGRAM,
        &env.user_data,
        file,
        st,
        &swipes,
        n,
        env.fake,
        out,
    )?;
    writeln!(
        out,
        "Pictures: {} creator(s) done, {} new picture(s), {} due creator(s) left.",
        r.done, r.saved, r.left
    )
    .map_err(io)?;
    if r.code == 3 {
        writeln!(
            out,
            "Not logged in to Instagram: run `npm run creators:scout -- --login` and sign in."
        )
        .map_err(io)?;
    }
    Ok(r.code)
}

/// The people `--profiles` looks up: favourite artists (with config.json's
/// changes), the pornstar list, creators swiped yes.
fn people(
    prefs: &slr_scout::prefs::Prefs,
    user: &slr_scout::library::UserConfig,
    file: &store::CreatorsFile,
    swipes: &state::Swipes,
) -> Vec<people::Person> {
    people::people(
        &prefs.favourites,
        &user.favourite_changes,
        &prefs.pornstar_tags.pornstars,
        file,
        swipes,
    )
}

fn run_profiles(
    env: &Env,
    opts: &cli::Options,
    n: usize,
    file: &mut store::CreatorsFile,
    st: &mut state::State,
    out: &mut dyn Write,
) -> Result<i32, String> {
    let prefs = slr_scout::prefs::load()?;
    let swipes = state::read_swipes(&env.user_data)?;
    let people = people(&prefs, &env.user, file, &swipes);
    let scoring = Scoring::new(&prefs, &env.user);
    let library = env.library();
    let cookie = slr_scout::forum::session_cookie(&env.config);
    let f = profiles::Profiles {
        filing: discover::Filing {
            http: env.http.as_ref(),
            user_data: &env.user_data,
            cookie: cookie.as_deref(),
            scoring: &scoring,
        },
        judge_cmd: opts.judge_cmd.clone().unwrap_or_else(|| {
            judge::default_people_command(opts.max_cost.unwrap_or(judge::PEOPLE_MAX_COST))
        }),
        fake: env.fake,
        library: &library,
        limit: n,
    };
    profiles::run(&f, &people, file, st, &swipes, out)
}

fn run_post_images(
    env: &Env,
    n: usize,
    file: &mut store::CreatorsFile,
    st: &mut state::State,
    out: &mut dyn Write,
) -> Result<i32, String> {
    let swipes = state::read_swipes(&env.user_data)?;
    let cookie = slr_scout::forum::session_cookie(&env.config);
    let p = post_images::PostImages {
        http: env.http.as_ref(),
        user_data: &env.user_data,
        cookie: cookie.as_deref(),
        fake: env.fake,
    };
    let r = post_images::run(&p, file, st, &swipes, n, out)?;
    writeln!(
        out,
        "Post images: {} filled, {} without a picture (tried again after {} days), {} failed, {} waiting.",
        r.filled,
        r.missed,
        post_images::MISS_DAYS,
        r.failed,
        r.left + r.forum_skipped
    )
    .map_err(io)?;
    Ok(r.code)
}

fn run_runtimes(
    env: &Env,
    n: usize,
    file: &mut store::CreatorsFile,
    st: &mut state::State,
    out: &mut dyn Write,
) -> Result<i32, String> {
    let swipes = state::read_swipes(&env.user_data)?;
    let cookie = slr_scout::forum::session_cookie(&env.config);
    let r = runtimes::Runtimes {
        http: env.http.as_ref(),
        user_data: &env.user_data,
        cookie: cookie.as_deref(),
        fake: env.fake,
    };
    let run = runtimes::run(&r, file, st, &swipes, n, out)?;
    writeln!(
        out,
        "Runtimes: {} filled, {} without a readable script (tried again after {} days), {} failed, {} waiting.",
        run.filled,
        run.missed,
        runtimes::MISS_DAYS,
        run.failed,
        run.left + run.forum_skipped
    )
    .map_err(io)?;
    Ok(run.code)
}

/// `--status`'s post images and runtimes lines.
fn post_images_status(user_data: &Path) -> Result<String, String> {
    let file = store::read(user_data)?.unwrap_or_else(store::CreatorsFile::empty);
    let st = state::read(user_data)?;
    let swipes = state::read_swipes(user_data)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let due = post_images::due(&file, &st, &swipes, now);
    let of = |kind: &str| due.iter().filter(|w| w.kind == kind).count();
    Ok(format!(
        "Post images: {} post(s) waiting for their picture ({} FapTap, {} EroScripts), {} without one (tried again after {} days).\nRuntimes: {} EroScripts post(s) waiting for their runtime, {} without a readable script (tried again after {} days).",
        due.len(),
        of("faptap"),
        of("eroscripts"),
        post_images::missed(&st.post_image_misses, now),
        post_images::MISS_DAYS,
        runtimes::due(&file, &st, &swipes, now).len(),
        post_images::missed(&st.runtime_misses, now),
        runtimes::MISS_DAYS
    ))
}

fn run_follow(
    env: &Env,
    n: usize,
    file: &mut store::CreatorsFile,
    st: &mut state::State,
    out: &mut dyn Write,
) -> Result<i32, String> {
    let swipes = state::read_swipes(&env.user_data)?;
    if follow::due(file, st, &swipes).is_empty() {
        writeln!(out, "Follow: nobody to follow.").map_err(io)?;
        return Ok(0);
    }
    let login_hint =
        "Not logged in to Instagram: run `npm run creators:scout -- --login` and sign in.";
    let Some(session) =
        instagram::read_session(&env.user_data)?.filter(|s| s.cookie_header().is_some())
    else {
        writeln!(out, "No Instagram session: nobody followed.\n{login_hint}").map_err(io)?;
        return Ok(3);
    };
    let user_data = env.user_data.clone();
    let mut open = move |headless: bool| -> Result<Box<dyn follow::FollowBrowser>, String> {
        Ok(Box::new(follow::Chrome::open(
            &user_data, &session, headless,
        )?))
    };
    let r = follow::run(
        &mut open,
        verify::INSTAGRAM,
        &env.user_data,
        file,
        st,
        &swipes,
        n,
        env.fake,
        out,
    )?;
    writeln!(
        out,
        "Follow: {} followed, {} requested (private), {} failed, {} waiting.",
        r.following, r.requested, r.failed, r.left
    )
    .map_err(io)?;
    if r.code == 3 {
        writeln!(out, "{login_hint}").map_err(io)?;
    }
    Ok(r.code)
}

/// `--status`'s profiles and follows lines.
fn profiles_status(user_data: &Path) -> Result<String, String> {
    let file = store::read(user_data)?.unwrap_or_else(store::CreatorsFile::empty);
    let config = if user_data.join("config.json").is_file() {
        video_tags::config::read_json(user_data)?
    } else {
        Value::Null
    };
    let st = state::read(user_data)?;
    let swipes = state::read_swipes(user_data)?;
    let list = people(
        &slr_scout::prefs::load()?,
        &slr_scout::library::from_json(&config),
        &file,
        &swipes,
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let count = |s: profiles::Standing| {
        list.iter()
            .filter(|p| profiles::standing(&p.slug, &file, &st, &swipes, now) == s)
            .count()
    };
    let with = |s: &str| list.iter().filter(|p| p.sources.contains(&s)).count();
    let others = profiles::due(&list, &file, &st, &swipes, now)
        .iter()
        .filter(|p| p.sources.is_empty())
        .count();
    let follows = |s: &str| st.follows.values().filter(|f| f.status == s).count();
    Ok(format!(
        "Profiles: {} people ({} swiped yes, {} favourite(s), {} on the pornstar list): {} done, {} not found (looked up again after {} days), {} waiting, {} swiped no; {} other creator(s) with fewer than {} pictures waiting.\nFollow: {} followed, {} requested, {} failed, {} waiting.",
        list.len(),
        with("swiped"),
        with("favourite"),
        with("pornstar"),
        count(profiles::Standing::Done),
        count(profiles::Standing::Missing),
        profiles::MISS_DAYS,
        count(profiles::Standing::Waiting),
        count(profiles::Standing::SwipedNo),
        others,
        profiles::MIN_PICTURES,
        follows("following"),
        follows("requested"),
        follows("failed"),
        follow::due(&file, &st, &swipes)
            .iter()
            .filter(|s| !st.follows.contains_key(*s))
            .count(),
    ))
}

fn run_fetch(
    env: &Env,
    opts: &cli::Options,
    file: &mut store::CreatorsFile,
    st: &mut state::State,
    out: &mut dyn Write,
) -> Result<i32, String> {
    let swipes = state::read_swipes(&env.user_data)?;
    // Posts fetched outside this CLI count too.
    env.library().mark(file);
    let repo = opts.repo.clone().unwrap_or_else(fetch::default_repo);
    let r = fetch::fetch(
        file,
        &swipes,
        st,
        opts.npm.as_deref().unwrap_or("npm"),
        &repo,
        opts.dry_run,
        &|| env.library(),
        out,
    )?;
    if !opts.dry_run {
        store::write(&env.user_data, file)?;
        writeln!(out, "Fetch: {} fetched, {} not yet.", r.fetched, r.failed).map_err(io)?;
    }
    Ok(r.code)
}

fn age(iso: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    match instagram::parse_iso(iso) {
        Some(t) => {
            let s = (now - t).max(0);
            if s < 3600 {
                format!("{} min ago", s / 60)
            } else if s < 86_400 {
                format!("{} h ago", s / 3600)
            } else {
                format!("{} day(s) ago", s / 86_400)
            }
        }
        None => "at an unknown time".into(),
    }
}

fn status(user_data: &Path, out: &mut dyn Write) -> std::io::Result<()> {
    match store::read(user_data) {
        Ok(Some(f)) => {
            let posts: Vec<&store::Post> = f.creators.values().flat_map(|c| &c.posts).collect();
            writeln!(
                out,
                "creators.json: {} creator(s), {} post(s) ({} downloadable, {} in the library), updated {}.",
                f.creators.len(),
                posts.len(),
                posts.iter().filter(|p| p.downloadable).count(),
                posts.iter().filter(|p| p.in_library).count(),
                f.updated_at.as_deref().map_or("never".into(), age)
            )?;
            writeln!(
                out,
                "Cursors: EroScripts topic {}, FapTap video {}.",
                f.eroscripts_checked
                    .as_ref()
                    .map_or("none (first run reads 5 pages)".into(), |c| c
                        .newest_topic_id
                        .to_string()),
                f.faptap_checked
                    .as_ref()
                    .map_or("none (first run reads 5 pages)".into(), |c| c
                        .newest_video_id
                        .clone())
            )?;
        }
        Ok(None) => writeln!(
            out,
            "creators.json: none yet (run --import-seed or --discover)."
        )?,
        Err(e) => writeln!(out, "creators.json: {e}")?,
    }
    match instagram::read_session(user_data) {
        Ok(Some(s)) if s.cookie_header().is_some() => writeln!(
            out,
            "Instagram session: saved {}.",
            s.saved_at
                .as_deref()
                .map_or("at an unknown time".into(), age)
        )?,
        Ok(_) => writeln!(out, "Instagram session: none (run --login).")?,
        Err(e) => writeln!(out, "Instagram session: {e}")?,
    }
    match profiles_status(user_data) {
        Ok(lines) => writeln!(out, "{lines}")?,
        Err(e) => writeln!(out, "Profiles: {e}")?,
    }
    match post_images_status(user_data) {
        Ok(line) => writeln!(out, "{line}")?,
        Err(e) => writeln!(out, "Post images: {e}")?,
    }
    match state::read(user_data) {
        Ok(st) => {
            for (name, r) in &st.last_runs {
                writeln!(out, "Last {name}: {} (exit {}).", age(&r.at), r.code)?;
            }
            let waiting = st
                .fetch
                .values()
                .filter(|r| r.done_at.is_none() && r.tries < state::MAX_TRIES)
                .count();
            let given_up = st
                .fetch
                .values()
                .filter(|r| r.done_at.is_none() && r.tries >= state::MAX_TRIES)
                .count();
            writeln!(
                out,
                "Fetch: {} done, {waiting} to retry, {given_up} given up; {} post(s) waiting for details.",
                st.fetch.values().filter(|r| r.done_at.is_some()).count(),
                st.pending_details.len()
            )?;
        }
        Err(e) => writeln!(out, "State: {e}")?,
    }
    if let Some(holder) = std::fs::read_to_string(user_data.join(LOCK_NAME))
        .ok()
        .and_then(|s| host::LockHolder::parse(&s))
        .filter(|h| h.holds(host::pid_alive))
    {
        writeln!(out, "Running now ({holder}).")?;
    }
    Ok(())
}

#[cfg(test)]
pub mod testing {
    //! An in-memory [`Http`] for unit tests.
    use slr_scout::http::{Http, Request, Response};
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Default)]
    pub struct Canned {
        pub responses: HashMap<String, Response>,
        pub seen: RefCell<Vec<Request>>,
    }

    impl Canned {
        pub fn on(&mut self, method: &str, url: &str, status: u16, body: &str) {
            self.responses.insert(
                format!("{method} {url}"),
                Response {
                    status,
                    headers: Vec::new(),
                    body: body.to_string(),
                },
            );
        }
    }

    impl Http for Canned {
        fn send(&self, req: &Request) -> Result<Response, String> {
            self.seen.borrow_mut().push(req.clone());
            self.responses
                .get(&format!("{} {}", req.method, req.url))
                .cloned()
                .ok_or_else(|| format!("no canned response for {} {}", req.method, req.url))
        }
    }
}
