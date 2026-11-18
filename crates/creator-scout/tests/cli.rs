//! End to end through the binary, against a temp userData and library, with
//! the network faked (`CREATOR_SCOUT_FAKE_HTTP`: canned responses), a shell
//! script as the judge and another as npm.

use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

const FORUM: &str = "https://discuss.eroscripts.com";
const FAPTAP: &str = "https://faptap.net";
const IG: &str = "https://www.instagram.com";

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "creator-scout-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("userData")).unwrap();
        fs::create_dir_all(dir.join("lib/EroScripts")).unwrap();
        let t = TempDir(dir);
        let config = json!({"settings": {
            "eroscripts": {"session": {"cookie": "_t=SECRET-COOKIE"}},
            "library": {"sources": [{"name": "EroScripts", "path": t.lib().join("EroScripts"), "enabled": true}],
                        "tagScores": {"pixeldrain": 0}}
        }});
        fs::write(t.ud().join("config.json"), config.to_string()).unwrap();
        t
    }

    fn ud(&self) -> PathBuf {
        self.0.join("userData")
    }

    fn lib(&self) -> PathBuf {
        self.0.join("lib")
    }

    fn write(&self, name: &str, content: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, content).unwrap();
        path
    }

    fn creators(&self) -> Value {
        serde_json::from_str(&fs::read_to_string(self.ud().join("creators.json")).unwrap()).unwrap()
    }

    fn state(&self) -> Value {
        serde_json::from_str(
            &fs::read_to_string(self.ud().join("creator-scout-state.json")).unwrap(),
        )
        .unwrap()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(t: &TempDir, args: &[&str], fixtures: Option<&Path>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_creator-scout"));
    cmd.args(args)
        .arg("--user-data")
        .arg(t.ud())
        .env_remove("FUNSYNC_USER_DATA");
    // An unreadable fixture file fails a networked run before any request.
    cmd.env(
        "CREATOR_SCOUT_FAKE_HTTP",
        fixtures.map_or(
            PathBuf::from("/nonexistent/fixtures.json"),
            Path::to_path_buf,
        ),
    );
    cmd.output().unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

const SEED: &str = r#"{"version":1,"updatedAt":"2026-11-17T21:47:54Z","creators":[
  {"name":"StormiSkies","score":6.6,"instagram":{"handle":"stormiskies01","followers":"4,924","posts":"65","match":"high"},"twitch":null,
   "eroscripts":[{"id":345187,"title":"StormiSkies - Riding Dildo","free":true,"hosts":["pixeldrain"],"downloadable":true,
                  "creampie":false,"multiAxis":true,"score":6.6,"inLibrary":false}],
   "faptap":[{"id":"1938887263786110977","title":"Stormi solo","url":"https://faptap.net/v/1938887263786110977","duration":215}]},
  {"name":"Lucy Cat","score":5,"instagram":{"handle":"lucycat","followers":"1M","posts":"10","match":"high"},
   "twitch":{"handle":"lucycat","followers":3716,"lastLive":"2020-05-05T18:32:48Z","match":"high"},"eroscripts":[],"faptap":[]}
]}"#;

fn import(t: &TempDir) {
    // The seed's post is in the library (a sidecar), which the seed didn't know.
    let scene = t.lib().join("EroScripts/StormiSkies/Riding");
    fs::create_dir_all(&scene).unwrap();
    fs::write(scene.join("eroscripts.json"), r#"{"topicId":345187}"#).unwrap();
    fs::write(scene.join("._eroscripts.json"), "\0\0garbage").unwrap();
    let seed = t.write("content-creators.json", SEED);
    let o = run(t, &["--import-seed", seed.to_str().unwrap()], None);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("2 new creator(s)"), "{}", text(&o));
}

