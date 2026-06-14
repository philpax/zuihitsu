//! Environmental (operational) configuration: the TOML file that says *where and how this instance
//! runs* — the event-log and graph paths, and (later) endpoints and bind addresses. It is distinct
//! from behavioral config, which lives in the log as `ConfigSet` events (spec §Initialization).
//!
//! Because it carries the database paths, this file is the instance selector: two configs with
//! different paths are two independent agents. Relative paths resolve against the config file's own
//! directory, so an instance is relocatable by moving its directory.

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize, Serializer};

use crate::mcp::McpServerConfig;

/// The parsed environmental config. Unknown sections (e.g. `[model]`, wired in Stage 5) are
/// ignored, so the file can carry settings later stages will consume.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct EnvConfig {
    pub storage: StorageConfig,
    pub model: ModelConfig,
    pub embedding: EmbeddingConfig,
    pub serving: ServingConfig,
    pub snapshots: SnapshotConfig,
    /// The MCP servers to connect (one `[mcp.<name>]` block each, spec §MCP server blocks). The table
    /// key is the `mcp.<name>.*` projection prefix, so it must be a valid Lua identifier — validated
    /// at load.
    #[serde(default)]
    pub mcp: BTreeMap<String, McpServerConfig>,
}

/// Where to reach the generation model, and how to sample from it. An empty `endpoint` means "not
/// configured". Each sampling field is optional: unset fields are simply not sent, so the serving
/// layer applies its own per-model default.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
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

/// How this instance serves its HTTP API (spec §Clients and the server boundary): the local address
/// the long-running server binds, and the per-surface API keys that authorize remote clients. Defaults
/// to a loopback port with no keys — reachable only from the same host (spec §Trust model). A loopback
/// peer is trusted without a key; a remote peer must present one of the surface's keys as
/// `Authorization: Bearer <key>`, so binding a routable address is safe by default (fail-closed: no
/// keys means no remote access).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct ServingConfig {
    pub bind: SocketAddr,
    /// Valid API keys for the operator surface (`/control/*`). A remote peer must present one; a
    /// loopback peer is trusted without one. An empty list (the default) rejects every remote control
    /// request. Kept as an array so a single per-integration key can be revoked by removing its entry.
    /// Serializes as a count, never the keys themselves, so the config view (`GET /control/config`)
    /// cannot leak a secret.
    #[serde(default, serialize_with = "redact_keys")]
    pub control_keys: Vec<String>,
    /// Valid API keys for the participant surface (`/platform/*`); the same rule as `control_keys`.
    #[serde(default, serialize_with = "redact_keys")]
    pub platform_keys: Vec<String>,
}

/// Serialize a list of API keys as its length — the count is informative ("two keys configured"); the
/// secrets never cross the wire. Intrinsic to the type, so no serialization of a `ServingConfig` can
/// expose a key.
fn redact_keys<S: Serializer>(keys: &[String], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_u64(keys.len() as u64)
}

impl Default for ServingConfig {
    fn default() -> Self {
        ServingConfig {
            bind: SocketAddr::from(([127, 0, 0, 1], 7777)),
            control_keys: Vec::new(),
            platform_keys: Vec::new(),
        }
    }
}

/// Where to reach the embedding model, and the dimensionality it produces.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct EmbeddingConfig {
    pub endpoint: String,
    pub model: String,
    pub dimensions: usize,
}

/// Where this instance's two databases live. The event log is the source of truth; the graph is a
/// rebuildable projection.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct StorageConfig {
    pub event_log: PathBuf,
    pub graph: PathBuf,
    /// The sqlite-vec index backing semantic search — a third rebuildable projection of the log,
    /// populated only when an embedding endpoint is configured (spec §Storage → vector store).
    pub vectors: PathBuf,
}

impl Default for StorageConfig {
    fn default() -> Self {
        StorageConfig {
            event_log: PathBuf::from("zuihitsu.events.sqlite"),
            graph: PathBuf::from("zuihitsu.graph.sqlite"),
            vectors: PathBuf::from("zuihitsu.vectors.sqlite"),
        }
    }
}

/// Graph snapshotting (spec §Snapshots): periodic `VACUUM INTO` checkpoints so boot restores the
/// latest and replays only the log tail instead of the whole log. **On by default** — the graph is
/// always rebuildable from the log, but a checkpoint turns a slow cold rebuild into a fast one, so the
/// safe default is to keep them. Set `enabled = false` to turn it off. The cadence is activity-gated:
/// the background task checks every `check_interval_seconds` and snapshots only when at least
/// `min_new_events` have been appended since the last one, so idle periods never snapshot.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct SnapshotConfig {
    pub enabled: bool,
    /// Where snapshots are written; defaults to a `snapshots/` directory beside the graph database
    /// (see [`SnapshotConfig::effective_dir`]). Resolved relative to the config file's directory.
    pub dir: Option<PathBuf>,
    /// How often the background snapshotter checks whether a snapshot is due.
    pub check_interval_seconds: u64,
    /// The minimum events appended since the last snapshot before a new one is taken — the activity
    /// gate that keeps idle periods from snapshotting.
    pub min_new_events: u64,
    /// How many snapshots to retain; older ones are pruned after each new one.
    pub keep: usize,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        SnapshotConfig {
            enabled: true,
            dir: None,
            check_interval_seconds: 3_600,
            min_new_events: 20,
            keep: 5,
        }
    }
}

