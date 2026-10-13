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
/// done, for the CLI to print.
#[cfg(target_os = "macos")]
pub fn install(program: &Path, user: &str, paths: &Paths) -> Result<Vec<String>, String> {
    let dir = launch_agents_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut done = Vec::new();
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

/// Removes the `LaunchAgents` (including any backstop) and the sudoers rule.
#[cfg(target_os = "macos")]
pub fn uninstall() -> Result<Vec<String>, String> {
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
pub fn uninstall() -> Result<Vec<String>, String> {
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
    fn sudoers_rule_covers_only_disablesleep() {
        let rule = sudoers_rule("marti");
        assert!(rule.contains("marti ALL=(root) NOPASSWD:"));
        assert!(rule.contains("/usr/bin/pmset -a disablesleep 1"));
        assert!(rule.contains("/usr/bin/pmset -a disablesleep 0"));
        assert_eq!(rule.matches("pmset").count(), 2);
    }
}
