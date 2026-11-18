//! `--follow [N]`: our Instagram account (the `--login` session) follows the
//! creators the user wants: a confirmed Instagram account and source
//! `favourite` or `swiped` (yes on the Creators site; one swiped no is
//! left alone). Like the web app: her profile page (already followed?
//! her user id), then `POST /api/v1/friendships/create/<id>/`. 20–40 s
//! between creators; the first challenge or lost session stops the run
//! (exit 3), a rate limit or "feedback required" too (exit 2, next day).
//! Recorded in the state (`follows`) and on her Instagram entry
//! (`followed`: following / requested); never undone.

use crate::instagram::{self, Answer};
use crate::state::{Follow, State, Swipes};
use crate::store::{self, CreatorsFile};
use serde_json::Value;
use slr_scout::http::{Http, Request, Response};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const APP_ID: &str = "936619743392459";

/// Creators to follow, in slug order: an Instagram account, a source
/// favourite or swiped, not swiped no, not followed (or requested) yet.
pub fn due(file: &CreatorsFile, state: &State, swipes: &Swipes) -> Vec<String> {
    file.creators
        .iter()
        .filter(|(slug, c)| {
            c.instagram.is_some()
                && c.sources.iter().any(|s| s == "favourite" || s == "swiped")
                && !swipes.creator_is_no(slug)
                && state
                    .follows
                    .get(*slug)
                    .is_none_or(|f| f.status == "failed")
        })
        .map(|(slug, _)| slug.clone())
        .collect()
}