impl SnapshotConfig {
    /// The directory snapshots are written to: the configured `dir`, or a `snapshots/` directory
    /// beside `graph_path` when unset (so the on-by-default behavior needs no configuration).
    pub fn effective_dir(&self, graph_path: &Path) -> PathBuf {
        self.dir.clone().unwrap_or_else(|| {
            graph_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("snapshots")
        })
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
        config.storage.vectors = base.join(&config.storage.vectors);
        if let Some(dir) = &config.snapshots.dir {
            config.snapshots.dir = Some(base.join(dir));
        }
        // Each MCP server name is the `mcp.<name>.*` projection prefix, so it must be a valid Lua
        // identifier — rejected here rather than producing an uncallable projection.
        for name in config.mcp.keys() {
            if !is_lua_identifier(name) {
                return Err(ConfigError::InvalidMcpServerName(name.clone()));
            }
        }
        Ok(config)
    }
}

/// Whether `name` is a valid Lua identifier (`[A-Za-z_][A-Za-z0-9_]*`) — the constraint on an MCP
/// server's config-table key (spec §MCP server blocks).
fn is_lua_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// A failure loading the environmental config.
#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(toml::de::Error),
    /// An `[mcp.<name>]` key that is not a valid Lua identifier (it is the projection prefix).
    InvalidMcpServerName(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(error) => write!(f, "config: could not read the file: {error}"),
            ConfigError::Parse(error) => write!(f, "config: invalid TOML: {error}"),
            ConfigError::InvalidMcpServerName(name) => write!(
                f,
                "config: MCP server name {name:?} is not a valid Lua identifier \
                 ([A-Za-z_][A-Za-z0-9_]*)"
            ),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Io(error) => Some(error),
            ConfigError::Parse(error) => Some(error),
            ConfigError::InvalidMcpServerName(_) => None,
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
        assert_eq!(config.storage.vectors, dir.join("zuihitsu.vectors.sqlite"));

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
    fn snapshots_default_on_with_a_dir_beside_the_graph() {
        // On by default (better safe than sorry), writing to `snapshots/` beside the graph.
        let config = EnvConfig::default();
        assert!(config.snapshots.enabled);
        assert_eq!(
            config
                .snapshots
                .effective_dir(std::path::Path::new("data/graph.sqlite")),
            PathBuf::from("data/snapshots")
        );
    }

    #[test]
    fn snapshots_parse_an_override_and_resolve_the_dir() {
        let dir = temp_dir();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[snapshots]\nenabled = false\ndir = \"snaps\"\nkeep = 3\nmin_new_events = 100\n",
        )
        .unwrap();

        let config = EnvConfig::load(&path).unwrap();
        assert!(!config.snapshots.enabled);
        assert_eq!(config.snapshots.keep, 3);
        assert_eq!(config.snapshots.min_new_events, 100);
        // An explicit dir is honored and resolved against the config's directory.
        assert_eq!(config.snapshots.dir, Some(dir.join("snaps")));
        assert_eq!(
            config.snapshots.effective_dir(&config.storage.graph),
            dir.join("snaps")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn serving_bind_defaults_to_loopback_and_parses_an_override() {
        // Absent, the server binds a loopback port; a `[serving]` block overrides it.
        assert_eq!(
            EnvConfig::default().serving.bind,
            "127.0.0.1:7777".parse().unwrap()
        );
        let dir = temp_dir();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[serving]\nbind = \"127.0.0.1:9090\"\n").unwrap();
        let config = EnvConfig::load(&path).unwrap();
        assert_eq!(config.serving.bind, "127.0.0.1:9090".parse().unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn serving_api_keys_default_empty_and_parse_as_arrays() {
        // No keys by default — a loopback-only, no-remote-access posture.
        assert!(EnvConfig::default().serving.control_keys.is_empty());
        assert!(EnvConfig::default().serving.platform_keys.is_empty());

        let dir = temp_dir();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[serving]\n\
             bind = \"0.0.0.0:7777\"\n\
             control_keys = [\"op-key\"]\n\
             platform_keys = [\"discord-key\", \"web-key\"]\n",
        )
        .unwrap();
        let config = EnvConfig::load(&path).unwrap();
        assert_eq!(config.serving.control_keys, vec!["op-key"]);
        assert_eq!(config.serving.platform_keys, vec!["discord-key", "web-key"]);
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

    #[test]
    fn parses_mcp_server_blocks() {
        let dir = temp_dir();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[mcp.lightpanda]\n\
             command = \"mcp/lightpanda\"\n\
             args = [\"mcp\"]\n\
             deny = [\"evaluate\"]\n",
        )
        .unwrap();

        let config = EnvConfig::load(&path).unwrap();
        let server = config.mcp.get("lightpanda").expect("the lightpanda block");
        assert_eq!(server.command, "mcp/lightpanda");
        assert_eq!(server.args, ["mcp"]);
        assert_eq!(
            server.deny.as_deref(),
            Some(["evaluate".to_owned()].as_slice())
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_mcp_server_name_that_is_not_a_lua_identifier_is_rejected() {
        let dir = temp_dir();
        let path = dir.join("config.toml");
        // `light-panda` is not a valid Lua identifier, so it cannot be a `mcp.<name>` prefix.
        std::fs::write(&path, "[mcp.\"light-panda\"]\ncommand = \"x\"\n").unwrap();

        match EnvConfig::load(&path).unwrap_err() {
            super::ConfigError::InvalidMcpServerName(name) => assert_eq!(name, "light-panda"),
            other => panic!("unexpected error: {other}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }
}
