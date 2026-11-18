//! The judge: an omp run (default `omp -p …`, `--judge-cmd` to replace it)
//! that names the on-screen creator of each new title and, for new
//! creators, finds Instagram / Twitch handle candidates by web search
//! (`--discover`), or finds them for the people the user likes
//! (`--favourites`); and the checks its results files must pass.

use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const DEFAULT_MAX_COST: f64 = 5.0;
pub const WORK_DIR: &str = "creator-scout-work";
pub const BATCH_FILE: &str = "titles.json";
pub const RESULTS_FILE: &str = "results.json";
/// Handle candidates per account kind, at most.
pub const MAX_HANDLES: usize = 5;
const MAX_NAME: usize = 60;

/// The default judge command; the prompt is appended as the last argument.
/// `--wait-async`: print mode would otherwise exit before background work
/// ends.
pub fn default_command(max_cost: f64) -> String {
    format!(
        "omp -p --wait-async --tools read,write,web_search --auto-approve --max-cost {max_cost}"
    )
}

/// A title to judge.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Item {
    /// `eroscripts:<id>` / `faptap:<id>`.
    pub key: String,
    pub title: String,
    pub tags: Vec<String>,
    pub performers: Vec<String>,
    /// Who posted it (the scripter, never the creator).
    pub uploader: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Known {
    pub name: String,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Batch {
    pub titles: Vec<Item>,
    /// Creators already in creators.json: reuse their spelling.
    pub known: Vec<Known>,
    /// Names searched recently without an account: don't search again.
    pub skip: Vec<String>,
}

/// A person or creator to find the profiles and pictures of (`--profiles`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PersonItem {
    pub name: String,
    pub aliases: Vec<String>,
    /// `swiped` / `favourite` / `pornstar`; empty for a creator only
    /// discovered.
    pub sources: Vec<String>,
    /// Her funscript posts' titles (EroScripts / FapTap), to tell her from
    /// namesakes.
    pub posts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PeopleBatch {
    pub people: Vec<PersonItem>,
}

/// Write `batch` to `work/<name>`; returns (batch file, results file
/// `work/<results>`). A stale results file is removed first.
fn write_files(
    work: &Path,
    name: &str,
    results: &str,
    batch: &impl Serialize,
) -> Result<(PathBuf, PathBuf), String> {
    fs::create_dir_all(work).map_err(|e| format!("{}: {e}", work.display()))?;
    let batch_path = work.join(name);
    let results = work.join(results);
    let _ = fs::remove_file(&results);
    let json = serde_json::to_string_pretty(batch).map_err(|e| e.to_string())?;
    fs::write(&batch_path, json).map_err(|e| format!("{}: {e}", batch_path.display()))?;
    Ok((batch_path, results))
}

/// Write the titles batch; returns (batch file, results file).
pub fn write_batch(work: &Path, batch: &Batch) -> Result<(PathBuf, PathBuf), String> {
    write_files(work, BATCH_FILE, RESULTS_FILE, batch)
}

/// People per judge batch (one subagent each).
pub const PEOPLE_BATCH: usize = 8;
/// The default spending cap of `--profiles`' judge (it searches many sites
/// for every person).
pub const PEOPLE_MAX_COST: f64 = 10.0;

/// The default judge command of `--profiles`: the main agent spawns one
/// subagent per batch (`task`), each searching the web.
pub fn default_people_command(max_cost: f64) -> String {
    format!(
        "omp -p --wait-async --tools read,write,task,web_search --auto-approve --max-cost {max_cost}"
    )
}

/// A people batch file, its results file and the names in it.
#[derive(Debug, Clone, PartialEq)]
pub struct PeopleFiles {
    pub batch: PathBuf,
    pub results: PathBuf,
    pub names: Vec<String>,
}

/// Write the people in batches of [`PEOPLE_BATCH`] (`people-<n>.json`,
/// results `people-results-<n>.json`); earlier batch and results files are
/// removed first.
pub fn write_people(work: &Path, people: &[PersonItem]) -> Result<Vec<PeopleFiles>, String> {
    fs::create_dir_all(work).map_err(|e| format!("{}: {e}", work.display()))?;
    for e in fs::read_dir(work).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.starts_with("people") && n.ends_with(".json") {
            let _ = fs::remove_file(e.path());
        }
    }
    people
        .chunks(PEOPLE_BATCH)
        .enumerate()
        .map(|(i, chunk)| {
            let (batch, results) = write_files(
                work,
                &format!("people-{}.json", i + 1),
                &format!("people-results-{}.json", i + 1),
                &PeopleBatch {
                    people: chunk.to_vec(),
                },
            )?;
            Ok(PeopleFiles {
                batch,
                results,
                names: chunk.iter().map(|p| p.name.clone()).collect(),
            })
        })
        .collect()
}

