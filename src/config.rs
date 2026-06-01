//! Environmental (operational) configuration: the TOML file that says *where and how this instance
//! runs* — the event-log and graph paths, and (later) endpoints and bind addresses. It is distinct
//! from behavioral config, which lives in the log as `ConfigSet` events (spec §Initialization).
//!
//! Because it carries the database paths, this file is the instance selector: two configs with
//! different paths are two independent agents. Relative paths resolve against the config file's own
//! directory, so an instance is relocatable by moving its directory.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The parsed environmental config. Unknown sections (e.g. `[model]`, wired in Stage 5) are
/// ignored, so the file can carry settings later stages will consume.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct EnvConfig {
    pub storage: StorageConfig,
}

/// Where this instance's two databases live. The event log is the source of truth; the graph is a
/// rebuildable projection.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub event_log: PathBuf,
    pub graph: PathBuf,
}

impl Default for StorageConfig {
    fn default() -> Self {
        StorageConfig {
            event_log: PathBuf::from("zuihitsu.events.sqlite"),
            graph: PathBuf::from("zuihitsu.graph.sqlite"),
        }
    }
}

impl EnvConfig {
    /// Load config from `path`, resolving relative storage paths against the file's directory. A
    /// missing file yields defaults (resolved against the file's intended directory), so a bare
    /// instance still has somewhere to put its databases.
    pub fn load(path: &Path) -> Result<EnvConfig, ConfigError> {
        let mut config = match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(ConfigError::Parse)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => EnvConfig::default(),
            Err(error) => return Err(ConfigError::Io(error)),
        };
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        config.storage.event_log = base.join(&config.storage.event_log);
        config.storage.graph = base.join(&config.storage.graph);
        Ok(config)
    }
}

/// A failure loading the environmental config.
#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(toml::de::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(error) => write!(f, "could not read the config file: {error}"),
            ConfigError::Parse(error) => write!(f, "invalid config TOML: {error}"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Io(error) => Some(error),
            ConfigError::Parse(error) => Some(error),
        }
    }
}
