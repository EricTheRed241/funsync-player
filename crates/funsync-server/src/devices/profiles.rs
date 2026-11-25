//! TCode device profiles (ported from the old Electron app): MiraBot, FUNSR
//! MINI and Custom, one at a time. `settings.tcode.*` holds the active one,
//! `settings.tcode.profiles.saved` the others; FUNSR MINI `D` commands.

use serde_json::{Map, Value, json};

const FIELDS: [&str; 14] = [
    "transport",
    "port",
    "baudRate",
    "udpHost",
    "udpPort",
    "wsUrl",
    "remoteBaudRate",
    "precision",
    "updateRateHz",
    "axes",
    "defaultOffset",
    "defaultOffsetSource",
    "autoConnect",
    "funsr",
];

/// `(id, display name)`; Custom has no name of its own.
pub const PROFILES: [(&str, &str); 3] = [
    ("custom", ""),
    ("mirabot", "MiraBot"),
    ("funsr-mini", "FUNSR MINI"),
];

fn axes_only(on: &[&str]) -> Value {
    let all = [
        "L0", "L1", "L2", "R0", "R1", "R2", "V0", "V1", "A0", "A1", "A2",
    ];
    all.iter()
        .map(|a| (a.to_string(), json!({"enabled": on.contains(a)})))
        .collect::<Map<_, _>>()
        .into()
}

fn defaults(id: &str) -> Value {
    let mut d = json!({
        "transport": "serial", "port": "", "baudRate": 115200, "udpHost": "", "udpPort": 0,
        "wsUrl": "", "remoteBaudRate": 115200, "precision": 3, "updateRateHz": 60, "axes": {},
        "defaultOffset": null, "defaultOffsetSource": null, "autoConnect": null, "funsr": null,
    });
    let own = match id {
        "mirabot" => json!({"transport": "websocket", "wsUrl": "ws://mirabot.local:8888",
            "precision": 4, "axes": axes_only(&["L0", "R0", "R1", "R2"])}),
        "funsr-mini" => {
            let mut axes = axes_only(&["L0", "R0"]);
            axes["L0"] = json!({"enabled": true, "min": 5, "max": 95});
            json!({"precision": 4, "axes": axes})
        }
        _ => json!({}),
    };
    for (k, v) in own.as_object().unwrap() {
        d[k] = v.clone();
    }
    d
}

/// The active profile: `profiles.active`, else MiraBot for a MiraBot URL.
pub fn active(tc: &Value) -> &'static str {
    let a = tc["profiles"]["active"].as_str().unwrap_or("");
    if let Some(p) = PROFILES.iter().find(|p| p.0 == a) {
        return p.0;
    }
    let ws = tc["wsUrl"].as_str().unwrap_or("").to_lowercase();
    if tc["transport"] == "websocket" && ws.contains("mirabot") {
        "mirabot"
    } else {
        "custom"
    }
}

/// Save the active profile's fields under it and load `target`'s (its saved
/// ones, else its defaults). False when unknown or already active.
pub fn switch(tc: &mut Map<String, Value>, target: &str) -> bool {
    let current = active(&Value::Object(tc.clone()));
    if current == target || !PROFILES.iter().any(|p| p.0 == target) {
        return false;
    }
    let snap = |src: &Map<String, Value>, base: &Value| -> Map<String, Value> {
        FIELDS
            .iter()
            .map(|f| (f.to_string(), src.get(*f).unwrap_or(&base[*f]).clone()))
            .collect()
    };
    let mut saved = tc
        .get("profiles")
        .and_then(|p| p["saved"].as_object())
        .cloned()
        .unwrap_or_default();
    saved.insert(current.into(), snap(tc, &defaults(current)).into());
    let next = match saved.get(target).and_then(Value::as_object) {
        Some(s) => snap(s, &defaults(target)),
        None => snap(&Map::new(), &defaults(target)),
    };
    tc.extend(next);
    tc.insert("profiles".into(), json!({"active": target, "saved": saved}));
    true
}

/// FUNSR MINI settings as `D` commands: stroke length `DAG`, speed `DKP`,
/// start position `DSP`; only the values that are set.
pub fn funsr_commands(f: &Value) -> String {
    let mut out = String::new();
    if let Some(v) = f["dag"].as_f64() {
        out += &format!("DAG{:.2}\n", v.clamp(1.0, 3.0));
    }
    if let Some(v) = f["dkp"].as_f64() {
        out += &format!("DKP{:.1}\n", v.clamp(0.5, 5.0));
    }
    if let Some(v @ 1..=3) = f["dsp"].as_u64() {
        out += &format!("DSP{v}\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_saves_the_current_and_loads_defaults_then_saved() {
        let mut tc = json!({"transport": "websocket", "wsUrl": "ws://mirabot.local:8888",
            "precision": 4, "defaultOffset": 40})
        .as_object()
        .unwrap()
        .clone();
        assert_eq!(active(&Value::Object(tc.clone())), "mirabot");
        assert!(switch(&mut tc, "funsr-mini"));
        assert_eq!(tc["transport"], "serial");
        assert_eq!(tc["axes"]["L0"]["max"], 95);
        assert_eq!(tc["axes"]["R1"]["enabled"], false);
        assert_eq!(tc["defaultOffset"], Value::Null);
        assert!(!switch(&mut tc, "funsr-mini"));
        assert!(!switch(&mut tc, "nope"));
        assert!(switch(&mut tc, "mirabot"));
        assert_eq!(tc["wsUrl"], "ws://mirabot.local:8888");
        assert_eq!(tc["defaultOffset"], 40);
        assert_eq!(tc["profiles"]["saved"]["funsr-mini"]["transport"], "serial");
    }

    #[test]
    fn funsr_d_commands() {
        assert_eq!(
            funsr_commands(&json!({"dag": 1.534, "dkp": 9, "dsp": 1})),
            "DAG1.53\nDKP5.0\nDSP1\n"
        );
        assert_eq!(funsr_commands(&json!({"dsp": 4})), "");
        assert_eq!(funsr_commands(&Value::Null), "");
    }
}
