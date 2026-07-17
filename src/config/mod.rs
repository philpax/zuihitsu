//! Environmental (operational) configuration: the TOML file that says *where and how this instance
//! runs* — the event-log and graph paths, and (later) endpoints and bind addresses. It is distinct
//! from behavioral config, which lives in the log as `ConfigSet` events (spec §Initialization).
//!
//! Because it carries the database paths, this file is the instance selector: two configs with
//! different paths are two independent agents. Relative paths resolve against the config file's own
//! directory, so an instance is relocatable by moving its directory.

mod defaults;

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize, Serializer};

use crate::mcp::{McpServerConfig, McpTransportError};

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
    /// The registered connectors, one `[connectors.<id>]` entry each. The table key is the connector's
    /// platform id (`discord`, `slack`, `direct`): a `/platform/*` request bearing that connector's
    /// `key` is scoped to that platform and attributed to that connector, so a connector can only ever
    /// act on its own platform — there is no per-request platform to spoof (spec §Trust model).
    #[serde(default)]
    pub connectors: BTreeMap<String, ConnectorConfig>,
}

/// One registered connector. The platform/connector id is the `[connectors]` map key; this carries the
/// bearer key it authenticates with. The key serializes redacted, so the config view cannot leak it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConnectorConfig {
    #[serde(serialize_with = "redact_key")]
    pub key: String,
}

/// Where to reach the generation model, and how to sample from it. An empty `endpoint` means "not
/// configured". Each sampling field is optional: unset fields are simply not sent, so the serving
/// layer applies its own per-model default.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ModelConfig {
    pub endpoint: String,
    pub llm: String,
    /// The model's context window, in tokens. Required whenever an `endpoint` is set: the OpenAI-style
    /// API does not report it, so the operator states it, and the agent derives its compaction budget
    /// from it (a fraction of the window). Update it when the context length or the backing model
    /// changes (see `docs/model_management.md`).
    pub context_length: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub min_p: Option<f32>,
    pub presence_penalty: Option<f32>,
    /// Override the serving layer's thinking default (`chat_template_kwargs.enable_thinking`).
    pub thinking: Option<bool>,
    /// Transport resilience for the served model client (`[model.resilience]`): the request
    /// timeout, retries, backoff, and the circuit breaker.
    pub resilience: ResilienceConfig,
}

/// Transport resilience for the served model client: the per-call request timeout, bounded retries
/// of transient failures with exponential backoff, and the circuit breaker that fails fast while
/// the backend stays down. Operational config, not behavioral: retries the agent never saw emit
/// nothing to the event log (spec §Event sourcing), so replay never depends on these values — which
/// is why they live here and not in the logged `Settings`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct ResilienceConfig {
    /// The whole-request timeout for one backend HTTP call, in seconds. reqwest's default is no
    /// timeout, so without this a hung backend stalls the turn forever instead of surfacing a
    /// retryable timeout error. Generous by default: a local model reprocessing a long prompt can
    /// legitimately take minutes.
    pub request_timeout_seconds: u64,
    /// The total attempts for one `generate` call — the first try plus retries of transient
    /// failures. Non-transient failures (schema, auth, other 4xx) are never retried.
    pub max_attempts: u32,
    /// The first retry's backoff, in milliseconds; each further retry doubles it (with jitter).
    pub backoff_base_ms: u64,
    /// The per-retry backoff ceiling, in milliseconds.
    pub backoff_max_ms: u64,
    /// How many consecutive transient failures open the circuit, after which model calls fail fast
    /// without reaching the backend.
    pub breaker_failure_threshold: u32,
    /// How long an open circuit fails fast, in seconds, before one half-open probe request is let
    /// through (success closes the circuit; failure re-opens it for another window).
    pub breaker_open_seconds: u64,
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
}

/// Serialize a list of API keys as its length — the count is informative ("two keys configured"); the
/// secrets never cross the wire. Intrinsic to the type, so no serialization of a `ServingConfig` can
/// expose a key.
fn redact_keys<S: Serializer>(keys: &[String], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_u64(keys.len() as u64)
}

/// Serialize a single connector key as a fixed placeholder — its presence is informative, the secret
/// never is. Intrinsic to [`ConnectorConfig`], so no serialization can expose a connector's key.
fn redact_key<S: Serializer>(_key: &str, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str("<redacted>")
}

/// Where to reach the embedding model, and the dimensionality it produces.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct EmbeddingConfig {
    pub endpoint: String,
    pub model: String,
    pub dimensions: usize,
    /// The whole-request timeout for one embedding HTTP call, in seconds — the same hung-backend
    /// guard the model client has (see [`ResilienceConfig::request_timeout_seconds`]).
    pub request_timeout_seconds: u64,
    /// The embedding model's context window, in tokens. When set, every input is truncated to 2.5
    /// characters per context token before the request, and a backend length-overflow rejection
    /// retries with progressively smaller truncations (see `OpenAiEmbedder::embed`). `None` sends
    /// inputs whole, for a backend whose window comfortably exceeds any memory entry.
    pub context_length: Option<usize>,
}