#[test]
fn help_and_bad_flags() {
    let t = TempDir::new();
    let o = run(&t, &["--help"], None);
    assert_eq!(o.status.code(), Some(0));
    assert!(text(&o).contains("Usage:"));
    let o = run(&t, &["--bogus"], None);
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn imports_the_seed_and_reports_status() {
    let t = TempDir::new();
    import(&t);
    let f = t.creators();
    assert_eq!(f["creators"].as_object().unwrap().len(), 2);
    let s = &f["creators"]["stormiskies"];
    assert_eq!(s["instagram"]["handle"], "stormiskies01");
    assert_eq!(s["posts"][0]["inLibrary"], true);
    assert_eq!(s["posts"][0]["score"], 6.6);
    // The seed's FapTap videos have no score; the creator's is her
    // scored posts' average.
    assert!(s["posts"][1]["score"].is_null());
    assert_eq!(s["score"], 6.6);
    assert!(f["creators"]["lucycat"]["score"].is_null());
    assert_eq!(f["creators"]["lucycat"]["twitch"]["followers"], 3716);
    // Importing again keeps everything and adds nothing.
    let seed = t.0.join("content-creators.json");
    let o = run(&t, &["--import-seed", seed.to_str().unwrap()], None);
    assert!(
        text(&o).contains("0 new creator(s), 2 merged, 0 post(s) added"),
        "{}",
        text(&o)
    );

    let o = run(&t, &["--status"], None);
    assert_eq!(o.status.code(), Some(0));
    let out = text(&o);
    assert!(
        out.contains("2 creator(s), 2 post(s) (2 downloadable, 1 in the library)"),
        "{out}"
    );
    assert!(out.contains("Instagram session: none"), "{out}");
}

fn ig_page(handle: &str, name: &str) -> String {
    format!(
        r#"<html><head><meta property="og:image" content="https://cdn.ig/{handle}.jpg" />
        <meta content="5M Followers, 12 Following, 225 Posts - {name} (&#064;{handle}) on Instagram: &quot;hi&quot;" name="description" /></head></html>"#
    )
}

fn discover_fixtures() -> Value {
    let topic = |id: u64, title: &str| {
        json!({"id": id, "title": title, "category_id": 14, "tags": ["real", "non-vr"], "like_count": 12,
               "created_at": "2026-11-17T09:00:00Z"})
    };
    let video = |id: &str, title: &str| {
        json!({"id": id, "name": title, "duration": 300, "rating": {"likes": 4}, "tags": [{"name": "Blowjob"}],
               "user": {"username": "scripter"}, "user_thumbnail_url": format!("user_thumbnails/{id}.webp"),
               "created_at": "2026-11-17T10:00:00Z"})
    };
    let cooked = r#"<p><a href="https://pixeldrain.com/u/abc">video</a> <a href="https://onlyfans.com/sf">OF</a></p>
        <p><img src="https://cdn.es/optimized/cover.jpeg" alt="cover"></p>"#;
    let mut detail = topic(400_000, "Sweetie Fox - Morning");
    detail["post_stream"] = json!({"posts": [{"cooked": cooked}]});
    // The seed's post, read for its details (picture, hosts) after the import.
    let mut seeded = topic(345_187, "StormiSkies - Riding Dildo");
    seeded["post_stream"] = json!({"posts": [{"cooked": r#"<p><a href="https://pixeldrain.com/u/xyz">video</a></p>
        <p><img src="https://cdn.es/optimized/stormi.jpeg" alt="stormi"></p>"#}]});
    json!({
        format!("GET {FORUM}/tags/intersection/real/non-vr.json?page=0"): {"body": {"topic_list": {"topics": [
            topic(400_001, "[Brazzers] Studio scene"), topic(400_000, "Sweetie Fox - Morning"),
            topic(345_187, "StormiSkies - Riding Dildo")]}}},
        format!("GET {FORUM}/t/400000.json"): {"body": detail},
        format!("GET {FORUM}/t/345187.json"): {"body": seeded},
        "GET https://cdn.es/optimized/stormi.jpeg": {"body": "JPEG-STORMI"},
        "GET https://cdn.es/optimized/cover.jpeg": {"body": "JPEG-ES"},
        format!("GET {FAPTAP}/api/videos?limit=42&page=1&order_by=created_at.desc"):
            {"body": {"data": {"total": 1, "videos": [video("9001", "Sweetie Fox riding")]}}},
        format!("GET {FAPTAP}/api/videos?limit=42&page=1&q=Sweetie%20Fox"):
            {"body": {"data": {"total": 3, "videos": [video("9001", "Sweetie Fox riding"),
                video("9002", "SweetieFox cosplay"), video("9003", "Another girl")]}}},
        format!("GET {FAPTAP}/api/assets/user_thumbnails/9001.webp"): {"body": "WEBP-1"},
        format!("GET {FAPTAP}/api/assets/user_thumbnails/9002.webp"): {"body": "WEBP-2"},
        format!("GET {IG}/swfx_real/"): {"body": ig_page("swfx_real", "Sweetie Fox")},
        format!("GET {IG}/someone_else/"): {"body": ig_page("someone_else", "Jane Doe")},
        "GET https://cdn.ig/swfx_real.jpg": {"body": "JPEG-IG"},
        "POST https://gql.twitch.tv/gql": {"body": {"data": {"user": {"login": "sweetiefoxxx", "displayName": "SweetieFoxxx",
            "description": "", "followers": {"totalCount": 9225}, "lastBroadcast": {"startedAt": "2020-09-11T18:17:44Z"}}}}}
    })
}

/// A judge that records its prompt and writes fixed results.
fn judge(t: &TempDir, results: &Value) -> String {
    let r = t.write("judge-results.json", &results.to_string());
    let log = t.0.join("judge-prompt.txt");
    let script = t.write(
        "judge.sh",
        &format!(
            "printf '%s' \"$1\" > '{}'\ncp '{}' results.json\n",
            log.display(),
            r.display()
        ),
    );
    format!("sh {}", script.display())
}

#[test]
fn discovers_creators_their_posts_and_pictures_then_fetches() {
    let t = TempDir::new();
    import(&t);
    let fixtures = t.write("fixtures.json", &discover_fixtures().to_string());
    let results = json!({
        "titles": [
            {"key": "eroscripts:400000", "creator": "Sweetie Fox"},
            {"key": "eroscripts:400001", "creator": null},
            {"key": "faptap:9001", "creator": "Sweetie Fox"}
        ],
        "creators": [{"name": "Sweetie Fox", "instagram": ["nope_handle", "someone_else", "swfx_real"], "twitch": ["sweetiefoxxx"]}]
    });
    let cmd = judge(&t, &results);
    let o = run(&t, &["--discover", "--judge-cmd", &cmd], Some(&fixtures));
    let out = text(&o);
    assert_eq!(o.status.code(), Some(0), "{out}");
    assert!(
        !out.contains("SECRET-COOKIE"),
        "the cookie is never printed"
    );
    let prompt = fs::read_to_string(t.0.join("judge-prompt.txt")).unwrap();
    assert!(prompt.contains("titles.json") && prompt.contains("results.json"));
    let batch: Value = serde_json::from_str(
        &fs::read_to_string(t.ud().join("creator-scout-work/titles.json")).unwrap(),
    )
    .unwrap();
    // The seed's post was filed already: not judged again.
    let keys: Vec<&str> = batch["titles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["key"].as_str().unwrap())
        .collect();
    assert_eq!(
        keys,
        ["eroscripts:400000", "eroscripts:400001", "faptap:9001"]
    );
    assert_eq!(batch["known"].as_array().unwrap().len(), 2);

    let f = t.creators();
    assert_eq!(f["eroscriptsChecked"]["newestTopicId"], 400_001);
    assert_eq!(f["faptapChecked"]["newestVideoId"], "9001");
    let c = &f["creators"]["sweetiefox"];
    assert_eq!(c["name"], "Sweetie Fox");
    // The first candidate doesn't exist, the second isn't her.
    assert_eq!(c["instagram"]["handle"], "swfx_real");
    assert_eq!(c["instagram"]["match"], "high");
    assert_eq!(c["instagram"]["followers"], "5M");
    assert_eq!(c["twitch"]["handle"], "sweetiefoxxx");
    assert_eq!(c["twitch"]["followers"], 9225);
    let posts = c["posts"].as_array().unwrap();
    let ids: Vec<&str> = posts.iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["400000", "9001", "9002"]);
    let es = &posts[0];
    assert_eq!(es["hosts"], json!(["pixeldrain", "onlyfans"]));
    assert_eq!(es["downloadable"], true);
    assert_eq!(es["image"], "es-400000-0.jpg");
    assert_eq!(es["likes"], 12);
    assert!(es["score"].is_number());
    assert!(c["score"].is_number());
    let pics = t.ud().join("creator-pictures/sweetiefox");
    assert_eq!(
        fs::read_to_string(pics.join("es-400000-0.jpg")).unwrap(),
        "JPEG-ES"
    );
    assert_eq!(
        fs::read_to_string(pics.join("ft-9002.jpg")).unwrap(),
        "WEBP-2"
    );
    assert_eq!(c["pictures"].as_array().unwrap().len(), 3);

    // Nothing new: the judge isn't asked (it would fail).
    let o = run(&t, &["--discover", "--judge-cmd", "false"], Some(&fixtures));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("0 to judge"), "{}", text(&o));

    // Pictures without a login: profile photos, exit 3.
    let o = run(&t, &["--pictures", "5"], Some(&fixtures));
    let out = text(&o);
    assert_eq!(o.status.code(), Some(3), "{out}");
    assert!(out.contains("--login"), "{out}");
    assert_eq!(
        fs::read_to_string(pics.join("ig-profile.jpg")).unwrap(),
        "JPEG-IG"
    );
    let f = t.creators();
    let pictures = f["creators"]["sweetiefox"]["pictures"].as_array().unwrap();
    assert!(
        pictures
            .iter()
            .any(|p| p["file"] == "ig-profile.jpg" && p["source"] == "instagram")
    );
    assert!(t.state()["pictures"]["sweetiefox"].is_string());

    // Fetch: the EroScripts post arrives (the fake npm writes its sidecar),
    // the FapTap one doesn't.
    fs::write(
        t.ud().join("creator-swipes.json"),
        json!({"version": 1, "creators": {"sweetiefox": {"choice": "yes"}}, "posts": {
            "eroscripts:400000": {"choice": "want", "creator": "sweetiefox"},
            "faptap:9002": {"choice": "want", "creator": "sweetiefox"},
            "faptap:9001": {"choice": "hide", "creator": "sweetiefox"},
            "eroscripts:345187": {"choice": "want", "creator": "stormiskies"}}})
        .to_string(),
    )
    .unwrap();
    let npm_log = t.0.join("npm.log");
    let npm = t.write(
        "npm.sh",
        &format!(
            "echo \"$(pwd -P) $*\" >> '{log}'\nif [ \"$3\" = eroscripts:scout ] && [ \"$5\" = --fetch ]; then\n  mkdir -p '{lib}/EroScripts/Sweetie Fox/Morning'\n  echo '{{\"topicId\": '$7'}}' > '{lib}/EroScripts/Sweetie Fox/Morning/eroscripts.json'\nfi\n",
            log = npm_log.display(),
            lib = t.lib().display()
        ),
    );
    let npm_cmd = format!("sh {}", npm.display());
    let repo = t.0.display().to_string();
    let o = run(
        &t,
        &["--fetch", "--dry-run", "--npm", &npm_cmd, "--repo", &repo],
        Some(&fixtures),
    );
    let out = text(&o);
    assert_eq!(o.status.code(), Some(0), "{out}");
    assert!(
        out.contains("would run: sh") && out.contains("eroscripts:scout -- --add 400000"),
        "{out}"
    );
    assert!(!npm_log.exists());

    let o = run(
        &t,
        &["--fetch", "--npm", &npm_cmd, "--repo", &repo],
        Some(&fixtures),
    );
    let out = text(&o);
    assert_eq!(o.status.code(), Some(2), "{out}");
    let log = fs::read_to_string(&npm_log).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 4, "{log}");
    let real_repo = fs::canonicalize(&t.0).unwrap().display().to_string();
    assert!(
        lines[0].starts_with(&real_repo)
            && lines[0].ends_with("run -s eroscripts:scout -- --add 400000")
    );
    assert!(lines[1].ends_with("run -s eroscripts:scout -- --fetch --ids 400000"));
    assert!(lines[3].ends_with("run -s faptap:scout -- --fetch --ids 9002"));
    let st = t.state();
    assert!(st["fetch"]["eroscripts:400000"]["doneAt"].is_string());
    assert_eq!(st["fetch"]["faptap:9002"]["tries"], 1);
    assert!(st["fetch"]["faptap:9002"]["doneAt"].is_null());
    assert!(
        st["fetch"].get("eroscripts:345187").is_none(),
        "already in the library"
    );
    let f = t.creators();
    assert_eq!(f["creators"]["sweetiefox"]["posts"][0]["inLibrary"], true);
}

#[test]
fn pictures_with_a_session() {
    let t = TempDir::new();
    import(&t);
    fs::write(
        t.ud().join("instagram-session.json"),
        json!({"cookies": [
            {"name": "sessionid", "value": "SID", "domain": ".instagram.com"},
            {"name": "csrftoken", "value": "CSRF", "domain": ".instagram.com"}],
            "savedAt": "2026-11-17T00:00:00.000Z"})
        .to_string(),
    )
    .unwrap();
    let old = t.ud().join("creator-pictures/lucycat");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("ig-111.jpg"), "stale").unwrap();
    let node = |id: u64, at: i64| {
        json!({"node": {"code": format!("SC{id}"), "pk": id.to_string(), "taken_at": at,
            "owner_id": {"pk": "42"}, "timeline_pinned_user_ids": [],
            "image_versions2": {"candidates": [
                {"width": 1440, "height": 1800, "url": format!("https://cdn.ig/p{id}_big.jpg")},
                {"width": 1080, "height": 1350, "url": format!("https://cdn.ig/p{id}.jpg")}]}}})
    };
    let fixtures = t.write(
        "fixtures.json",
        &json!({
            format!("GET {IG}/lucycat/"): {"body": profile_page("_9dls", "35.5K", "Lucy",
                &[node(1, 1_793_793_600), node(2, 1_793_000_000)])},
            format!("GET {IG}/stormiskies01/"): {"body": profile_page("_9dls not-logged-in", "1", "S", &[])},
            "GET https://cdn.ig/p1.jpg": {"body": "P1"},
            "GET https://cdn.ig/p2.jpg": {"body": "P2"}
        })
        .to_string(),
    );
    let o = run(&t, &["--pictures"], Some(&fixtures));
    let out = text(&o);
    // Lucy Cat comes first (slug order among the never fetched), then the
    // session turns out to be expired.
    assert_eq!(o.status.code(), Some(3), "{out}");
    assert!(out.contains("run --login"), "{out}");
    assert!(!out.contains("SID"), "the session is never printed");
    let dir = t.ud().join("creator-pictures/lucycat");
    assert_eq!(fs::read_to_string(dir.join("ig-1.jpg")).unwrap(), "P1");
    assert!(dir.join("ig-2.jpg").is_file());
    assert!(
        !dir.join("ig-111.jpg").exists(),
        "pictures no longer listed are pruned"
    );
    let f = t.creators();
    let ig = &f["creators"]["lucycat"]["instagram"];
    assert_eq!(ig["lastPostAt"], "2026-11-04T12:00:00.000Z");
    assert_eq!(ig["followers"], "35.5K");
    let pics = f["creators"]["lucycat"]["pictures"].as_array().unwrap();
    assert_eq!(pics[0]["file"], "ig-1.jpg");
    assert_eq!(pics[0]["url"], "https://www.instagram.com/p/SC1/");
    let st = t.state();
    assert!(st["pictures"]["lucycat"].is_string());
    assert!(st["pictures"].get("stormiskies").is_none());

    // Lucy Cat isn't due again for 3 days.
    let o = run(&t, &["--pictures", "1"], Some(&fixtures));
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    assert!(t.state()["pictures"].get("stormiskies").is_none());
}

#[test]
fn a_held_lock_exits_2() {
    let t = TempDir::new();
    // This test process is alive: its pid holds the lock.
    fs::write(
        t.ud().join("creator-scout.lock"),
        std::process::id().to_string(),
    )
    .unwrap();
    let o = run(&t, &["--discover"], None);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains("creator-scout.lock"));
}

/// The real preferences (embedded in the binary): their favourites, to turn
/// them off, and the first pornstar.
fn prefs() -> Value {
    serde_json::from_str(
        &fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../renderer/js/content-preferences.json"),
        )
        .unwrap(),
    )
    .unwrap()
}

