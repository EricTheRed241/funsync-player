//! The scouts' files in userData and the readers the deck builds on: each file is parsed
//! fresh on every request, and a missing one reads as `Null` (an empty section).

use crate::swipes::{Store, Swipes};
use serde_json::Value;
use std::path::Path;

pub const STATE_FILE: &str = "eroscripts-scout-state.json";
pub const WAITING_FILE: &str = "eroscripts-scout.json";

pub const FORUM_TOPIC: &str = "https://discuss.eroscripts.com/t/";

/// Read and parse `<user_data>/<name>`: `Value::Null` when missing, or when
/// unreadable / unparsable (then also a line in `problems`).
pub fn read_json(user_data: &Path, name: &str, problems: &mut Vec<String>) -> Value {
    match std::fs::read(user_data.join(name)) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            problems.push(format!("{name}: {e}"));
            Value::Null
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Null,
        Err(e) => {
            problems.push(format!("{name}: {e}"));
            Value::Null
        }
    }
}

/// A deck's swipes; an unreadable file is a line in `problems` and no swipes.
pub fn read_swipes(user_data: &Path, store: Store, problems: &mut Vec<String>) -> Swipes {
    store.read(user_data).unwrap_or_else(|e| {
        problems.push(e);
        Swipes::default()
    })
}

pub(crate) fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn strings_of(v: &Value, key: &str) -> Option<Vec<String>> {
    v.get(key).and_then(Value::as_array).map(|a| {
        a.iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    })
}

/// A topic id from a number or a numeric string.
pub(crate) fn id_of(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Total size of the scenes' videos whose size is known, in GB (10^9 bytes, 2 decimals).
pub(crate) fn total_gb(scenes: &[Value]) -> Option<f64> {
    let sizes: Vec<f64> = scenes
        .iter()
        .filter_map(|s| s.pointer("/video/size").and_then(Value::as_f64))
        .collect();
    if sizes.is_empty() {
        return None;
    }
    let gb = sizes.iter().sum::<f64>() / 1e9;
    Some((gb * 100.0).round() / 100.0)
}
