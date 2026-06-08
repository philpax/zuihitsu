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
    pub model: ModelConfig,
    pub embedding: EmbeddingConfig,
}

/// Where to reach the generation model, and how to sample from it. An empty `endpoint` means "not
/// configured". Each sampling field is optional: unset fields are simply not sent, so the serving
/// layer applies its own per-model default.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    pub endpoint: String,
    pub llm: String,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub min_p: Option<f32>,
    pub presence_penalty: Option<f32>,
    /// Override the serving layer's thinking default (`chat_template_kwargs.enable_thinking`).
    pub thinking: Option<bool>,
}

/// Where to reach the embedding model, and the dimensionality it produces.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct EmbeddingConfig {
    pub endpoint: String,
    pub model: String,
    pub dimensions: usize,
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
            ConfigError::Io(error) => write!(f, "config: could not read the file: {error}"),
            ConfigError::Parse(error) => write!(f, "config: invalid TOML: {error}"),
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

#[cfg(test)]
mod tests {
    //! Environmental-config loading: defaults when the file is absent, parsing when present, and
    //! relative storage paths resolved against the config file's own directory (spec §Initialization).
    use std::path::PathBuf;

    use super::EnvConfig;
    use crate::ids::MemoryId;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zuihitsu-cfg-{}", MemoryId::generate().0));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_yields_defaults_resolved_against_its_directory() {
        let dir = temp_dir();
        let path = dir.join("config.toml"); // does not exist
        let config = EnvConfig::load(&path).unwrap();

        assert_eq!(config.storage.event_log, dir.join("zuihitsu.events.sqlite"));
        assert_eq!(config.storage.graph, dir.join("zuihitsu.graph.sqlite"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_storage_and_resolves_relative_paths() {
        let dir = temp_dir();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[storage]\nevent_log = \"db/events.sqlite\"\ngraph = \"db/graph.sqlite\"\n",
        )
        .unwrap();

        let config = EnvConfig::load(&path).unwrap();
        assert_eq!(config.storage.event_log, dir.join("db/events.sqlite"));
        assert_eq!(config.storage.graph, dir.join("db/graph.sqlite"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_sections_are_ignored() {
        let dir = temp_dir();
        let path = dir.join("config.toml");
        // A [model] section (consumed by a later stage) must not break loading.
        std::fs::write(
            &path,
            "[model]\nendpoint = \"http://example/v1\"\nllm = \"some-model\"\n",
        )
        .unwrap();

        let config = EnvConfig::load(&path).unwrap();
        assert_eq!(config.storage.event_log, dir.join("zuihitsu.events.sqlite"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn malformed_toml_is_an_error() {
        let dir = temp_dir();
        let path = dir.join("config.toml");
        std::fs::write(&path, "this is not = = valid toml").unwrap();
        assert!(EnvConfig::load(&path).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
