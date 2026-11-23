//! The SLR queue: the order in which the weekly SLR download (job `slr-downloads`:
//! `slr:scout --fetch`) takes the right swipes that download from SexLikeReal (the Downloads
//! page's SLR tab), since each one costs one of the account's weekly video downloads.
//!
//! Highest score first, except the ones put first on the SLR tab (`<userData>/slr-priority.json`,
//! `{version, first: [key…], at}`), which go ahead in that order. A key is the item's
//! `<source>:<id>`: `slr:51579`, `eroscripts:349897`, `creators:eroscripts:123`.
//!
//! The tab marks what the next run takes (`thisWeek`): the first `videos.remaining`
//! (slr-scout.json's quota) that a used-up monthly script quota doesn't hold back.

use crate::downloads::Item;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

pub const PRIORITY_FILE: &str = "slr-priority.json";
const BACKUP: &str = "slr-priority.json.bak";
pub const VERSION: u32 = 1;
/// Most keys `first` may hold.
pub const MAX_FIRST: usize = 500;
const MAX_KEY_LEN: usize = 80;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Priority {
    pub version: u32,
    /// Keys put first, in order.
    pub first: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

impl Default for Priority {
    fn default() -> Priority {
        Priority {
            version: VERSION,
            first: Vec::new(),
            at: None,
        }
    }
}

/// An item's queue key.
pub fn key(item: &Item) -> String {
    format!("{}:{}", item.source, item.id)
}

/// `slr:…`, `eroscripts:…` or `creators:…` with a short id of letters, digits, `-`, `_`, `:`.
pub fn is_key(s: &str) -> bool {
    let Some((source, id)) = s.split_once(':') else {
        return false;
    };
    matches!(source, "slr" | "eroscripts" | "creators")
        && !id.is_empty()
        && s.len() <= MAX_KEY_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':'))
}

/// The priority file; a missing one is empty.
pub fn read(user_data: &Path) -> Result<Priority, String> {
    let bytes = match std::fs::read(user_data.join(PRIORITY_FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Priority::default()),
        Err(e) => return Err(format!("{PRIORITY_FILE}: {e}")),
    };
    let p: Priority =
        serde_json::from_slice(&bytes).map_err(|e| format!("{PRIORITY_FILE}: {e}"))?;
    if p.version != VERSION {
        return Err(format!(
            "{PRIORITY_FILE}: version {}, this CLI reads version {VERSION}",
            p.version
        ));
    }
    Ok(p)
}

/// Whether `first` may be written: at most MAX_FIRST keys, each a queue key.
pub fn check(first: &[String]) -> Result<(), String> {
    if first.len() > MAX_FIRST {
        return Err(format!("at most {MAX_FIRST} keys"));
    }
    match first.iter().find(|k| !is_key(k)) {
        Some(bad) => Err(format!("not a queue key: {bad}")),
        None => Ok(()),
    }
}

/// Replace the keys put first (duplicates dropped, order kept) and write the file.
pub fn write(user_data: &Path, first: Vec<String>) -> Result<Priority, String> {
    check(&first)?;
    let mut seen = std::collections::HashSet::new();
    let first: Vec<String> = first
        .into_iter()
        .filter(|k| seen.insert(k.clone()))
        .collect();
    let p = Priority {
        version: VERSION,
        first,
        at: Some(video_tags::timefmt::now_iso()),
    };
    video_tags::store::write_json_atomic(user_data, PRIORITY_FILE, BACKUP, &p)?;
    Ok(p)
}

/// Why a scene waits however high it stands: its script needs a used-up monthly SLR quota.
/// Posts bring their own script; an unknown quota holds nothing back.
fn script_wait(item: &Item, quota: &Value) -> Option<String> {
    if item.source != "slr" {
        return None;
    }
    let left = |k: &str| {
        quota
            .pointer(&format!("/{k}/remaining"))
            .and_then(Value::as_i64)
    };
    let (hand, ai) = (left("scripts"), left("aiScripts"));
    match item.script.as_deref() {
        Some("eroscripts") => None,
        Some("slr-handcrafted") if hand.is_none_or(|n| n > 0) => None,
        Some("slr-handcrafted") if item.ai_script && ai.is_none_or(|n| n > 0) => None,
        Some("slr-handcrafted") => Some(format!(
            "waits for SLR's monthly hand-crafted script quota ({} left)",
            hand.unwrap_or(0)
        )),
        Some("slr-ai") if ai.is_none_or(|n| n > 0) => None,
        Some("slr-ai") => Some(format!(
            "waits for SLR's monthly AI script quota ({} left)",
            ai.unwrap_or(0)
        )),
        _ => Some("no script to take: not downloaded".into()),
    }
}

