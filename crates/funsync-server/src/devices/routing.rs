//! Custom routing (ported from the old Electron app): a video's
//! association (`settings.library.associations[path]`, `active: "custom"`)
//! gives each device its own script: `tcode`, or a Buttplug device found by
//! the stored index when the name there still matches, else by name (the
//! caller then stores the new index); Buttplug devices without a route get
//! nothing. A route's script that moved is looked for by name in the
//! video's folder (and the new path stored).

use std::path::Path;

use serde_json::Value;

use super::interp::Action;
use super::script;

#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    /// Position in the stored `custom.routes`.
    pub slot: usize,
    /// `tcode` or `buttplug:<device name>`.
    pub device_id: String,
    pub index: Option<u64>,
    pub actions: Vec<Action>,
}

/// The routes of an association entry (the `main` route plays `main`), and
/// `(slot, new path)` for scripts found again in `dir`.
pub fn load(entry: &Value, main: &[Action], dir: &Path) -> (Vec<Route>, Vec<(usize, String)>) {
    let mut moved = Vec::new();
    if entry["active"] != "custom" {
        return (Vec::new(), moved);
    }
    let routes = entry["custom"]["routes"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(slot, r)| {
            let device_id = r["deviceId"].as_str()?;
            (device_id == "tcode" || device_id.starts_with("buttplug:")).then_some(())?;
            let actions = if r["role"] == "main" {
                main.to_vec()
            } else {
                let p = r["scriptPath"].as_str()?;
                script::read_actions(p).or_else(|| {
                    let again = dir.join(Path::new(p).file_name()?);
                    let a = script::read_actions(again.to_str()?)?;
                    moved.push((slot, again.to_string_lossy().into_owned()));
                    Some(a)
                })?
            };
            Some(Route {
                slot,
                device_id: device_id.into(),
                index: r["buttplugIndex"].as_u64(),
                actions,
            })
        })
        .collect();
    (routes, moved)
}

/// The device a route drives among `(index, name)`: the stored index when
/// its name still matches, else the first device of that name not in
/// `taken`.
pub fn find(route: &Route, devices: &[(u64, String)], taken: &[u64]) -> Option<u64> {
    let is = |d: &(u64, String)| route.device_id == format!("buttplug:{}", d.1);
    if let Some(d) = devices.iter().find(|d| Some(d.0) == route.index)
        && is(d)
    {
        return Some(d.0);
    }
    devices
        .iter()
        .find(|d| is(d) && !taken.contains(&d.0))
        .map(|d| d.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(name: &str, index: Option<u64>) -> Route {
        Route {
            slot: 0,
            device_id: format!("buttplug:{name}"),
            index,
            actions: vec![],
        }
    }

    #[test]
    fn index_first_then_name() {
        let devs = vec![
            (0, "Lush".to_string()),
            (1, "Handy".to_string()),
            (2, "Lush".to_string()),
        ];
        assert_eq!(find(&route("Handy", Some(1)), &devs, &[]), Some(1));
        // Intiface reshuffled: index 0 is now another device; found by name.
        assert_eq!(find(&route("Handy", Some(0)), &devs, &[]), Some(1));
        // Two devices of one name: the second route skips the claimed one.
        assert_eq!(find(&route("Lush", None), &devs, &[0]), Some(2));
        assert_eq!(find(&route("Lush", Some(2)), &devs, &[2]), Some(2));
        assert_eq!(find(&route("Nora", Some(1)), &devs, &[]), None);
    }

    #[test]
    fn custom_entries_route_tcode_and_buttplug_and_find_moved_scripts() {
        let t = tempfile::tempdir().unwrap();
        let flat = r#"{"actions":[{"at":0,"pos":5},{"at":9,"pos":5}]}"#;
        std::fs::write(t.path().join("moved.funscript"), flat).unwrap();
        let main = vec![Action { at: 0.0, pos: 0.0 }, Action { at: 1.0, pos: 1.0 }];
        let e = serde_json::json!({"active": "custom", "custom": {"routes": [
            {"deviceId": "buttplug:Handy", "buttplugIndex": 3, "role": "main"},
            {"deviceId": "other", "role": "main"},
            {"deviceId": "buttplug:Lush", "scriptPath": "/nope.funscript"},
            {"deviceId": "tcode", "scriptPath": "/old/place/moved.funscript"},
        ]}});
        let (routes, moved) = load(&e, &main, t.path());
        let ids: Vec<_> = routes
            .iter()
            .map(|r| (r.slot, r.device_id.as_str()))
            .collect();
        assert_eq!(ids, [(0, "buttplug:Handy"), (3, "tcode")]);
        assert_eq!(routes[0].actions, main);
        assert_eq!(routes[1].actions[0].pos, 5.0);
        let p = t
            .path()
            .join("moved.funscript")
            .to_string_lossy()
            .into_owned();
        assert_eq!(moved, [(3, p)]);
        assert!(
            load(&serde_json::json!({"active": "single"}), &main, t.path())
                .0
                .is_empty()
        );
    }
}
