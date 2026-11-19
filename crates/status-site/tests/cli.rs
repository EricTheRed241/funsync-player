//! End to end through the binary: serve on a free loopback port; the page and
//! /api/status (every site, in order) over raw HTTP; --help.

use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_status-site");

/// The server process, killed when dropped.
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(user_data: &std::path::Path) -> (Server, u16) {
    let mut child = Command::new(BIN)
        .args(["--port", "0", "--user-data"])
        .arg(user_data)
        .env_remove("FUNSYNC_USER_DATA")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let server = Server(child);
    let port = line
        .trim()
        .strip_prefix("Status: http://127.0.0.1:")
        .and_then(|rest| rest.strip_suffix('/'))
        .unwrap_or_else(|| panic!("unexpected banner {line:?}"))
        .parse()
        .unwrap();
    (server, port)
}

fn get(port: u16, path: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, body.to_string())
}

#[test]
fn serves_the_page_and_every_sites_status() {
    let dir = std::env::temp_dir().join(format!("status-site-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (_server, port) = start(&dir);

    let (status, page) = get(port, "/");
    assert_eq!(status, 200);
    assert!(page.contains("FunSync status"));

    // What's up on this machine varies; the list and each entry's shape don't.
    let (status, body) = get(port, "/api/status");
    assert_eq!(status, 200);
    let v: Value = serde_json::from_str(&body).unwrap();
    let names: Vec<&str> = v["sites"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["Backend", "Web remote", "Swipe deck", "Creators", "Posts"]
    );
    for s in v["sites"].as_array().unwrap() {
        let up = s["up"].as_bool().unwrap();
        assert_eq!(up, s["error"].is_null(), "{s}");
    }
    assert!(v["checkedAt"].as_u64().unwrap() > 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn help_prints_usage() {
    let out = Command::new(BIN).arg("--help").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("npm run status:site"));
}
