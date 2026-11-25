//! Custom routing (`renderer/js/custom-routing-match.js`): a video's
//! association (`settings.library.associations[path]`, `active: "custom"`)
//! gives each Buttplug device its own script. A route finds its device by
//! the stored index when the name there still matches, else by name (the
//! caller then stores the new index); devices without a route get nothing.

use serde_json::Value;

use super::interp::Action;
use super::script;

#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    /// `buttplug:<device name>`.
    pub device_id: String,
    pub index: Option<u64>,
    pub actions: Vec<Action>,
}

/// The routes of an association entry; the `main` route plays `main`.
pub fn load(entry: &Value, main: &[Action]) -> Vec<Route> {
    if entry["active"] != "custom" {
        return Vec::new();
    }
    entry["custom"]["routes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let device_id = r["deviceId"].as_str()?;
            device_id.starts_with("buttplug:").then_some(())?;
            let actions = if r["role"] == "main" {
                main.to_vec()
            } else {
                script::read_actions(r["scriptPath"].as_str()?)?
            };
            Some(Route {
                device_id: device_id.into(),
                index: r["buttplugIndex"].as_u64(),
                actions,
            })
        })
        .collect()
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
    fn only_custom_entries_route_buttplug_devices() {
        let main = vec![Action { at: 0.0, pos: 0.0 }, Action { at: 1.0, pos: 1.0 }];
        let e = serde_json::json!({"active": "custom", "custom": {"routes": [
            {"deviceId": "buttplug:Handy", "buttplugIndex": 3, "role": "main"},
            {"deviceId": "tcode", "role": "main"},
            {"deviceId": "buttplug:Lush", "scriptPath": "/nope.funscript"},
        ]}});
        assert_eq!(
            load(&e, &main),
            vec![Route {
                device_id: "buttplug:Handy".into(),
                index: Some(3),
                actions: main.clone()
            }]
        );
        assert!(load(&serde_json::json!({"active": "single"}), &main).is_empty());
    }
}
