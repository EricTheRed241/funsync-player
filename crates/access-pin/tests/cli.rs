//! End to end through the binary against a temp userData.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use video_tags::access::{self, AccessPin};

const BIN: &str = env!("CARGO_BIN_EXE_access-pin");

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("access-pin-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Run with `--user-data dir` → (exit code, stdout, stderr).
fn run(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(BIN)
        .args(args)
        .arg("--user-data")
        .arg(dir)
        .env_remove("FUNSYNC_USER_DATA")
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn load(dir: &Path) -> AccessPin {
    AccessPin::load(dir).unwrap().unwrap()
}

#[test]
fn set_status_and_clear() {
    let tmp = TempDir::new("set");
    let dir = &tmp.0;
    let file = access::file_path(dir);

    let (code, out, _) = run(dir, &["status"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("No access PIN"), "{out}");

    let (code, out, err) = run(dir, &["set", "0428"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(out.contains("has to log in again"), "{out}");
    assert!(!out.contains("0428"));
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let first = load(dir);
    assert_eq!(first.pin, "0428");
    assert!(first.changed_at.ends_with('Z'));
    let raw: serde_json::Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(raw["version"], 1);
    let secret = raw["secret"].as_str().unwrap();
    assert!(secret.len() == 64 && secret.bytes().all(|b| b.is_ascii_hexdigit()));
    // No temp file left behind.
    assert_eq!(fs::read_dir(dir).unwrap().count(), 1);

    let (code, out, _) = run(dir, &["status"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("Access PIN set (changed "), "{out}");
    assert!(!out.contains("0428"), "{out}");
    let (code, out, _) = run(dir, &["status", "--show"]);
    assert_eq!(code, 0);
    assert!(out.contains("PIN: 0428\n"), "{out}");

    // The same PIN again: a new secret, so every login so far stops working.
    let token = first.token(access::now_secs());
    assert_eq!(run(dir, &["set", "0428"]).0, 0);
    let second = load(dir);
    assert!(!second.verify(&token, access::now_secs()));
    assert!(second.verify(&second.token(access::now_secs()), access::now_secs()));

    // A bad PIN changes nothing.
    for bad in ["123", "1234567890123", "12a4", ""] {
        let (code, _, err) = run(dir, &["set", bad]);
        assert_eq!(code, 1, "{bad}");
        assert!(err.contains("4 to 12 digits"), "{err}");
    }
    assert_eq!(load(dir), second);

    // Even a file left with other permissions comes back 0600.
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(run(dir, &["set", "123456789012"]).0, 0);
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let (code, out, _) = run(dir, &["clear"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("Access PIN removed"), "{out}");
    assert!(!file.exists());
    let (code, out, _) = run(dir, &["clear"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("No access PIN was set"), "{out}");
}

#[test]
fn a_damaged_file_is_an_error_until_set_again() {
    let tmp = TempDir::new("damaged");
    let dir = &tmp.0;
    fs::write(access::file_path(dir), "{\"version\":1}").unwrap();
    let (code, out, err) = run(dir, &["status"]);
    assert_eq!((code, out.as_str()), (1, ""));
    assert!(
        err.contains("access-pin.json") && err.contains("refused"),
        "{err}"
    );
    assert_eq!(run(dir, &["set", "9999"]).0, 0);
    assert_eq!(load(dir).pin, "9999");
}

#[test]
fn help_and_errors() {
    let out = Command::new(BIN).arg("--help").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("npm run access:pin -- set <pin>"));

    let tmp = TempDir::new("errors");
    for args in [&[][..], &["lock"], &["set"], &["clear", "--show"]] {
        let (code, out, err) = run(&tmp.0, args);
        assert_eq!((code, out.as_str()), (1, ""), "{args:?}");
        assert!(err.starts_with("access-pin: "), "{err}");
    }
    let (code, _, err) = run(&tmp.0.join("missing"), &["set", "1234"]);
    assert_eq!(code, 1);
    assert!(err.contains("is not a folder"), "{err}");
    assert_eq!(fs::read_dir(&tmp.0).unwrap().count(), 0);
}
