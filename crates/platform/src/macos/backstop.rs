//! The backstop `LaunchAgent`: runs `vigilia recover` every 5 minutes, but only exists while
//! a session runs.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use vigilia_core::UndoRecord;

use crate::{BACKSTOP_LABEL, PlatformError, Result};

pub fn plist_path() -> PathBuf {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("~"), PathBuf::from);
    home.join("Library/LaunchAgents")
        .join(format!("{BACKSTOP_LABEL}.plist"))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The backstop plist: `vigilia recover` on a fixed interval.
pub fn backstop_plist(program: &Path, interval: Duration) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \t<key>Label</key>\n\t<string>{BACKSTOP_LABEL}</string>\n\
         \t<key>ProgramArguments</key>\n\t<array>\n\
         \t\t<string>{}</string>\n\t\t<string>recover</string>\n\t</array>\n\
         \t<key>StartInterval</key>\n\t<integer>{}</integer>\n\
         </dict>\n</plist>\n",
        xml_escape(&program.display().to_string()),
        interval.as_secs().max(1),
    )
}

fn launchctl(args: &[&str]) -> Result<()> {
    let output = Command::new("/bin/launchctl")
        .args(args)
        .output()
        .map_err(|e| PlatformError::Os {
            op: "launchctl",
            detail: e.to_string(),
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(PlatformError::Os {
            op: "launchctl",
            detail: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

pub fn register(program: &Path, interval: Duration) -> Result<UndoRecord> {
    let path = plist_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| PlatformError::Os {
            op: "create LaunchAgents dir",
            detail: e.to_string(),
        })?;
    }
    std::fs::write(&path, backstop_plist(program, interval)).map_err(|e| PlatformError::Os {
        op: "write backstop plist",
        detail: e.to_string(),
    })?;
    // Reload if a stale job is already there; a failed unload just means none was loaded.
    let path_str = path.display().to_string();
    let _ = launchctl(&["unload", &path_str]);
    if let Err(e) = launchctl(&["load", "-w", &path_str]) {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(UndoRecord::Backstop {
        label: BACKSTOP_LABEL.into(),
        plist: path_str,
    })
}

pub fn remove() -> Result<()> {
    let path = plist_path();
    let path_str = path.display().to_string();
    if path.exists() {
        // Not loaded is fine; the file removal is what guarantees it stays gone.
        let _ = launchctl(&["unload", &path_str]);
        std::fs::remove_file(&path).map_err(|e| PlatformError::Os {
            op: "remove backstop plist",
            detail: e.to_string(),
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_runs_recover_every_five_minutes() {
        let text = backstop_plist(Path::new("/opt/vigilia"), Duration::from_secs(300));
        assert!(text.contains(BACKSTOP_LABEL));
        assert!(text.contains("<string>recover</string>"));
        assert!(text.contains("<integer>300</integer>"));
        assert!(text.contains("/opt/vigilia"));
    }

    #[test]
    fn plist_escapes_the_program_path() {
        let text = backstop_plist(Path::new("/tmp/a&b/vigilia"), Duration::from_secs(60));
        assert!(text.contains("/tmp/a&amp;b/vigilia"));
    }
}