/// What the judge looks for, the same for new creators and for people.
const FIND: &str = "her Instagram account, her Twitch channel and her public profile pages on OnlyFans, Fansly, \
Pornhub (model / pornstar / channel page), X (Twitter), Reddit (u/…), ManyVids, Linktree or her own link page, and \
pornstar databases (IAFD, Babepedia, Boobpedia, FreeOnes). Only her own accounts and pages that you saw in search \
results, not fan pages, reposts or namesakes; never guessed ones. List up to 3 Instagram and up to 3 Twitch handle \
candidates, most likely first, [] when none (handles without @ and without URL); up to 10 profile page URLs (full \
https URLs); and, when you see them, up to 10 free public pictures of her as {\"url\": \"<image URL>\", \"page\": \
\"<the profile page URL above it is on>\"}";

/// The main judge's instructions: one subagent per people batch.
pub fn people_prompt(batches: &[PeopleFiles]) -> String {
    let list: String = batches
        .iter()
        .map(|b| {
            format!(
                "- {} → {} ({} people)\n",
                b.batch.display(),
                b.results.display(),
                b.names.len()
            )
        })
        .collect();
    format!(
        "You find the public profiles of people a user likes: adult performers, OnlyFans / Fansly creators, \
camgirls, cosplayers, and adult artists or animators (3D, hentai, PMV makers) and studios. Each batch file below is \
JSON: {{\"people\": [{{\"name\", \"aliases\", \"sources\", \"posts\"}}]}} (sources: favourite = a favourite artist, \
pornstar = on the user's pornstar list, swiped = liked on the user's Creators site; posts: titles of funscripts made \
for her videos, to tell her from namesakes).

Batches (batch file → results file):
{list}
Spawn one `task` subagent per batch, all in a single parallel call. Give each subagent its batch file and its \
results file and these instructions, word for word:
1. Read the batch file with the read tool.
2. For every person, search the web (web_search) for {FIND}. Check every one of those sites for every person: \
search each site by her name (and aliases), e.g. \"<name> instagram\", \"<name> onlyfans\", \"<name> iafd\", \
\"<name> babepedia\", \"<name> pornhub\", \"<name> twitch\"; don't stop at the first find.
3. Write the results file with the write tool, and nothing else in it: \
{{\"people\": [{{\"name\": \"<name as in the batch>\", \"instagram\": [\"handle\"], \"twitch\": [\"login\"], \
\"profiles\": [\"https://…\"], \"images\": [{{\"url\": \"https://…\", \"page\": \"https://…\"}}]}}]}} with exactly \
one entry per batch person.

Don't change any other file. The subagents' results come back to you as they finish; when every subagent has \
reported and every results file exists, reply with one line: done. If a subagent failed, run that batch again."
    )
}

/// The people results: handle candidates by batch name (people the file
/// leaves out aren't in it).
pub fn read_people_results(path: &Path, names: &[String]) -> Result<Vec<Handles>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err("no results file".into());
        }
        Err(e) => return Err(e.to_string()),
    };
    validate_people(&raw, names)
}

/// Check a people results file against the batch's names; any problem
/// rejects the whole file.
pub fn validate_people(raw: &str, names: &[String]) -> Result<Vec<Handles>, String> {
    let v: Value = serde_json::from_str(raw).map_err(|e| format!("not JSON ({e})"))?;
    let top = v.as_object().ok_or("not a JSON object")?;
    check_keys(top, &["people"], "results")?;
    let list = top
        .get("people")
        .and_then(Value::as_array)
        .ok_or("no \"people\" list")?;
    let mut out: Vec<Handles> = Vec::new();
    for (i, e) in list.iter().enumerate() {
        let what = format!("person {}", i + 1);
        let o = e
            .as_object()
            .ok_or_else(|| format!("{what} is not an object"))?;
        check_keys(
            o,
            &[
                "name",
                "instagram",
                "twitch",
                "profiles",
                "images",
                "reason",
            ],
            &what,
        )?;
        let n = name(o.get("name").unwrap_or(&Value::Null), &what)?;
        let slug = crate::store::slug(&n);
        let Some(batch_name) = names.iter().find(|b| crate::store::slug(b) == slug) else {
            return Err(format!("{what}: \"{n}\" is not in the batch"));
        };
        if out.iter().any(|h| crate::store::slug(&h.name) == slug) {
            return Err(format!("{what}: \"{n}\" appears twice"));
        }
        out.push(found(o, batch_name.clone(), &what)?);
    }
    Ok(out)
}