/// Where this instance's databases live — one directory holding all three. The event log is the
/// source of truth; the graph and the vector index are rebuildable projections of it. The directory
/// *is* the instance selector, and a single field keeps the three from ever scattering.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct StorageConfig {
    /// The directory the databases live in, resolved relative to the config file's directory.
    pub dir: PathBuf,
}

impl StorageConfig {
    /// The event log — the single-writer source of truth.
    pub fn event_log(&self) -> PathBuf {
        self.dir.join("events.sqlite")
    }

    /// The graph projection.
    pub fn graph(&self) -> PathBuf {
        self.dir.join("graph.sqlite")
    }

    /// The sqlite-vec index backing semantic search, populated only when an embedding endpoint is
    /// configured (spec §Storage → vector store).
    pub fn vectors(&self) -> PathBuf {
        self.dir.join("vectors.sqlite")
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
    /// instance still has somewhere to put its databases. The parse, resolution, and validation are
    /// [`load_from_string`](Self::load_from_string); this only reads the file.
    pub fn load(path: &Path) -> Result<EnvConfig, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            // A missing file is not an error — an empty document parses to the all-defaults config.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(source) => {
                return Err(ConfigError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        // Re-attach the file path to a parse failure: `load_from_string` only knows the base directory,
        // but the operator wants the offending file named.
        EnvConfig::load_from_string(&text, base).map_err(|error| match error {
            ConfigError::Parse { source, .. } => ConfigError::Parse {
                path: path.to_path_buf(),
                source,
            },
            other => other,
        })
    }

    /// Parse config from TOML `text`, resolving relative storage paths against `base` (the directory
    /// the config file lives in) and validating the MCP blocks. The in-memory core of [`load`]: a
    /// caller with the text already in hand — a test, or an embedded default — skips the filesystem.
    /// An empty `text` yields the all-defaults config, so a missing file and a blank one coincide.
    pub fn load_from_string(text: &str, base: &Path) -> Result<EnvConfig, ConfigError> {
        let mut config: EnvConfig = toml::from_str(text).map_err(|source| ConfigError::Parse {
            path: base.to_path_buf(),
            source,
        })?;
        config.storage.dir = base.join(&config.storage.dir);
        if let Some(dir) = &config.snapshots.dir {
            config.snapshots.dir = Some(base.join(dir));
        }
        // Each MCP server name is the `mcp.<name>.*` projection prefix, so it must be a valid Lua
        // identifier — rejected here rather than producing an uncallable projection. Its transport must
        // resolve (exactly one of `command`/`url`), so a misconfiguration is caught at load rather than
        // silently dropping the server when it fails to spawn.
        for (name, server) in &config.mcp {
            if !is_lua_identifier(name) {
                return Err(ConfigError::InvalidMcpServerName(name.clone()));
            }
            if let Err(error) = server.transport() {
                return Err(match error {
                    McpTransportError::Missing => {
                        ConfigError::McpServerMissingTransport(name.clone())
                    }
                    McpTransportError::Ambiguous => {
                        ConfigError::McpServerAmbiguousTransport(name.clone())
                    }
                });
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
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    /// An `[mcp.<name>]` key that is not a valid Lua identifier (it is the projection prefix).
    InvalidMcpServerName(String),
    /// An `[mcp.<name>]` block that sets neither `command` nor `url` — it has no transport.
    McpServerMissingTransport(String),
    /// An `[mcp.<name>]` block that sets both `command` and `url` — a server has one transport.
    McpServerAmbiguousTransport(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io { path, source } => {
                write!(f, "config: could not read {}: {source}", path.display())
            }
            ConfigError::Parse { path, source } => {
                write!(f, "config: invalid TOML in {}: {source}", path.display())
            }
            ConfigError::InvalidMcpServerName(name) => write!(
                f,
                "config: MCP server name {name:?} is not a valid Lua identifier \
                 ([A-Za-z_][A-Za-z0-9_]*)"
            ),
            ConfigError::McpServerMissingTransport(name) => write!(
                f,
                "config: MCP server {name:?} has no transport: set either `command` (a stdio \
                 subprocess) or `url` (a streamable-HTTP endpoint)"
            ),
            ConfigError::McpServerAmbiguousTransport(name) => write!(
                f,
                "config: MCP server {name:?} sets both `command` and `url`: a server has one \
                 transport, not both"
            ),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Io { source, .. } => Some(source),
            ConfigError::Parse { source, .. } => Some(source),
            ConfigError::InvalidMcpServerName(_)
            | ConfigError::McpServerMissingTransport(_)
            | ConfigError::McpServerAmbiguousTransport(_) => None,
        }
    }
}

#[cfg(test)]
mod tests;
