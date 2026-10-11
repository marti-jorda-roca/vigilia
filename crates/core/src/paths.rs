//! Where Vigilia keeps its files.
//!
//! | | macOS | Linux | Windows |
//! |---|---|---|---|
//! | config | `~/Library/Application Support/vigilia/config.toml` | `~/.config/vigilia/config.toml` | `%APPDATA%\vigilia\config.toml` |
//! | journal, lock, logs | `~/Library/Application Support/vigilia/` | `~/.local/state/vigilia/` | `%LOCALAPPDATA%\vigilia\` |
//! | socket | `~/Library/Application Support/vigilia/vigilia.sock` | `$XDG_RUNTIME_DIR/vigilia.sock` | (named pipe, later) |
//!
//! `VIGILIA_HOME` puts everything in one directory instead (tests, dev builds).

use std::io;
use std::path::{Path, PathBuf};

pub const HOME_ENV: &str = "VIGILIA_HOME";
const APP: &str = "vigilia";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub config: PathBuf,
    pub data_dir: PathBuf,
    pub socket: PathBuf,
    pub journal: PathBuf,
    pub lock: PathBuf,
    pub log_dir: PathBuf,
}

impl Paths {
    /// Paths for the current user, honouring `VIGILIA_HOME`.
    pub fn for_user() -> io::Result<Self> {
        if let Some(home) = std::env::var_os(HOME_ENV).filter(|h| !h.is_empty()) {
            return Ok(Self::in_dir(PathBuf::from(home)));
        }
        let base = directories::BaseDirs::new()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home directory"))?;
        let config_dir = base.config_dir().join(APP);
        let data_dir = base
            .state_dir()
            .unwrap_or_else(|| base.data_local_dir())
            .join(APP);
        let socket = match base.runtime_dir() {
            Some(runtime) if cfg!(target_os = "linux") => runtime.join("vigilia.sock"),
            _ => data_dir.join("vigilia.sock"),
        };
        Ok(Self {
            config: config_dir.join("config.toml"),
            socket,
            journal: data_dir.join("journal.jsonl"),
            lock: data_dir.join("vigilia.lock"),
            log_dir: data_dir.join("logs"),
            data_dir,
        })
    }

    /// Everything inside `dir`.
    pub fn in_dir(dir: PathBuf) -> Self {
        Self {
            config: dir.join("config.toml"),
            socket: dir.join("vigilia.sock"),
            journal: dir.join("journal.jsonl"),
            lock: dir.join("vigilia.lock"),
            log_dir: dir.join("logs"),
            data_dir: dir,
        }
    }

    /// Creates the data and log directories, owner-only on Unix.
    pub fn create_dirs(&self) -> io::Result<()> {
        create_private_dir(&self.data_dir)?;
        create_private_dir(&self.log_dir)?;
        if let Some(socket_dir) = self.socket.parent() {
            create_private_dir(socket_dir)?;
        }
        Ok(())
    }
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_dir_keeps_everything_together() {
        let p = Paths::in_dir(PathBuf::from("/tmp/v"));
        assert_eq!(p.socket, PathBuf::from("/tmp/v/vigilia.sock"));
        assert_eq!(p.journal, PathBuf::from("/tmp/v/journal.jsonl"));
        assert_eq!(p.config, PathBuf::from("/tmp/v/config.toml"));
    }

    #[cfg(unix)]
    #[test]
    fn dirs_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::in_dir(tmp.path().join("home"));
        p.create_dirs().unwrap();
        let mode = std::fs::metadata(&p.data_dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_uses_application_support() {
        // Only meaningful without the override.
        if std::env::var_os(HOME_ENV).is_some() {
            return;
        }
        let p = Paths::for_user().unwrap();
        assert!(
            p.data_dir.ends_with("Library/Application Support/vigilia"),
            "{p:?}"
        );
        assert_eq!(p.socket, p.data_dir.join("vigilia.sock"));
    }
}