pub fn prompt(batch: &Path, results: &Path) -> String {
    format!(
        "You sort new adult-video titles by the independent creator who appears on screen. \
Read {batch} with the read tool. It is JSON: {{\"titles\": [{{\"key\", \"title\", \"tags\", \"performers\", \"uploader\"}}], \
\"known\": [{{\"name\", \"aliases\"}}], \"skip\": [\"name\"]}}. The titles are new EroScripts script posts and FapTap \
videos of real people (not VR).

1. For every title, name the on-screen independent creator: an OnlyFans / Fansly creator, camgirl, cosplayer or \
influencer who makes and sells her own content. Answer null for studio pornstars (Brazzers, Vixen, Blacked, Tushy, \
Bangbros and the like, or anyone known mainly from studio scenes), JAV actresses, PMVs / compilations / edits of \
several people, celebrities, fictional or game characters, and when the title only names the scripter. \"uploader\" \
is the scripter, never the creator. When the creator is one of \"known\" (by name or alias), write the known name \
exactly. Search the web when a name is unfamiliar and you can't tell otherwise.
2. For every creator you named who is not in \"known\" and not in \"skip\": search the web (web_search) for {FIND}.

Write {results} with the write tool, and nothing else in it:
{{\"titles\": [{{\"key\": \"<key>\", \"creator\": \"<name>\" or null}}], \
\"creators\": [{{\"name\": \"<name as in titles>\", \"instagram\": [\"handle\"], \"twitch\": [\"login\"], \
\"profiles\": [\"https://…\"], \"images\": [{{\"url\": \"https://…\", \"page\": \"https://…\"}}]}}]}}
\"titles\" has exactly one entry per batch title with its key; \"creators\" lists only the new creators of step 2. \
Don't change any other file. When the results file is written, reply with one line: done.",
        batch = batch.display(),
        results = results.display()
    )
}

/// Run the judge (`sh -c '<cmd> "$1"' creator-scout-judge <prompt>`) in the
/// work folder; its output goes to this process's stderr.
pub fn run(command: &str, prompt: &str, work: &Path) -> Result<(), String> {
    use std::os::fd::AsFd;
    let to_stderr = std::io::stderr()
        .as_fd()
        .try_clone_to_owned()
        .map_err(|e| format!("can't start the judge ({e})"))?;
    let status = Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("{command} \"$1\""))
        .arg("creator-scout-judge")
        .arg(prompt)
        .current_dir(work)
        .stdin(Stdio::null())
        .stdout(Stdio::from(to_stderr))
        .status()
        .map_err(|e| format!("can't start the judge ({e})"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("the judge exited with {status}"))
    }
}

/// A picture the judge found, on a page it says is hers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Image {
    pub url: String,
    pub page: String,
}

/// What the judge found for a creator: handle candidates, profile pages,
/// pictures.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Handles {
    pub name: String,
    pub instagram: Vec<String>,
    pub twitch: Vec<String>,
    pub profiles: Vec<String>,
    pub images: Vec<Image>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Results {
    /// Batch key → creator name (None: no independent creator), in batch
    /// order.
    pub titles: Vec<(String, Option<String>)>,
    pub creators: Vec<Handles>,
}

pub fn read_results(path: &Path, keys: &[String]) -> Result<Results, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err("no results file".into());
        }
        Err(e) => return Err(e.to_string()),
    };
    validate(&raw, keys)
}

fn check_keys(
    o: &serde_json::Map<String, Value>,
    allowed: &[&str],
    what: &str,
) -> Result<(), String> {
    match o.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(format!("{what}: unknown field \"{k}\"")),
        None => Ok(()),
    }
}

fn name(v: &Value, what: &str) -> Result<String, String> {
    let s = v
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{what}: name missing"))?;
    if s.chars().count() > MAX_NAME || s.chars().any(char::is_control) {
        return Err(format!("{what}: \"{s}\" is not a name"));
    }
    if crate::store::slug(s).is_empty() {
        return Err(format!("{what}: \"{s}\" has no letters or digits"));
    }
    Ok(s.to_string())
}