/// Put the SLR items in queue order and mark them: `rank` (1-based), `pinned` (its place
/// among the ones put first), `note` (why it waits) and `this_week` (the next run takes it).
pub fn arrange(items: &mut [Item], first: &[String], quota: &Value) {
    let place = |i: &Item| first.iter().position(|k| *k == key(i));
    items.sort_by(|a, b| {
        let (pa, pb) = (place(a), place(b));
        match (pa, pb) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => b
                .score
                .unwrap_or(f64::MIN)
                .total_cmp(&a.score.unwrap_or(f64::MIN))
                .then_with(|| b.swiped_at.cmp(&a.swiped_at)),
        }
        .then_with(|| key(a).cmp(&key(b)))
    });
    let mut videos = quota
        .pointer("/videos/remaining")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let mut pinned = 0;
    for (n, item) in items.iter_mut().enumerate() {
        item.rank = Some(n + 1);
        if place(item).is_some() {
            pinned += 1;
            item.pinned = Some(pinned);
        }
        item.note = script_wait(item, quota);
        if item.note.is_none() && videos > 0 {
            item.this_week = true;
            videos -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(source: &'static str, id: &str, score: f64, script: Option<&str>) -> Item {
        Item {
            source,
            site: source.into(),
            id: id.into(),
            score: Some(score),
            script: script.map(String::from),
            ..Item::default()
        }
    }

    fn quota(videos: i64, hand: i64, ai: i64) -> Value {
        json!({"videos": {"remaining": videos, "total": 35},
               "scripts": {"remaining": hand, "total": 3},
               "aiScripts": {"remaining": ai, "total": 3}})
    }

    #[test]
    fn put_first_then_highest_score_and_the_quota_line() {
        let mut items = vec![
            item("slr", "1", 30.0, Some("slr-ai")),
            item("slr", "2", 20.0, Some("eroscripts")),
            item("eroscripts", "3", 25.0, None),
            item("slr", "4", 10.0, Some("slr-handcrafted")),
            item("creators", "eroscripts:5", 5.0, None),
        ];
        items[3].ai_script = true;
        let first = vec!["creators:eroscripts:5".to_string(), "slr:2".to_string()];
        arrange(&mut items, &first, &quota(3, 0, 0));
        let keys: Vec<String> = items.iter().map(key).collect();
        assert_eq!(
            keys,
            [
                "creators:eroscripts:5",
                "slr:2",
                "slr:1",
                "eroscripts:3",
                "slr:4"
            ]
        );
        let ranks: Vec<_> = items.iter().map(|i| (i.rank, i.pinned)).collect();
        assert_eq!(
            ranks,
            [
                (Some(1), Some(1)),
                (Some(2), Some(2)),
                (Some(3), None),
                (Some(4), None),
                (Some(5), None)
            ]
        );
        // Scripts used up: the AI and hand-crafted scenes wait and the quota goes further down.
        let week: Vec<bool> = items.iter().map(|i| i.this_week).collect();
        assert_eq!(week, [true, true, false, true, false]);
        assert!(
            items[2]
                .note
                .as_deref()
                .unwrap()
                .contains("AI script quota")
        );
        assert!(items[4].note.as_deref().unwrap().contains("hand-crafted"));
    }

    #[test]
    fn script_fallbacks_and_unknown_quota() {
        let hand = |ai_script| Item {
            ai_script,
            ..item("slr", "1", 1.0, Some("slr-handcrafted"))
        };
        assert_eq!(script_wait(&hand(true), &quota(5, 0, 2)), None);
        assert!(script_wait(&hand(false), &quota(5, 0, 2)).is_some());
        assert_eq!(script_wait(&hand(false), &Value::Null), None);
        assert!(script_wait(&item("slr", "1", 1.0, None), &quota(5, 3, 3)).is_some());
        // Without a quota read nothing is marked for the week.
        let mut items = vec![item("eroscripts", "3", 1.0, None)];
        arrange(&mut items, &[], &Value::Null);
        assert!(!items[0].this_week);
    }

    #[test]
    fn keys_and_the_file() {
        for good in ["slr:51579", "eroscripts:12", "creators:eroscripts:5"] {
            assert!(is_key(good), "{good}");
        }
        for bad in ["slr:", "vrb:1", "slr", "slr:a b", "eroscripts:1/2"] {
            assert!(!is_key(bad), "{bad}");
        }
        let dir = std::env::temp_dir().join(format!("scout-report-slrq-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read(&dir).unwrap().first, Vec::<String>::new());
        let p = write(&dir, vec!["slr:2".into(), "slr:1".into(), "slr:2".into()]).unwrap();
        assert_eq!(p.first, ["slr:2", "slr:1"]);
        assert_eq!(read(&dir).unwrap().first, ["slr:2", "slr:1"]);
        assert!(write(&dir, vec!["vrb:1".into()]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
