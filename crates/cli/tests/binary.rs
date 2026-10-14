//! The installed binary end to end, on macOS. Tests that take a real power assertion only run
//! on demand:
//!
//! ```sh
//! cargo test -p vigilia --test binary -- --ignored
//! ```
#![cfg(target_os = "macos")]

use std::path::Path;
use std::process::{Command, Output};

use vigilia_protocol::{ConfigResult, SessionResult, SessionState};

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
fn config_get_and_set() {
    // No session, so no power assertion: this one runs with the rest of the suite.
    let home = tempfile::tempdir().unwrap();
    let _kill = KillService(home.path());

    let got_out = vigilia(home.path(), &["config", "get", "--json"]);
    assert!(got_out.status.success(), "{got_out:?}");
    let got: ConfigResult = serde_json::from_slice(&got_out.stdout).unwrap();
    assert_eq!(got.config.rules.battery_floor_percent, 10);
    assert!(got.config.rules.end_at_critical_heat);

    let set_out = vigilia(
        home.path(),
        &[
            "config",
            "set",
            "rules.battery_floor_percent",
            "20",
            "--json",
        ],
    );
    assert!(set_out.status.success(), "{set_out:?}");
    let set: ConfigResult = serde_json::from_slice(&set_out.stdout).unwrap();
    assert_eq!(set.config.rules.battery_floor_percent, 20);

    let got_out = vigilia(home.path(), &["config", "get", "--json"]);
    let got: ConfigResult = serde_json::from_slice(&got_out.stdout).unwrap();
    assert_eq!(got.config.rules.battery_floor_percent, 20);

    let bad = vigilia(
        home.path(),
        &["config", "set", "rules.battery_floor_percent", "101"],
    );
    assert_eq!(bad.status.code(), Some(1));
}

#[test]
fn sudoers_refuses_without_root() {
    let home = tempfile::tempdir().unwrap();
    let out = vigilia(home.path(), &["sudoers", "install", "--user", "nobody"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("must be run as root"), "{stderr}");
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