/// Instagram: 1–30 of `[A-Za-z0-9._]`; Twitch: 1–25 of `[A-Za-z0-9_]`.
pub fn valid_handle(h: &str, twitch: bool) -> bool {
    let max = if twitch { 25 } else { 30 };
    !h.is_empty()
        && h.len() <= max
        && h.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || (!twitch && b == b'.'))
}

fn handles(v: Option<&Value>, twitch: bool, what: &str) -> Result<Vec<String>, String> {
    let kind = if twitch { "twitch" } else { "instagram" };
    let Some(v) = v else { return Ok(Vec::new()) };
    let list = v
        .as_array()
        .ok_or_else(|| format!("{what}: {kind} is not a list"))?;
    if list.len() > MAX_HANDLES {
        return Err(format!("{what}: more than {MAX_HANDLES} {kind} handles"));
    }
    let mut out: Vec<String> = Vec::new();
    for h in list {
        let h = h
            .as_str()
            .map(|s| s.trim().trim_start_matches('@'))
            .ok_or_else(|| format!("{what}: a {kind} handle is not text"))?;
        if !valid_handle(h, twitch) {
            return Err(format!("{what}: \"{h}\" is not a {kind} handle"));
        }
        if !out.iter().any(|x| x.eq_ignore_ascii_case(h)) {
            out.push(h.to_string());
        }
    }
    Ok(out)
}

/// Profile pages or pictures per creator, at most.
pub const MAX_LINKS: usize = 10;

/// A full `https://` URL: no spaces or control characters, 400 bytes at
/// most.
fn https_url(v: &Value, what: &str) -> Result<String, String> {
    let s = v
        .as_str()
        .map(str::trim)
        .ok_or_else(|| format!("{what}: a URL is not text"))?;
    let ok = s.len() <= 400
        && s.strip_prefix("https://").is_some_and(|r| !r.is_empty())
        && !s.chars().any(|c| c.is_whitespace() || c.is_control());
    if !ok {
        return Err(format!("{what}: \"{s}\" is not an https URL"));
    }
    Ok(s.to_string())
}

fn list<'a>(v: Option<&'a Value>, kind: &str, what: &str) -> Result<&'a [Value], String> {
    let list = match v {
        None | Some(Value::Null) => return Ok(&[]),
        Some(v) => v
            .as_array()
            .ok_or_else(|| format!("{what}: {kind} is not a list"))?,
    };
    if list.len() > MAX_LINKS {
        return Err(format!("{what}: more than {MAX_LINKS} {kind}"));
    }
    Ok(list)
}

/// A creator's or person's finds: handles, profile pages, pictures.
fn found(o: &serde_json::Map<String, Value>, name: String, what: &str) -> Result<Handles, String> {
    let mut profiles: Vec<String> = Vec::new();
    for v in list(o.get("profiles"), "profiles", what)? {
        let url = https_url(v, what)?;
        if !profiles.contains(&url) {
            profiles.push(url);
        }
    }
    let mut images: Vec<Image> = Vec::new();
    for v in list(o.get("images"), "images", what)? {
        let i = v
            .as_object()
            .ok_or_else(|| format!("{what}: an image is not an object"))?;
        check_keys(i, &["url", "page"], what)?;
        let image = Image {
            url: https_url(i.get("url").unwrap_or(&Value::Null), what)?,
            page: https_url(i.get("page").unwrap_or(&Value::Null), what)?,
        };
        if !images.iter().any(|x| x.url == image.url) {
            images.push(image);
        }
    }
    Ok(Handles {
        instagram: handles(o.get("instagram"), false, what)?,
        twitch: handles(o.get("twitch"), true, what)?,
        profiles,
        images,
        name,
    })
}

