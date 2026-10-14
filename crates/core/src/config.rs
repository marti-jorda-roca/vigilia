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
    pub rules: RulesConfig,
    pub log: LogConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RulesConfig {
    /// End the session when the battery drops below this percent.
    pub battery_floor_percent: u8,
    /// End the session when the thermal level is critical.
    pub end_at_critical_heat: bool,
}

impl Default for RulesConfig {
    fn default() -> Self {
        Self {
            battery_floor_percent: 10,
            end_at_critical_heat: true,
        }
    }
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
        Duration::from_secs(self.idle_exit_minutes.saturating_mul(60))
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
    #[error("could not write {path}: {source}")]
    Write { path: String, source: io::Error },
    #[error("invalid config in {path}: {source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },
    #[error("invalid config in {path}: {source}")]
    Edit {
        path: String,
        source: toml_edit::TomlError,
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

    /// Saves only the `[rules]` keys to `path`, preserving comments and unknown keys.
    /// The file is written atomically (temp file in the same directory, then rename).
    pub fn save_rules(path: &Path, rules: &RulesConfig) -> Result<(), ConfigError> {
        let display = || path.display().to_string();
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(source) => {
                return Err(ConfigError::Read {
                    path: display(),
                    source,
                });
            }
        };
        let mut doc: toml_edit::DocumentMut = text.parse().map_err(|source| ConfigError::Edit {
            path: display(),
            source,
        })?;
        if !doc.get("rules").is_some_and(toml_edit::Item::is_table_like) {
            doc["rules"] = toml_edit::table();
        }
        doc["rules"]["battery_floor_percent"] =
            toml_edit::value(i64::from(rules.battery_floor_percent));
        doc["rules"]["end_at_critical_heat"] = toml_edit::value(rules.end_at_critical_heat);

        let write_error = |source| ConfigError::Write {
            path: display(),
            source,
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(write_error)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, doc.to_string()).map_err(write_error)?;
        std::fs::rename(&tmp, path).map_err(write_error)
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
    fn save_rules_preserves_comments_and_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "# keep me\n[energy]\nidle_exit_minutes = 1 # and me\n\n[unknown]\nx = 1\n\n[rules]\n# floor comment\nbattery_floor_percent = 10\n",
        )
        .unwrap();
        let rules = RulesConfig {
            battery_floor_percent: 25,
            end_at_critical_heat: false,
        };
        Config::save_rules(&path, &rules).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# keep me"), "{text}");
        assert!(text.contains("idle_exit_minutes = 1"), "{text}");
        assert!(text.contains("[unknown]"), "{text}");
        assert!(text.contains("x = 1"), "{text}");
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.rules, rules);
        assert_eq!(loaded.energy.idle_exit_minutes, 1);
    }

    #[test]
    fn save_rules_creates_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        Config::save_rules(&path, &RulesConfig::default()).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.rules, RulesConfig::default());
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
