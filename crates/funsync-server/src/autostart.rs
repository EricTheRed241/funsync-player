//! `funsync-server autostart install|uninstall|status [serve flags…]`: a
//! LaunchAgent (`com.funsync.rust`) that starts the server at login and
//! again whenever it stops (deploy/macos/app-autostart.sh does this for the
//! Electron app). The server writes its own log (crate::log); launchd keeps
//! only stderr, in `~/Library/Logs/funsync-rust/launchd.log`.

use std::path::{Path, PathBuf};
use std::process::Command;

pub const LABEL: &str = "com.funsync.rust";

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".into())
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The LaunchAgent for `bin serve <args>`.
pub fn plist(bin: &Path, args: &[String], log_dir: &Path) -> String {
    let argv: String = std::iter::once(bin.display().to_string())
        .chain(std::iter::once("serve".to_string()))
        .chain(args.iter().cloned())
        .map(|a| format!("    <string>{}</string>\n", esc(&a)))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{argv}  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key><string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>StandardOutPath</key><string>/dev/null</string>
  <key>StandardErrorPath</key><string>{}</string>
</dict>
</plist>
"#,
        esc(&log_dir.join("launchd.log").display().to_string())
    )
}

fn launchctl(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("launchctl").args(args).output()
}

pub fn run(action: &str, serve_args: &[String]) -> Result<(), String> {
    let home = home()?;
    let plist_path = home.join(format!("Library/LaunchAgents/{LABEL}.plist"));
    let domain = format!(
        "gui/{}",
        String::from_utf8_lossy(
            &Command::new("id")
                .arg("-u")
                .output()
                .map_err(|e| e.to_string())?
                .stdout
        )
        .trim()
    );
    let target = format!("{domain}/{LABEL}");
    match action {
        "install" => {
            let bin = std::env::current_exe().map_err(|e| e.to_string())?;
            let log_dir = home.join("Library/Logs/funsync-rust");
            std::fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;
            std::fs::create_dir_all(plist_path.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&plist_path, plist(&bin, serve_args, &log_dir))
                .map_err(|e| e.to_string())?;
            let _ = launchctl(&["bootout", &target]);
            // bootout returns before the old process is gone.
            for _ in 0..30 {
                if !launchctl(&["print", &target]).is_ok_and(|o| o.status.success()) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            let out = launchctl(&["bootstrap", &domain, &plist_path.display().to_string()])
                .map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
            }
            println!(
                "installed {}: the server runs now and at every login ({} serve {})",
                plist_path.display(),
                bin.display(),
                serve_args.join(" ")
            );
        }
        "uninstall" => {
            let _ = launchctl(&["bootout", &target]);
            let _ = std::fs::remove_file(&plist_path);
            println!("removed {LABEL}");
        }
        "status" => match launchctl(&["print", &target]) {
            Ok(o) if o.status.success() => {
                for line in String::from_utf8_lossy(&o.stdout).lines() {
                    let t = line.trim_start();
                    if ["state =", "pid =", "last exit code =", "runs ="]
                        .iter()
                        .any(|k| t.starts_with(k))
                    {
                        println!("{t}");
                    }
                }
            }
            _ => println!("{LABEL} not loaded"),
        },
        other => {
            return Err(format!(
                "autostart: unknown action {other} (install, uninstall, status)"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_runs_serve_with_the_given_flags() {
        let p = plist(
            Path::new("/r/funsync-server"),
            &["--lan".into(), "--data-dir".into(), "/a&b".into()],
            Path::new("/logs"),
        );
        assert!(p.contains("<string>/r/funsync-server</string>\n    <string>serve</string>\n    <string>--lan</string>"));
        assert!(p.contains("<string>/a&amp;b</string>"));
        assert!(p.contains("<key>KeepAlive</key><true/>"));
        assert!(p.contains("/logs/launchd.log"));
    }
}
