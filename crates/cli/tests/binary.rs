//! The installed binary end to end: it spawns a real service that takes a real power assertion,
//! so it only runs on demand, on macOS:
//!
//! ```sh
//! cargo test -p vigilia --test binary -- --ignored
//! ```
#![cfg(target_os = "macos")]

use std::path::Path;
use std::process::{Command, Output};

use vigilia_protocol::{SessionResult, SessionState};

fn vigilia(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vigilia"))
        .args(args)
        .env("VIGILIA_HOME", home)
        .output()
        .unwrap()
}

fn session(out: &Output) -> SessionResult {
    assert!(out.status.success(), "{out:?}");
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {out:?}"))
}

/// Stops the service the first command spawned; otherwise it lingers for its idle timeout.
struct KillService<'a>(&'a Path);

impl Drop for KillService<'_> {
    fn drop(&mut self) {
        if let Ok(pid) = std::fs::read_to_string(self.0.join("vigilia.lock")) {
            let _ = Command::new("kill").arg(pid.trim()).status();
        }
    }
}

#[test]
#[ignore = "takes a real idle-sleep assertion"]
fn start_status_stop() {
    let home = tempfile::tempdir().unwrap();
    let _kill = KillService(home.path());

    let started = session(&vigilia(home.path(), &["start", "1m", "--json"]));
    assert_eq!(started.session.unwrap().state, SessionState::Active);

    let again = vigilia(home.path(), &["start", "1m"]);
    assert_eq!(again.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&again.stderr);
    assert!(stderr.contains("session_active"), "{stderr}");

    let status = session(&vigilia(home.path(), &["status", "--json"]));
    assert_eq!(status.session.unwrap().state, SessionState::Active);

    let stopped = session(&vigilia(home.path(), &["stop", "--json"]));
    assert_eq!(stopped.session.unwrap().state, SessionState::Ended);
    let stopped_again = session(&vigilia(home.path(), &["stop", "--json"]));
    assert_eq!(stopped_again.session, None);

    let usage = vigilia(home.path(), &["start", "30"]);
    assert_eq!(usage.status.code(), Some(2));
}
