//! Manual script association (`library.associations[videoPath]`, the old
//! app's shape): `{active: single|multi|custom, single, multi: {main, axes},
//! custom}`. An active `single` / `multi` entry whose files exist replaces
//! the scan's pairing; `custom` is device routing (devices::routing).

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::scan::{Axis, Video};

/// Apply every active `single` / `multi` association to the scanned videos.
pub fn apply(associations: &Value, videos: &mut [Video]) {
    let exists = |p: &str| Path::new(p).is_file();
    for v in videos.iter_mut() {
        let e = &associations[&v.path];
        match e["active"].as_str() {
            Some("single") => {
                if let Some(p) = e["single"].as_str().filter(|p| exists(p)) {
                    v.funscript_path = Some(p.to_string());
                }
            }
            Some("multi") => {
                if let Some(p) = e["multi"]["main"].as_str().filter(|p| exists(p)) {
                    v.funscript_path = Some(p.to_string());
                }
                let axes: Vec<Axis> = e["multi"]["axes"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter_map(|(axis, p)| {
                        let p = p.as_str().filter(|p| exists(p))?;
                        Some(Axis {
                            axis: axis.clone(),
                            path: p.to_string(),
                            generated: p.contains(".generated."),
                        })
                    })
                    .collect();
                v.manual_axes = axes;
            }
            _ => {}
        }
    }
}

/// The stored entry with `active` and its slot set, the other slots kept
/// (association-shape.js `buildAssociationEntry`, mirror fields included).
pub fn entry(
    old: &Value,
    active: Option<&str>,
    single: Option<&str>,
    multi: Option<Value>,
) -> Value {
    let mut e = Map::new();
    e.insert("active".into(), json!(active));
    e.insert(
        "single".into(),
        single.map_or_else(|| old["single"].clone(), |s| json!(s)),
    );
    e.insert(
        "multi".into(),
        multi.unwrap_or_else(|| old["multi"].clone()),
    );
    e.insert("custom".into(), old["custom"].clone());
    for k in ["single", "multi", "custom"] {
        if e[k].is_null() {
            e.insert(k.into(), Value::Null);
        }
    }
    if active == Some("custom") && !e["custom"].is_null() {
        e.insert("mode".into(), json!("custom"));
        e.insert("routes".into(), e["custom"]["routes"].clone());
    } else if active == Some("multi") && !e["multi"].is_null() {
        e.insert("main".into(), e["multi"]["main"].clone());
        e.insert(
            "axes".into(),
            e["multi"].get("axes").cloned().unwrap_or(json!({})),
        );
        e.insert(
            "buttplugVib".into(),
            json!(e["multi"]["buttplugVib"] == true),
        );
    }
    Value::Object(e)
}
#[cfg(test)]
mod tests {
    use super::*;

    fn video(path: &str) -> Video {
        serde_json::from_value(json!({"id": "x", "name": "V.mp4", "path": path, "ext": ".mp4",
            "funscriptPath": null, "fuzzyMatched": false, "subtitlePath": null, "variants": [], "axes": [],
            "sourceName": "", "dateAdded": 0.0, "size": 0})).unwrap()
    }

    #[test]
    fn active_entries_override_the_scan_and_missing_files_are_ignored() {
        let t = tempfile::tempdir().unwrap();
        let main = t.path().join("Other.funscript");
        let twist = t.path().join("Other.generated.twist.funscript");
        std::fs::write(&main, "{}").unwrap();
        std::fs::write(&twist, "{}").unwrap();
        let (m, tw) = (
            main.to_string_lossy().into_owned(),
            twist.to_string_lossy().into_owned(),
        );
        let assoc = json!({
            "/a.mp4": {"active": "multi", "multi": {"main": m, "axes": {"twist": tw, "roll": "/gone.funscript"}}},
            "/b.mp4": {"active": "single", "single": "/gone.funscript"},
            "/c.mp4": {"active": "custom", "single": m},
        });
        let mut vids = vec![video("/a.mp4"), video("/b.mp4"), video("/c.mp4")];
        apply(&assoc, &mut vids);
        assert_eq!(vids[0].funscript_path.as_deref(), Some(m.as_str()));
        assert_eq!(vids[0].manual_axes.len(), 1);
        assert!(vids[0].manual_axes[0].generated);
        assert!(
            vids[0].axes.is_empty(),
            "the scan's axes (axis kind) stay as scanned"
        );
        assert!(vids[1].funscript_path.is_none());
        assert!(vids[2].funscript_path.is_none());
    }

    #[test]
    fn entry_keeps_other_slots_and_mirrors_multi() {
        let old = json!({"active": "custom", "custom": {"routes": [1]}, "single": "/s"});
        let e = entry(
            &old,
            Some("multi"),
            None,
            Some(json!({"main": "/m", "axes": {"roll": "/r"}})),
        );
        assert_eq!(e["custom"]["routes"], json!([1]));
        assert_eq!(e["single"], "/s");
        assert_eq!(e["main"], "/m");
        assert_eq!(e["axes"]["roll"], "/r");
        let cleared = entry(&e, None, None, None);
        assert_eq!(cleared["active"], Value::Null);
        assert_eq!(cleared["multi"]["main"], "/m");
    }
}
