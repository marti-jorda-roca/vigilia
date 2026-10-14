//! `vigilia install` / `uninstall` (macOS): the socket-activated service `LaunchAgent`, the
//! run-once-at-login `recover` job, and the narrow sudoers rule for `pmset disablesleep`.
//!
//! The generators are pure so their output is testable; only `install`/`uninstall` touch
//! the machine, and the CLI shows the sudoers rule and asks before calling them.

use std::path::Path;
#[cfg(target_os = "macos")]
use std::path::PathBuf;

use vigilia_core::Paths;

pub const SERVICE_LABEL: &str = "com.vigilia.service";
pub const RECOVER_LABEL: &str = "com.vigilia.recover";
pub const SUDOERS_PATH: &str = "/etc/sudoers.d/vigilia";

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn plist_header() -> &'static str {
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
     <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
     \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
     <plist version=\"1.0\">\n<dict>\n"
}

/// The socket-activated service: launchd owns the socket and starts `vigilia daemon` on
/// the first connection. The `Listeners` name must match `activation.rs`.
pub fn service_plist(program: &Path, socket: &Path) -> String {
    format!(
        "{}<key>Label</key>\n<string>{SERVICE_LABEL}</string>\n\
         <key>ProgramArguments</key>\n<array>\n<string>{}</string>\n<string>daemon</string>\n</array>\n\
         <key>Sockets</key>\n<dict>\n<key>Listeners</key>\n<dict>\n\
         <key>SockPathName</key>\n<string>{}</string>\n\
         <key>SockFamily</key>\n<string>Unix</string>\n</dict>\n</dict>\n\
         <key>ProcessType</key>\n<string>Background</string>\n\
         </dict>\n</plist>\n",
        plist_header(),
        xml_escape(&program.display().to_string()),
        xml_escape(&socket.display().to_string()),
    )
}

/// Runs `vigilia recover` once at login; it removes itself via the backstop cleanup when
/// there is nothing to do.
pub fn recover_plist(program: &Path) -> String {
    format!(
        "{}<key>Label</key>\n<string>{RECOVER_LABEL}</string>\n\
         <key>ProgramArguments</key>\n<array>\n<string>{}</string>\n<string>recover</string>\n</array>\n\
         <key>RunAtLoad</key>\n<true/>\n\
         </dict>\n</plist>\n",
        plist_header(),
        xml_escape(&program.display().to_string()),
    )
}

/// Exactly the two `pmset disablesleep` commands, nothing else.
pub fn sudoers_rule(user: &str) -> String {
    format!(
        "# Vigilia: keep the Mac awake with the lid closed (vigilia install)\n\
         {user} ALL=(root) NOPASSWD: /usr/bin/pmset -a disablesleep 1, /usr/bin/pmset -a disablesleep 0\n"
    )
}

#[cfg(target_os = "macos")]
fn launch_agents_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("~"), PathBuf::from);
    home.join("Library/LaunchAgents")
}

#[cfg(target_os = "macos")]
fn launchctl(args: &[&str]) -> Result<(), String> {
    let output = std::process::Command::new("/bin/launchctl")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// Installs the `LaunchAgents` and the sudoers rule (via `sudo install`). Returns what was
/// done, for the CLI to print. A service already running is stopped first: launchd takes
/// over its socket path, which would leave it running with no way to reach it.
#[cfg(target_os = "macos")]
pub fn install(program: &Path, user: &str, paths: &Paths) -> Result<Vec<String>, String> {
    let dir = launch_agents_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut done = Vec::new();
    done.extend(stop_running_service(paths)?);
    for (label, contents) in [
        (SERVICE_LABEL, service_plist(program, &paths.socket)),
        (RECOVER_LABEL, recover_plist(program)),
    ] {
        let path = dir.join(format!("{label}.plist"));
        std::fs::write(&path, contents).map_err(|e| e.to_string())?;
        let path_str = path.display().to_string();
        let _ = launchctl(&["unload", &path_str]);
        launchctl(&["load", "-w", &path_str])?;
        done.push(format!("installed LaunchAgent {label}"));
    }
    let staged = paths.data_dir.join("sudoers.vigilia");
    std::fs::write(&staged, sudoers_rule(user)).map_err(|e| e.to_string())?;
    let status = std::process::Command::new("/usr/bin/sudo")
        .args([
            "/usr/bin/install",
            "-m",
            "0440",
            "-o",
            "root",
            "-g",
            "wheel",
            staged.to_str().ok_or("non-UTF8 data dir")?,
            SUDOERS_PATH,
        ])
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("installing the sudoers rule failed".into());
    }
    done.push(format!("installed sudoers rule {SUDOERS_PATH}"));
    Ok(done)
}

/// Whether `ps -o comm=` output names the vigilia binary, so a stale pid in the lock file
/// that now belongs to another program is never signalled.
pub fn is_vigilia_command(comm: &str) -> bool {
    Path::new(comm.trim()).file_name() == Some(std::ffi::OsStr::new("vigilia"))
}

#[cfg(target_os = "macos")]
fn process_is_vigilia(pid: u32) -> bool {
    std::process::Command::new("/bin/ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .is_ok_and(|out| {
            out.status.success() && is_vigilia_command(&String::from_utf8_lossy(&out.stdout))
        })
}

/// Polls until the instance lock is free or `timeout` passes.
#[cfg(target_os = "macos")]
fn wait_for_lock_release(paths: &Paths, timeout: std::time::Duration) -> Result<bool, String> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if !crate::lock::is_held(&paths.lock).map_err(|e| e.to_string())? {
            return Ok(true);
        }
        if std::time::Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Stops a service that holds the instance lock: before `install`, which would orphan it,