/// Check a results file against the batch's keys. Any problem rejects the
/// whole file (the titles stay unjudged and the cursors don't move).
pub fn validate(raw: &str, keys: &[String]) -> Result<Results, String> {
    let v: Value = serde_json::from_str(raw).map_err(|e| format!("not JSON ({e})"))?;
    let top = v.as_object().ok_or("not a JSON object")?;
    check_keys(top, &["titles", "creators"], "results")?;
    let titles = top
        .get("titles")
        .and_then(Value::as_array)
        .ok_or("no \"titles\" list")?;
    let mut by_key: HashMap<String, Option<String>> = HashMap::new();
    for (i, e) in titles.iter().enumerate() {
        let what = format!("title {}", i + 1);
        let o = e
            .as_object()
            .ok_or_else(|| format!("{what} is not an object"))?;
        check_keys(o, &["key", "creator", "reason"], &what)?;
        let key = o
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{what} has no key"))?;
        if !keys.iter().any(|k| k == key) {
            return Err(format!("key {key} is not in the batch"));
        }
        let creator = match o.get("creator") {
            None | Some(Value::Null) => None,
            Some(c) => Some(name(c, &format!("key {key}"))?),
        };
        if by_key.insert(key.to_string(), creator).is_some() {
            return Err(format!("key {key} appears twice"));
        }
    }
    let missing: Vec<&str> = keys
        .iter()
        .filter(|k| !by_key.contains_key(*k))
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        return Err(format!("no result for {}", missing.join(", ")));
    }
    let named: HashSet<String> = by_key
        .values()
        .flatten()
        .map(|n| crate::store::slug(n))
        .collect();
    let mut creators: Vec<Handles> = Vec::new();
    let none = Vec::new();
    let list = match top.get("creators") {
        None | Some(Value::Null) => &none,
        Some(Value::Array(a)) => a,
        Some(_) => return Err("\"creators\" is not a list".into()),
    };
    for (i, e) in list.iter().enumerate() {
        let what = format!("creator {}", i + 1);
        let o = e
            .as_object()
            .ok_or_else(|| format!("{what} is not an object"))?;
        check_keys(
            o,
            &["name", "instagram", "twitch", "profiles", "images"],
            &what,
        )?;
        let n = name(o.get("name").unwrap_or(&Value::Null), &what)?;
        let slug = crate::store::slug(&n);
        if !named.contains(&slug) {
            return Err(format!("{what}: \"{n}\" isn't named for any title"));
        }
        if creators.iter().any(|c| crate::store::slug(&c.name) == slug) {
            return Err(format!("{what}: \"{n}\" appears twice"));
        }
        creators.push(found(o, n, &what)?);
    }
    Ok(Results {
        titles: keys
            .iter()
            .map(|k| (k.clone(), by_key.remove(k).flatten()))
            .collect(),
        creators,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Vec<String> {
        vec!["eroscripts:1".into(), "faptap:2".into()]
    }

    #[test]
    fn accepts_a_complete_file() {
        let r = validate(
            r#"{"titles":[{"key":"faptap:2","creator":null},{"key":"eroscripts:1","creator":" Sweetie Fox ","reason":"x"}],
                "creators":[{"name":"Sweetie Fox","instagram":["@swfx_real","swfx_real","sweetie.fox"],"twitch":[]}]}"#,
            &keys(),
        )
        .unwrap();
        assert_eq!(
            r.titles,
            [
                ("eroscripts:1".to_string(), Some("Sweetie Fox".to_string())),
                ("faptap:2".to_string(), None)
            ]
        );
        assert_eq!(r.creators[0].instagram, ["swfx_real", "sweetie.fox"]);
        assert!(r.creators[0].twitch.is_empty());
        let r = validate(
            r#"{"titles":[{"key":"faptap:2"},{"key":"eroscripts:1","creator":null}]}"#,
            &keys(),
        )
        .unwrap();
        assert!(r.creators.is_empty());
    }

    #[test]
    fn rejects_anything_off() {
        let bad = [
            "[]",
            "not json",
            r#"{"titles":[{"key":"eroscripts:1","creator":null}]}"#,
            r#"{"titles":[{"key":"eroscripts:1","creator":null},{"key":"faptap:2","creator":null},{"key":"faptap:2","creator":null}]}"#,
            r#"{"titles":[{"key":"eroscripts:1","creator":null},{"key":"faptap:3","creator":null}]}"#,
            r#"{"titles":[{"key":"eroscripts:1","creator":""},{"key":"faptap:2","creator":null}]}"#,
            r#"{"titles":[{"key":"eroscripts:1","creator":7},{"key":"faptap:2","creator":null}]}"#,
            r#"{"titles":[{"key":"eroscripts:1","creator":"A","x":1},{"key":"faptap:2","creator":null}]}"#,
            r#"{"titles":[{"key":"eroscripts:1","creator":"A"},{"key":"faptap:2","creator":null}],"extra":1}"#,
            r#"{"titles":[{"key":"eroscripts:1","creator":"A"},{"key":"faptap:2","creator":null}],"creators":[{"name":"B","instagram":[]}]}"#,
            r#"{"titles":[{"key":"eroscripts:1","creator":"A"},{"key":"faptap:2","creator":null}],"creators":[{"name":"A","instagram":["https://instagram.com/a"]}]}"#,
            r#"{"titles":[{"key":"eroscripts:1","creator":"A"},{"key":"faptap:2","creator":null}],"creators":[{"name":"A","twitch":["a.b"]}]}"#,
            r#"{"titles":[{"key":"eroscripts:1","creator":"A"},{"key":"faptap:2","creator":null}],"creators":[{"name":"A"},{"name":"a"}]}"#,
            r#"{"titles":[{"key":"eroscripts:1","creator":"A"},{"key":"faptap:2","creator":null}],"creators":[{"name":"A","instagram":["a","b","c","d","e","f"]}]}"#,
        ];
        for raw in bad {
            assert!(validate(raw, &keys()).is_err(), "{raw}");
        }
    }

    #[test]
    fn handle_rules() {
        assert!(valid_handle("swfx_real", false));
        assert!(valid_handle("sweetie.fox", false));
        assert!(!valid_handle("sweetie.fox", true));
        assert!(!valid_handle("", false));
        assert!(!valid_handle(&"a".repeat(31), false));
        assert!(
            default_command(5.0)
                .ends_with("--tools read,write,web_search --auto-approve --max-cost 5")
        );
    }

    #[test]
    fn people_results_with_profiles_and_images() {
        let names = vec!["Bea Bright".to_string(), "Sweetie Fox".to_string()];
        let r = validate_people(
            r#"{"people":[{"name":"bea bright","instagram":["@beabright"],"twitch":[],
                "profiles":["https://onlyfans.com/beabright","https://onlyfans.com/beabright"],
                "images":[{"url":"https://cdn/a.jpg","page":"https://onlyfans.com/beabright"}]}]}"#,
            &names,
        )
        .unwrap();
        // Named as in the batch; one person may be left out.
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "Bea Bright");
        assert_eq!(r[0].instagram, ["beabright"]);
        assert_eq!(r[0].profiles, ["https://onlyfans.com/beabright"]);
        assert_eq!(r[0].images[0].page, "https://onlyfans.com/beabright");
        let bad = [
            r#"{"people":[{"name":"Someone Else"}]}"#,
            r#"{"people":[{"name":"Bea Bright"},{"name":"bea  bright"}]}"#,
            r#"{"people":[{"name":"Bea Bright","profiles":["http://onlyfans.com/b"]}]}"#,
            r#"{"people":[{"name":"Bea Bright","profiles":["https://a b"]}]}"#,
            r#"{"people":[{"name":"Bea Bright","images":[{"url":"https://a/b.jpg"}]}]}"#,
            r#"{"people":[{"name":"Bea Bright","images":[{"url":"https://a/b.jpg","page":"https://p","x":1}]}]}"#,
            r#"{"people":[{"name":"Bea Bright","extra":1}]}"#,
            r#"{"titles":[]}"#,
        ];
        for raw in bad {
            assert!(validate_people(raw, &names).is_err(), "{raw}");
        }
        // New creators of --discover carry them too.
        let r = validate(
            r#"{"titles":[{"key":"eroscripts:1","creator":"A"},{"key":"faptap:2","creator":null}],
                "creators":[{"name":"A","profiles":["https://fansly.com/a"],"images":[]}]}"#,
            &keys(),
        )
        .unwrap();
        assert_eq!(r.creators[0].profiles, ["https://fansly.com/a"]);
        let dir = std::env::temp_dir().join(format!("creator-scout-people-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("people-results-7.json"), "stale").unwrap();
        let item = |n: usize| PersonItem {
            name: format!("P{n}"),
            aliases: vec![],
            sources: vec!["favourite".into()],
            posts: vec![],
        };
        let people: Vec<PersonItem> = (0..19).map(item).collect();
        let batches = write_people(&dir, &people).unwrap();
        // Batches of 8: one subagent each.
        let sizes: Vec<usize> = batches.iter().map(|b| b.names.len()).collect();
        assert_eq!(sizes, [8, 8, 3]);
        assert!(batches[2].batch.ends_with("people-3.json"));
        assert!(
            !dir.join("people-results-7.json").exists(),
            "stale results removed"
        );
        let p = people_prompt(&batches);
        assert!(p.contains("people-2.json") && p.contains("people-results-3.json"));
        assert!(
            p.contains("one `task` subagent per batch")
                && p.contains("IAFD")
                && p.contains("\"images\"")
        );
        assert!(default_people_command(PEOPLE_MAX_COST).contains("task,web_search"));
        let _ = fs::remove_dir_all(&dir);
    }
}