fn slug(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

/// A userData for --profiles: every built-in favourite turned off, Sweetie
/// Fox and Nobody Known turned on; Lucy Cat (with Instagram) and Bea Bright
/// (Twitch only) swiped yes; the first listed pornstar a creator already.
fn favourites_setup(t: &TempDir) -> String {
    let prefs = prefs();
    let mut changes = serde_json::Map::new();
    for f in prefs["favourites"].as_array().unwrap() {
        changes.insert(f.as_str().unwrap().to_string(), json!(false));
    }
    changes.insert("Sweetie Fox".into(), json!(true));
    changes.insert("Nobody Known".into(), json!(true));
    let config = json!({"settings": {
        "eroscripts": {"session": {"cookie": "_t=SECRET-COOKIE"}},
        "library": {"sources": [{"name": "EroScripts", "path": t.lib().join("EroScripts"), "enabled": true}],
                    "favouriteArtists": changes}
    }});
    fs::write(t.ud().join("config.json"), config.to_string()).unwrap();
    let star = prefs["pornstarTags"]["pornstars"][0]["name"]
        .as_str()
        .unwrap()
        .to_string();
    let ig = |h: &str| json!({"handle": h, "followers": "1M", "posts": "10", "match": "high"});
    let mut creators = json!({
        "lucycat": {"name": "Lucy Cat", "instagram": ig("lucycat")},
        "beabright": {"name": "Bea Bright", "twitch": {"handle": "beabright", "followers": 10}},
        "oldfav": {"name": "Old Fav", "instagram": ig("oldfav"), "sources": ["favourite"]}
    });
    creators[slug(&star)] = json!({"name": star, "instagram": ig("star")});
    fs::write(
        t.ud().join("creators.json"),
        json!({"version": 1, "creators": creators}).to_string(),
    )
    .unwrap();
    fs::write(
        t.ud().join("creator-swipes.json"),
        json!({"version": 1, "creators": {"lucycat": {"choice": "yes", "at": "t"},
            "beabright": {"choice": "yes", "at": "t"}}, "posts": {}})
        .to_string(),
    )
    .unwrap();
    slug(&star)
}

fn gif(w: u16, h: u16) -> String {
    let mut b = b"GIF89a".to_vec();
    b.extend(w.to_le_bytes());
    b.extend(h.to_le_bytes());
    b.extend([0; 8]);
    String::from_utf8(b).expect("dimensions whose bytes are ASCII")
}

/// A judge for --profiles: writes fixed people-results.json.
fn people_judge(t: &TempDir, results: &Value) -> String {
    let r = t.write("people-results.json", &results.to_string());
    let script = t.write(
        "people-judge.sh",
        &format!("cp '{}' people-results.json\n", r.display()),
    );
    format!("sh {}", script.display())
}

fn profiles_fixtures() -> Value {
    let topic = |id: u64, title: &str, cat: u64, tags: Value| {
        json!({"id": id, "title": title, "category_id": cat, "tags": tags, "like_count": 7,
            "created_at": "2026-11-10T00:00:00Z"})
    };
    let mut detail = topic(500_001, "Sweetie Fox - Yoga", 14, json!(["real"]));
    detail["post_stream"] = json!({"posts": [{"cooked":
        r#"<p><a href="https://pixeldrain.com/u/sf">video</a> <a href="https://www.pornhub.com/view_video.php?viewkey=sf1">PH</a>
        <img src="https://cdn.es/optimized/yoga.jpeg" alt="y"></p>"#}]});
    let og = |title: &str, image: &str| {
        format!(
            r#"<html><head><meta property="og:title" content="{title}"><meta property="og:image" content="{image}"></head></html>"#
        )
    };
    json!({
        format!("GET {IG}/beabright/"): {"body": ig_page("beabright", "Bea Bright")},
        "GET https://onlyfans.com/beabright": {"body": og("Bea Bright", "https://cdn.web/bea-of.gif")},
        "GET https://www.iafd.com/person.rme/id=9/bea-bright.htm":
            {"body": "<html><title>IAFD</title><h1>Bea Bright</h1><meta property=\"og:image\" content=\"https://cdn.web/banner.gif\"></html>"},
        "GET https://cdn.web/bea-of.gif": {"body": gif(344, 600)},
        "GET https://cdn.web/bea1.gif": {"body": gif(600, 600)},
        "GET https://cdn.web/banner.gif": {"body": gif(1919, 300)},
        "GET https://x.com/someone": {"body": og("Jane Doe (@someone)", "https://cdn.web/jane.gif")},
        "GET https://fansly.com/SweetieFox": {"body": og("Sweetie Fox (@SweetieFox) | Fansly", "https://cdn.web/sf.gif")},
        "GET https://cdn.web/sf.gif": {"body": gif(600, 344)},
        "GET https://www.pornhub.com/view_video.php?viewkey=sf1": {"body": og("Yoga", "https://ci.phncdn.com/sf1.gif")},
        "GET https://ci.phncdn.com/sf1.gif": {"body": gif(600, 344)},
        format!("GET {FORUM}/search.json?q=Sweetie%20Fox"): {"body": {"posts": [], "topics": [
            topic(500_001, "Sweetie Fox - Yoga", 14, json!(["real"])),
            topic(500_002, "Another girl", 14, json!([])),
            topic(500_003, "Sweetie Fox AI", 14, json!(["ai-generated"])),
            topic(500_004, "Sweetie Fox chat", 2, json!([]))]}},
        format!("GET {FORUM}/t/500001.json"): {"body": detail},
        "GET https://cdn.es/optimized/yoga.jpeg": {"body": "JPEG-YOGA"},
        format!("GET {FAPTAP}/api/videos?limit=42&page=1&q=Sweetie%20Fox"):
            {"body": {"data": {"total": 2, "videos": [
                {"id": "9101", "name": "Sweetie Fox dance", "duration": 120, "rating": {"likes": 2}, "tags": [],
                 "user": {"username": "s"}, "user_thumbnail_url": "user_thumbnails/9101.webp"},
                {"id": "9102", "name": "Someone else", "duration": 120, "tags": [], "user": {"username": "s"}}]}}},
        format!("GET {FAPTAP}/api/assets/user_thumbnails/9101.webp"): {"body": "WEBP-9101"}
    })
}

#[test]
fn profiles_make_the_people_you_like_creators_with_profiles_and_pictures() {
    let t = TempDir::new();
    let star = favourites_setup(&t);
    let fixtures = t.write("fixtures.json", &profiles_fixtures().to_string());
    let judge = people_judge(
        &t,
        &json!({"people": [
            {"name": "Bea Bright", "instagram": ["beabright"], "twitch": [],
             "profiles": ["https://onlyfans.com/beabright", "https://www.iafd.com/person.rme/id=9/bea-bright.htm"],
             "images": [{"url": "https://cdn.web/bea1.gif", "page": "https://onlyfans.com/beabright/"},
                        {"url": "https://cdn.web/jane.gif", "page": "https://x.com/someone"}]},
            {"name": "Nobody Known", "instagram": [], "twitch": [], "profiles": ["https://x.com/someone"]},
            {"name": "Sweetie Fox", "profiles": ["https://fansly.com/SweetieFox"]}
        ]}),
    );
    let o = run(
        &t,
        &["--profiles", "3", "--judge-cmd", &judge],
        Some(&fixtures),
    );
    let out = text(&o);
    // The rest of the pornstar list waits for the next run.
    assert_eq!(o.status.code(), Some(2), "{out}");
    assert!(!out.contains("SECRET-COOKIE"), "{out}");
    assert!(
        out.contains("3 looked up, 2 with a confirmed profile, 1 not found, 1 new creator(s), 4 picture(s) added"),
        "{out}"
    );
    let batch: Value = serde_json::from_str(
        &fs::read_to_string(t.ud().join("creator-scout-work/people.json")).unwrap(),
    )
    .unwrap();
    let names: Vec<&str> = batch["people"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    // Lucy Cat and the listed pornstar wait for their Instagram pictures.
    assert_eq!(names, ["Bea Bright", "Nobody Known", "Sweetie Fox"]);
    let f = t.creators();
    let c = &f["creators"];
    assert_eq!(c["lucycat"]["sources"], json!(["swiped"]));
    assert_eq!(c[&star]["sources"], json!(["pornstar"]));
    // No longer a favourite: the source is gone.
    assert!(c["oldfav"]["sources"].is_null(), "{}", c["oldfav"]);
    // Swiped yes, Twitch only: her Instagram, two profiles, two pictures
    // (the IAFD banner and the judge's picture on someone else's page are
    // dropped).
    let bea = &c["beabright"];
    assert_eq!(bea["instagram"]["handle"], "beabright");
    assert_eq!(bea["twitch"]["handle"], "beabright");
    let sites: Vec<&str> = bea["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["site"].as_str().unwrap())
        .collect();
    assert_eq!(sites, ["onlyfans", "iafd"]);
    assert_eq!(bea["profiles"][0]["match"], "high");
    let sources: Vec<&str> = bea["pictures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["source"].as_str().unwrap())
        .collect();
    assert_eq!(sources, ["onlyfans", "web"]);
    // A new creator with a Fansly page only: kept, with her posts and
    // pictures (Fansly's own, the Pornhub video's poster, the posts').
    let sf = &c["sweetiefox"];
    assert_eq!(sf["sources"], json!(["favourite"]));
    assert!(sf["instagram"].is_null() && sf["twitch"].is_null());
    assert_eq!(sf["profiles"][0]["url"], "https://fansly.com/SweetieFox");
    let ids: Vec<&str> = sf["posts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["500001", "9101"]);
    assert_eq!(
        sf["posts"][0]["videoPages"],
        json!(["https://www.pornhub.com/view_video.php?viewkey=sf1"])
    );
    let pics: Vec<(&str, &str)> = sf["pictures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["source"].as_str().unwrap(), p["file"].as_str().unwrap()))
        .collect();
    assert_eq!(pics[0].0, "fansly");
    assert!(pics[0].1.starts_with("web-") && pics[0].1.ends_with(".gif"));
    assert!(pics.iter().any(|p| p.0 == "pornhub"), "{pics:?}");
    assert!(
        pics.iter().any(|p| p == &("eroscripts", "es-500001-0.jpg")),
        "{pics:?}"
    );
    let dir = t.ud().join("creator-pictures/sweetiefox");
    assert_eq!(
        fs::read_to_string(dir.join("ft-9101.jpg")).unwrap(),
        "WEBP-9101"
    );
    assert!(dir.join(pics[0].1).is_file());
    // Nobody confirmed: not a creator, noted for 30 days.
    assert!(c.get("nobodyknown").is_none());
    let st = t.state();
    assert!(st["profileMisses"]["nobodyknown"].is_string());
    assert!(st["profileMisses"].get("sweetiefox").is_none());
    assert_eq!(st["lastRuns"]["profiles"]["code"], 2);

    let o = run(&t, &["--status"], None);
    let out = text(&o);
    assert!(
        out.contains("1 not found (looked up again after 30 days)"),
        "{out}"
    );
    assert!(out.contains("Follow: 0 followed"), "{out}");

    // A judge that writes nothing: an error, nobody marked missing.
    let o = run(
        &t,
        &["--profiles", "1", "--judge-cmd", "true"],
        Some(&fixtures),
    );
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("no results file"), "{}", text(&o));
}

/// A trimmed logged-in profile page: its description meta and the timeline
/// it embeds.
fn profile_page(class: &str, followers: &str, name: &str, edges: &[Value]) -> String {
    let conn = json!({"edges": edges, "page_info": {"end_cursor": "x}y", "has_next_page": false}});
    format!(
        r#"<!DOCTYPE html><html class="{class}"><head><meta property="og:description" content="{followers} Followers, 5 Following, 10 Posts - See Instagram photos and videos from {name} (&#064;someone)" /></head>
        <body><script type="application/json">{{"require":[[{{"profile_id":"42","data":{{"xdt_api__v1__feed__user_timeline_graphql_connection":{conn}}}}}]]}}</script></body></html>"#
    )
}

#[test]
fn follows_favourites_and_swiped_yes_on_instagram() {
    let t = TempDir::new();
    let ig = |h: &str| json!({"handle": h, "followers": "1M", "posts": "10", "match": "high"});
    fs::write(
        t.ud().join("creators.json"),
        json!({"version": 1, "creators": {
            "a1": {"name": "A One", "instagram": ig("a1"), "sources": ["favourite"]},
            "a2": {"name": "A Two", "instagram": ig("a2"), "sources": ["swiped"]},
            "a3": {"name": "A Three", "instagram": ig("a3"), "sources": ["favourite", "pornstar"]},
            "a4": {"name": "A Four", "instagram": ig("a4"), "sources": ["favourite"]},
            "a5": {"name": "A Five", "instagram": ig("a5"), "sources": ["favourite"]},
            "star": {"name": "Star", "instagram": ig("star"), "sources": ["pornstar"]},
            "noig": {"name": "No IG", "sources": ["favourite"]}
        }})
        .to_string(),
    )
    .unwrap();
    fs::write(
        t.ud().join("instagram-session.json"),
        json!({"cookies": [
            {"name": "sessionid", "value": "SID", "domain": ".instagram.com"},
            {"name": "csrftoken", "value": "CSRF", "domain": ".instagram.com"}],
            "savedAt": "2026-11-17T00:00:00.000Z"})
        .to_string(),
    )
    .unwrap();
    let page = |pk: &str, following: bool| {
        format!(
            r#"<html class="_9dls"><meta property="og:description" content="1M Followers, 1 Following, 10 Posts - See Instagram photos and videos from A (&#064;a)" />
            <script>{{"profile_id":"{pk}","friendship_status":{{"following":{following},"outgoing_request":false}}}}</script></html>"#
        )
    };
    let create = |pk: &str| format!("POST {IG}/api/v1/friendships/create/{pk}/");
    let mut fx = json!({
        format!("GET {IG}/a1/"): {"body": page("101", false)},
        format!("GET {IG}/a2/"): {"body": page("102", false)},
        format!("GET {IG}/a3/"): {"body": page("103", true)},
        format!("GET {IG}/a4/"): {"body": page("104", false)},
        format!("GET {IG}/a5/"): {"body": page("105", false)},
        create("101"): {"body": {"friendship_status": {"following": true, "outgoing_request": false}, "status": "ok"}},
        create("102"): {"body": {"friendship_status": {"following": false, "outgoing_request": true}, "status": "ok"}},
        create("104"): {"status": 400, "body": {"message": "feedback_required", "spam": true, "status": "fail"}}
    });
    let fixtures = t.write("fixtures.json", &fx.to_string());
    let o = run(&t, &["--follow"], Some(&fixtures));
    let out = text(&o);
    // Rate limited at A Four: stop for today.
    assert_eq!(o.status.code(), Some(2), "{out}");
    assert!(!out.contains("SID") && !out.contains("CSRF"), "{out}");
    let st = t.state();
    assert_eq!(st["follows"]["a1"]["status"], "following");
    assert_eq!(st["follows"]["a2"]["status"], "requested");
    // Followed already: no request needed.
    assert_eq!(st["follows"]["a3"]["status"], "following");
    for slug in ["a4", "a5", "star", "noig"] {
        assert!(st["follows"].get(slug).is_none(), "{slug}");
    }
    let f = t.creators();
    assert_eq!(f["creators"]["a1"]["instagram"]["followed"], "following");
    assert_eq!(f["creators"]["a2"]["instagram"]["followed"], "requested");
    assert!(f["creators"]["a4"]["instagram"]["followed"].is_null());

    // Next day: a challenge stops it, not logged in.
    fx[create("104")] =
        json!({"status": 400, "body": {"message": "challenge_required", "status": "fail"}});
    let fixtures = t.write("fixtures.json", &fx.to_string());
    let o = run(&t, &["--follow", "5"], Some(&fixtures));
    let out = text(&o);
    assert_eq!(o.status.code(), Some(3), "{out}");
    assert!(out.contains("--login"), "{out}");
    assert!(t.state()["follows"].get("a5").is_none());

    let o = run(&t, &["--status"], None);
    assert!(
        text(&o).contains("Follow: 2 followed, 1 requested"),
        "{}",
        text(&o)
    );
}
