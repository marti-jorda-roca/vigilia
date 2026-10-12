//! `config.toml`. Every field has a default, so a missing file or a partial one is fine, and
//! keys added by newer versions are ignored.

use std::io;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub energy: EnergyConfig,
    pub log: LogConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnergyConfig {
    /// The service exits after this long with no session and no subscribers.
    pub idle_exit_minutes: u64,
    /// How late the OS may fire the deadline timer, so it can group wake-ups.
    pub timer_leeway_secs: u64,
}

impl Default for EnergyConfig {
    fn default() -> Self {
        Self {
            idle_exit_minutes: 5,
            timer_leeway_secs: 1,
        }
    }
}

impl EnergyConfig {
    pub fn idle_exit(&self) -> Duration {
        Duration::from_secs(self.idle_exit_minutes * 60)
    }

    pub fn timer_leeway(&self) -> Duration {
        Duration::from_secs(self.timer_leeway_secs)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    /// A `tracing` filter, e.g. `info` or `vigilia_daemon=debug`.
    pub level: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read {path}: {source}")]
    Read { path: String, source: io::Error },
    #[error("invalid config in {path}: {source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },
}

impl Config {
    /// Loads the file at `path`, or the defaults if it doesn't exist.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|source| ConfigError::Parse {
                path: path.display().to_string(),
                source,
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(ConfigError::Read {
                path: path.display().to_string(),
                source,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load(&dir.path().join("config.toml")).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.energy.idle_exit(), Duration::from_secs(300));
        assert_eq!(config.log.level, "info");
    }

    #[test]
    fn partial_file_keeps_other_defaults_and_ignores_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[energy]\nidle_exit_minutes = 1\n\n[agents.auto_end]\nenabled = true\n",
        )
        .unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!(config.energy.idle_exit_minutes, 1);
        assert_eq!(config.energy.timer_leeway_secs, 1);
        assert_eq!(config.log, LogConfig::default());
    }

    #[test]
    fn bad_types_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[energy]\nidle_exit_minutes = \"soon\"\n").unwrap();
        assert!(matches!(
            Config::load(&path),
            Err(ConfigError::Parse { .. })
        ));
    }
}