/// Her user id in her profile page: `"profile_id":"…"`, else the owner of
/// the timeline's posts.
pub fn user_pk(page: &str) -> Option<String> {
    let key = "\"profile_id\":\"";
    if let Some(at) = page.find(key) {
        let id: String = page[at + key.len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if !id.is_empty() {
            return Some(id);
        }
    }
    let conn = instagram::timeline_json(page)?;
    conn["edges"].as_array()?.iter().find_map(|e| {
        let n = &e["node"];
        instagram::text_id(&n["owner_id"]["pk"])
            .or_else(|| instagram::text_id(&n["owner"]["pk"]))
            .or_else(|| instagram::text_id(&n["user"]["pk"]))
            .filter(|id| id.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// Whether her page says we follow her already.
pub fn already_following(page: &str) -> bool {
    if page.contains("\"followed_by_viewer\":true") {
        return true;
    }
    let key = "\"friendship_status\":{";
    page.match_indices(key).any(|(at, _)| {
        let rest = &page[at + key.len()..];
        let end = rest.find('}').unwrap_or(rest.len());
        rest[..end].contains("\"following\":true")
    })
}

/// The `csrftoken` of a session's cookie header.
fn csrf(cookie: &str) -> Option<&str> {
    cookie
        .split("; ")
        .find_map(|c| c.strip_prefix("csrftoken="))
}

/// The web app's follow request.
pub fn follow_request(base: &str, pk: &str, handle: &str, cookie: &str) -> Request {
    let req = Request::post(
        format!("{base}/api/v1/friendships/create/{pk}/"),
        format!("container_module=profile&user_id={pk}"),
    )
    .header("User-Agent", slr_scout::http::USER_AGENT)
    .header("Accept", "*/*")
    .header("Content-Type", "application/x-www-form-urlencoded")
    .header("x-ig-app-id", APP_ID)
    .header("X-Requested-With", "XMLHttpRequest")
    .header("Origin", base)
    .header("Referer", format!("{base}/{handle}/"))
    .header("Sec-Fetch-Site", "same-origin")
    .header("Sec-Fetch-Mode", "cors")
    .header("Sec-Fetch-Dest", "empty")
    .header("Cookie", cookie);
    match csrf(cookie) {
        Some(token) => req.header("X-CSRFToken", token),
        None => req,
    }
}

/// What a follow answer means.
#[derive(Debug, Clone, PartialEq)]
pub enum Followed {
    Following,
    /// A private account: the request waits for her.
    Requested,
    NotLoggedIn,
    Challenge,
    /// 429, "feedback required", "please wait": stop for today.
    Limited(String),
    Failed(String),
}

pub fn classify_follow(res: &Response) -> Followed {
    let body = res.body.as_str();
    let lower = body.to_lowercase();
    let location = res.header_values("location").next().unwrap_or("");
    let challenge = lower.contains("checkpoint_required") || lower.contains("challenge_required");
    let limited = lower.contains("feedback_required") || lower.contains("please wait");
    match res.status {
        429 => Followed::Limited("HTTP 429".into()),
        300..=399 if location.contains("/challenge") => Followed::Challenge,
        300..=399 if location.contains("/accounts/login") => Followed::NotLoggedIn,
        _ if challenge => Followed::Challenge,
        _ if limited => Followed::Limited("Instagram asks to wait (feedback required)".into()),
        401 | 403 => Followed::NotLoggedIn,
        200..=299 => {
            let Ok(v) = serde_json::from_str::<Value>(body) else {
                return Followed::NotLoggedIn;
            };
            let f = &v["friendship_status"];
            if f["following"] == true {
                Followed::Following
            } else if f["outgoing_request"] == true {
                Followed::Requested
            } else if v["require_login"] == true {
                Followed::NotLoggedIn
            } else {
                Followed::Failed("unexpected answer".into())
            }
        }
        s => Followed::Failed(format!("HTTP {s}")),
    }
}

/// 20–40 s between creators.
fn pause() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    std::thread::sleep(Duration::from_millis(20_000 + u64::from(nanos) % 20_001));
}

pub struct FollowRun {
    pub code: i32,
    pub following: usize,
    pub requested: usize,
    pub failed: usize,
    pub left: usize,
}

/// `--follow N`. Writes creators.json and the state after each creator.
#[allow(clippy::too_many_arguments)]
pub fn run(
    http: &dyn Http,
    base: &str,
    user_data: &Path,
    file: &mut CreatorsFile,
    state: &mut State,
    swipes: &Swipes,
    limit: usize,
    fake: bool,
    out: &mut dyn Write,
) -> Result<FollowRun, String> {
    let due = due(file, state, swipes);
    let mut run = FollowRun {
        code: 0,
        following: 0,
        requested: 0,
        failed: 0,
        left: due.len().saturating_sub(limit),
    };
    if due.is_empty() {
        return Ok(run);
    }
    let Some(cookie) = instagram::read_session(user_data)?
        .as_ref()
        .and_then(instagram::Session::cookie_header)
    else {
        let _ = writeln!(out, "No Instagram session: nobody followed.");
        run.code = 3;
        return Ok(run);
    };
    for (i, slug) in due.iter().take(limit).enumerate() {
        if i > 0 && !fake {
            pause();
        }
        let Some(handle) = file
            .creators
            .get(slug)
            .and_then(|c| c.instagram.as_ref())
            .map(|ig| ig.handle.clone())
        else {
            continue;
        };
        let now = video_tags::timefmt::now_iso();
        let res = http
            .send(&instagram::timeline_request(base, &handle, &cookie))
            .map_err(|e| format!("Instagram: {e}"))?;
        let record = |status: &str, error: Option<String>| Follow {
            handle: handle.clone(),
            at: now.clone(),
            status: status.into(),
            error,
        };
        let outcome = match instagram::classify_page(&res) {
            Answer::Profile(_) if already_following(&res.body) => Followed::Following,
            Answer::Profile(_) => match user_pk(&res.body) {
                Some(pk) => {
                    if !fake {
                        std::thread::sleep(Duration::from_secs(3));
                    }
                    let res = http
                        .send(&follow_request(base, &pk, &handle, &cookie))
                        .map_err(|e| format!("Instagram: {e}"))?;
                    classify_follow(&res)
                }
                None => Followed::Failed("no user id in her page".into()),
            },
            Answer::Missing => Followed::Failed("account not found".into()),
            Answer::NotLoggedIn => Followed::NotLoggedIn,
            Answer::Challenge => Followed::Challenge,
            Answer::Limited(why) => Followed::Limited(why),
            Answer::Failed(why) => Followed::Failed(why),
        };
        let followed = match &outcome {
            Followed::Following => Some("following"),
            Followed::Requested => Some("requested"),
            _ => None,
        };
        match outcome {
            Followed::Following | Followed::Requested => {
                let status = followed.unwrap_or_default();
                if status == "following" {
                    run.following += 1;
                } else {
                    run.requested += 1;
                }
                let _ = writeln!(out, "  @{handle}: {status}");
                state.follows.insert(slug.clone(), record(status, None));
                if let Some(ig) = file
                    .creators
                    .get_mut(slug)
                    .and_then(|c| c.instagram.as_mut())
                {
                    ig.followed = Some(status.into());
                }
            }
            Followed::Failed(why) => {
                let _ = writeln!(out, "  @{handle}: {why}; tried again next run.");
                run.failed += 1;
                run.code = run.code.max(2);
                state
                    .follows
                    .insert(slug.clone(), record("failed", Some(why)));
            }
            Followed::NotLoggedIn => {
                let _ = writeln!(
                    out,
                    "Instagram asks to log in: the session expired; run --login."
                );
                run.code = 3;
                break;
            }
            Followed::Challenge => {
                let _ = writeln!(
                    out,
                    "Instagram asks to confirm the account (a challenge): run --login and finish it in the window."
                );
                run.code = 3;
                break;
            }
            Followed::Limited(why) => {
                let _ = writeln!(out, "Instagram: {why}; no more follows today.");
                run.code = 2;
                break;
            }
        }
        store::write(user_data, file)?;
        crate::state::write(user_data, state)?;
    }
    if run.code == 0 && run.left > 0 {
        run.code = 2;
    }
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Creator, Instagram};

    fn res(status: u16, body: &str) -> Response {
        Response {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    #[test]
    fn classifies_follow_answers() {
        assert_eq!(
            classify_follow(&res(
                200,
                r#"{"friendship_status":{"following":true,"outgoing_request":false},"status":"ok"}"#
            )),
            Followed::Following
        );
        assert_eq!(
            classify_follow(&res(
                200,
                r#"{"friendship_status":{"following":false,"outgoing_request":true},"status":"ok"}"#
            )),
            Followed::Requested
        );
        assert!(matches!(
            classify_follow(&res(429, "")),
            Followed::Limited(_)
        ));
        assert!(matches!(
            classify_follow(&res(
                400,
                r#"{"message":"feedback_required","spam":true,"status":"fail"}"#
            )),
            Followed::Limited(_)
        ));
        assert!(matches!(
            classify_follow(&res(
                400,
                r#"{"message":"Please wait a few minutes before you try again.","status":"fail"}"#
            )),
            Followed::Limited(_)
        ));
        assert_eq!(
            classify_follow(&res(
                400,
                r#"{"message":"challenge_required","status":"fail"}"#
            )),
            Followed::Challenge
        );
        assert_eq!(classify_follow(&res(401, "{}")), Followed::NotLoggedIn);
        assert_eq!(
            classify_follow(&res(200, "<html>login</html>")),
            Followed::NotLoggedIn
        );
        assert_eq!(
            classify_follow(&res(404, "<html></html>")),
            Followed::Failed("HTTP 404".into())
        );
    }

    #[test]
    fn reads_the_profile_page() {
        let page = r#"{"props":{"profile_id":"123456","x":1}}"#;
        assert_eq!(user_pk(page).as_deref(), Some("123456"));
        let timeline = r#"<script>{"xdt_api__v1__feed__user_timeline_graphql_connection":{"edges":[{"node":{"code":"A","owner_id":{"pk":"777"}}}]}}</script>"#;
        assert_eq!(user_pk(timeline).as_deref(), Some("777"));
        assert_eq!(user_pk("<html></html>"), None);
        assert!(already_following(
            r#"{"friendship_status":{"followed_by":false,"following":true}}"#
        ));
        assert!(already_following(r#"{"followed_by_viewer":true}"#));
        assert!(!already_following(
            r#"{"friendship_status":{"following":false},"x":{"following":true}}"#
        ));
        let req = follow_request(
            "https://www.instagram.com",
            "123",
            "ann",
            "csrftoken=C; sessionid=S",
        );
        assert_eq!(
            req.url,
            "https://www.instagram.com/api/v1/friendships/create/123/"
        );
        assert_eq!(
            req.body.as_deref(),
            Some("container_module=profile&user_id=123")
        );
        assert!(req.headers.contains(&("X-CSRFToken".into(), "C".into())));
        assert!(
            req.headers
                .contains(&("Referer".into(), "https://www.instagram.com/ann/".into()))
        );
    }

    #[test]
    fn follows_favourites_and_swiped_yes_with_instagram() {
        let mut file = CreatorsFile::empty();
        let mk = |sources: &[&str], ig: bool| Creator {
            instagram: ig.then(Instagram::default),
            sources: sources.iter().map(|s| s.to_string()).collect(),
            ..Creator::default()
        };
        file.creators.insert("fav".into(), mk(&["favourite"], true));
        file.creators.insert("star".into(), mk(&["pornstar"], true));
        file.creators.insert("swiped".into(), mk(&["swiped"], true));
        file.creators
            .insert("noig".into(), mk(&["favourite"], false));
        file.creators
            .insert("done".into(), mk(&["favourite"], true));
        file.creators
            .insert("failed".into(), mk(&["favourite"], true));
        file.creators
            .insert("said".into(), mk(&["favourite"], true));
        let mut state = State::default();
        let rec = |status: &str| Follow {
            handle: "h".into(),
            at: "t".into(),
            status: status.into(),
            error: None,
        };
        state.follows.insert("done".into(), rec("requested"));
        state.follows.insert("failed".into(), rec("failed"));
        let mut swipes = Swipes::default();
        swipes.creators.insert("said".into(), "no".into());
        assert_eq!(due(&file, &state, &swipes), ["failed", "fav", "swiped"]);
    }
}
