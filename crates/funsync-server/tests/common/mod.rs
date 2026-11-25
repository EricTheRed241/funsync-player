//! Shared e2e helpers: import an old `config.json` into a temp data folder
//! and start the server on a free port.

// Each test binary uses a part of this.
#![allow(dead_code)]

use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

pub const BIN: &str = env!("CARGO_BIN_EXE_funsync-server");

/// The server process, killed with the test.
pub struct Kill(pub Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// `funsync-server import` of `cfg` (the old app's `config.json` shape)
/// into a new temp data folder.
pub fn import(cfg: &serde_json::Value) -> tempfile::TempDir {
    let (old, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(old.path().join("config.json"), cfg.to_string()).unwrap();
    let ok = Command::new(BIN)
        .args(["import", "--from"])
        .arg(old.path())
        .arg("--data-dir")
        .arg(data.path())
        .status()
        .unwrap();
    assert!(ok.success());
    data
}

/// Serve `data` with `env` set; returns once the server answers. The port
/// is picked free, but a test running alongside may take it before the
/// server binds it: a server that exits is started again on another port.
pub fn start(data: &Path, env: &[(&str, &str)]) -> (Kill, u16) {
    for _ in 0..5 {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut srv = Kill(
            Command::new(BIN)
                .args(["--port", &port.to_string(), "--data-dir"])
                .arg(data)
                // Never the user's ~/Library/Logs/funsync-rust/server.log.
                .env("FUNSYNC_RUST_LOG", data.join("server.log"))
                // Library changes only where a test asks for them (`env`).
                .env("FUNSYNC_OWNS_LIBRARY", "0")
                // Never the user's userData (config.json, access PIN, sites).
                .env("FUNSYNC_USER_DATA", data)
                .envs(env.iter().copied())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return (srv, port);
            }
            if srv.0.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    panic!("the server never started");
}