/// and in `uninstall` once the `LaunchAgents` are gone, where one launchd is still shutting
/// down or one started outside launchd would keep its session running after the sudoers
/// rule is removed. SIGTERM lets it end the session and restore `disablesleep` itself;
/// SIGKILL is the fallback, and the next journal replay undoes what it left.
#[cfg(target_os = "macos")]
fn stop_running_service(paths: &Paths) -> Result<Option<String>, String> {
    use std::time::Duration;
    if wait_for_lock_release(paths, Duration::from_secs(2))? {
        return Ok(None);
    }
    let pid = crate::lock::holder_pid(&paths.lock)
        .ok_or("the vigilia lock is held but names no process; stop the service and try again")?;
    if !process_is_vigilia(pid) {
        return Err(format!(
            "pid {pid} holds the vigilia lock but is not vigilia; stop it and try again"
        ));
    }
    for (signal, wait) in [
        ("-TERM", Duration::from_secs(5)),
        ("-KILL", Duration::from_secs(2)),
    ] {
        let _ = std::process::Command::new("/bin/kill")
            .args([signal, &pid.to_string()])
            .status();
        if wait_for_lock_release(paths, wait)? {
            return Ok(Some(format!("stopped the running service (pid {pid})")));
        }
    }
    Err(format!("the vigilia service (pid {pid}) did not stop"))
}

/// Removes the `LaunchAgents` (including any backstop) and the sudoers rule. Before the
/// rule goes, it stops a service still running, replays and deletes the journal, and clears
/// a `disablesleep` still on: once the rule is gone nothing can clear the flag, so the Mac
/// would stay awake with the lid closed.
#[cfg(target_os = "macos")]
pub fn uninstall(paths: &Paths) -> Result<Vec<String>, String> {
    let dir = launch_agents_dir();
    let mut done = Vec::new();
    for label in [
        SERVICE_LABEL,
        RECOVER_LABEL,
        vigilia_platform::BACKSTOP_LABEL,
    ] {
        let path = dir.join(format!("{label}.plist"));
        if path.exists() {
            let path_str = path.display().to_string();
            let _ = launchctl(&["unload", &path_str]);
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            done.push(format!("removed LaunchAgent {label}"));
        }
    }
    done.extend(stop_running_service(paths)?);
    let outcome = crate::recover::recover_and_clear(paths, vigilia_platform::native())
        .map_err(|e| e.to_string())?;
    if outcome.already_running {
        return Err("a vigilia service started again while uninstalling; \
             run `vigilia uninstall` again"
            .into());
    }
    done.push(match outcome.failed {
        0 => format!("cleared the journal ({} actions undone)", outcome.undone),
        failed => format!(
            "cleared the journal ({} actions undone, {failed} could not be undone)",
            outcome.undone
        ),
    });
    if vigilia_platform::macos::sleep_disabled().map_err(|e| e.to_string())? {
        let status = std::process::Command::new("/usr/bin/sudo")
            .args(["/usr/bin/pmset", "-a", "disablesleep", "0"])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("clearing pmset disablesleep failed".into());
        }
        done.push("restored lid-closed sleep (pmset disablesleep was 1)".into());
    }
    let status = std::process::Command::new("/usr/bin/sudo")
        .args(["/bin/rm", "-f", SUDOERS_PATH])
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("removing the sudoers rule failed".into());
    }
    done.push(format!("removed sudoers rule {SUDOERS_PATH}"));
    Ok(done)
}

#[cfg(not(target_os = "macos"))]
pub fn install(_program: &Path, _user: &str, _paths: &Paths) -> Result<Vec<String>, String> {
    Err("`vigilia install` is only supported on macOS for now".into())
}

#[cfg(not(target_os = "macos"))]
pub fn uninstall(_paths: &Paths) -> Result<Vec<String>, String> {
    Err("`vigilia uninstall` is only supported on macOS for now".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_plist_is_socket_activated() {
        let text = service_plist(
            Path::new("/usr/local/bin/vigilia"),
            Path::new("/tmp/vigilia.sock"),
        );
        assert!(text.contains(SERVICE_LABEL));
        assert!(text.contains("<key>Listeners</key>"));
        assert!(text.contains("/tmp/vigilia.sock"));
        assert!(text.contains("<string>daemon</string>"));
    }

    #[test]
    fn recover_plist_runs_once_at_load() {
        let text = recover_plist(Path::new("/usr/local/bin/vigilia"));
        assert!(text.contains(RECOVER_LABEL));
        assert!(text.contains("<string>recover</string>"));
        assert!(text.contains("<key>RunAtLoad</key>"));
    }

    #[test]
    fn only_the_vigilia_binary_counts_as_the_service() {
        assert!(is_vigilia_command("/Users/me/.cargo/bin/vigilia\n"));
        assert!(is_vigilia_command("vigilia"));
        assert!(!is_vigilia_command("/usr/bin/vigilia-other"));
        assert!(!is_vigilia_command("/bin/zsh"));
        assert!(!is_vigilia_command(""));
    }

    #[test]
    fn sudoers_rule_covers_only_disablesleep() {
        let rule = sudoers_rule("marti");
        assert!(rule.contains("marti ALL=(root) NOPASSWD:"));
        assert!(rule.contains("/usr/bin/pmset -a disablesleep 1"));
        assert!(rule.contains("/usr/bin/pmset -a disablesleep 0"));
        assert_eq!(rule.matches("pmset").count(), 2);
    }
}
