//! A video's scripts as TCode axes: the main script (L0), its companion
//! axis files and the axes embedded in it (`renderer/js/embedded-multi-axis.js`).
//! Hand-made axes win: once any exists, generated companions are not played.

use std::collections::BTreeMap;

use serde_json::Value;

use super::interp::Action;
use crate::scan::{Axis, Video};

/// FunSync axis suffix ↔ TCode axis.
const AXES: [(&str, &str); 9] = [
    ("surge", "L1"),
    ("sway", "L2"),
    ("twist", "R0"),
    ("roll", "R1"),
    ("pitch", "R2"),
    ("vib", "V0"),
    ("lube", "V1"),
    ("suction", "V2"),
    ("valve", "A0"),
];

pub fn tcode_of(suffix: &str) -> Option<&'static str> {
    AXES.iter().find(|a| a.0 == suffix).map(|a| a.1)
}

/// Axis name (alias or TCode id, any case) → suffix.
fn suffix_of(name: &str) -> Option<&'static str> {
    let n = name.trim().to_lowercase();
    let alias = match n.as_str() {
        "rotate" => "twist",
        "vibrate" | "vibration" => "vib",
        "suck" => "suction",
        "pump" => return None,
        n => n,
    };
    AXES.iter()
        .find(|a| a.0 == alias || a.1.eq_ignore_ascii_case(alias))
        .map(|a| a.0)
}

/// A funscript action array (or `{actions}`), sorted, at least 2 actions.
fn actions(v: &Value) -> Option<Vec<Action>> {
    let arr = v.as_array().or_else(|| v["actions"].as_array())?;
    let mut out: Vec<Action> = arr
        .iter()
        .map(|a| {
            Some(Action {
                at: a["at"].as_f64()?,
                pos: a["pos"].as_f64().unwrap_or(0.0),
            })
        })
        .collect::<Option<_>>()?;
    out.sort_by(|a, b| a.at.total_cmp(&b.at));
    (out.len() >= 2).then_some(out)
}

/// Axes embedded in a main script, by suffix; earlier formats win.
pub fn embedded(s: &Value) -> BTreeMap<&'static str, Vec<Action>> {
    let mut out = BTreeMap::new();
    let mut put = |name: &str, v: &Value| {
        if let (Some(k), Some(a)) = (suffix_of(name), actions(v)) {
            out.entry(k).or_insert(a);
        }
    };
    for e in s["additional_axes"].as_array().into_iter().flatten() {
        put(e["axis"].as_str().unwrap_or(""), &e["actions"]);
    }
    for (k, v) in s["raw"].as_object().into_iter().flatten() {
        put(k, &v["actions"]);
    }
    for e in s["axes"].as_array().into_iter().flatten() {
        put(e["id"].as_str().unwrap_or(""), &e["actions"]);
    }
    for (k, v) in s["channels"].as_object().into_iter().flatten() {
        put(k, v);
    }
    const SKIP: [&str; 4] = ["actions", "raw", "axes", "channels"];
    for (k, v) in s.as_object().into_iter().flatten() {
        if !SKIP.contains(&k.as_str()) {
            put(k, v);
        }
    }
    // Inline TCode values on each action (`{"at":0,"pos":50,"R0":80}`).
    for (suffix, id) in AXES {
        let inline: Vec<Value> = s["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| Some(serde_json::json!({"at": a["at"], "pos": a.get(id)?})))
            .collect();
        put(suffix, &Value::Array(inline));
    }
    out
}

fn read(path: &str) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Axis companions next to a script: `<stem>.<axis>.funscript` and
/// `<stem>.generated.<axis>.funscript`.
fn companions(script: &str) -> Vec<Axis> {
    let stem = script.strip_suffix(".funscript").unwrap_or(script);
    let mut out = Vec::new();
    for (axis, _) in AXES {
        for (generated, mid) in [(false, ""), (true, "generated.")] {
            let path = format!("{stem}.{mid}{axis}.funscript");
            if std::path::Path::new(&path).is_file() {
                out.push(Axis {
                    axis: axis.into(),
                    path,
                    generated,
                });
            }
        }
    }
    out
}

/// `(TCode axis, actions)` for every axis the video plays (its main script,
/// or the variant labelled `variant` with that variant's own axes; the
/// video's hand-made axes fill in, and beat generated ones); None without a
/// playable script.
pub fn load(v: &Video, variant: Option<&str>) -> Option<Vec<(String, Vec<Action>)>> {
    let (path, axes_files) = match variant {
        None => (v.funscript_path.clone()?, v.axes.clone()),
        Some(label) => {
            let p = v.variants.iter().find(|x| x.label == label)?.path.clone();
            let mut files: Vec<Axis> = v.axes.iter().filter(|a| !a.generated).cloned().collect();
            files.extend(companions(&p));
            (p, files)
        }
    };
    let main = read(&path)?;
    let mut axes = vec![("L0".to_string(), actions(&main["actions"])?)];
    let mut by_suffix = embedded(&main);
    let hand_made = !by_suffix.is_empty() || axes_files.iter().any(|a| !a.generated);
    for a in &axes_files {
        let Some(k) = suffix_of(&a.axis) else {
            continue;
        };
        if hand_made && a.generated {
            continue;
        }
        if let Some(acts) = read(&a.path).as_ref().and_then(actions) {
            by_suffix.insert(k, acts);
        }
    }
    axes.extend(
        by_suffix
            .into_iter()
            .filter_map(|(k, a)| Some((tcode_of(k)?.to_string(), a))),
    );
    Some(axes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn embedded_formats() {
        let two = json!([{"at": 0, "pos": 0}, {"at": 10, "pos": 9}]);
        let s = json!({
            "actions": [{"at": 0, "pos": 1, "R2": 5}, {"at": 9, "pos": 2, "R2": 6}],
            "additional_axes": [{"axis": "twist", "actions": two}],
            "raw": {"twist": {"actions": [{"at": 0, "pos": 50}, {"at": 1, "pos": 50}]}},
            "axes": [{"id": "r1", "actions": two}, {"id": "L0", "actions": two}],
            "channels": {"L1": two},
            "sway": {"actions": two},
            "vib": [{"at": 0, "pos": 1}],
        });
        let e = embedded(&s);
        let keys: Vec<_> = e.keys().copied().collect();
        assert_eq!(keys, ["pitch", "roll", "surge", "sway", "twist"]);
        assert_eq!(e["twist"][1].pos, 9.0, "additional_axes beats raw");
        assert_eq!(e["pitch"][1].pos, 6.0);
    }
}
