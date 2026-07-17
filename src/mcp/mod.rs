//! The MCP (Model Context Protocol) client seam: the agent's only outward reach (spec §External I/O
//! via MCP).
//!
//! An operator-configured server hosts a capability — driving a browser, calling a tool, querying a
//! source — and the integration spawns it, snapshots its tool catalogue, and calls those tools on
//! demand. The seam is at the *instance* level so lifecycle is testable, not just calls: the real
//! [`RmcpHost`] drives a server through the [`rmcp`] SDK — a local stdio subprocess or a remote
//! streamable-HTTP endpoint — while the scriptable [`FakeMcpHost`] returns canned results with no
//! transport at all (spec §Testability). This module is the client itself; the Lua projection that
//! exposes these as `mcp.<server>.*` lives in `crate::agent::mcp_api`.

mod client;
mod fake;

pub use client::RmcpHost;
pub use fake::{FakeMcpHost, FakeServer};

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize, Serializer};

/// Spawns server instances from config — the swappable factory behind the seam (the real stdio host,
/// or a scriptable fake). `Send + Sync` so the host (and the instances it yields) ride behind the
/// `Arc` handles a multi-thread turn shares across worker threads.
#[async_trait]
pub trait McpHost: Send + Sync {
    /// Spawn the server named `name` per `config`, returning a live instance whose catalogue is
    /// already snapshotted, or a [`McpError`] if it could not be brought up.
    async fn spawn(
        &self,
        name: &str,
        config: &McpServerConfig,
    ) -> Result<Box<dyn McpInstance>, McpError>;
}

/// A spawned MCP server instance: its tool catalogue, snapshotted at spawn, and on-demand calls.
/// `Send + Sync` so a per-session instance can be held behind a shared `Arc` on a multi-thread runtime.
#[async_trait]
pub trait McpInstance: Send + Sync {
    /// The advertised tools, as snapshotted at spawn (`tools/list` for the real host).
    fn tools(&self) -> &[McpTool];

    /// Call `tool` with `arguments` (a JSON object), returning its result or a catchable failure. A
    /// `&self` call so the per-session instance can be held behind a shared handle; the I/O is
    /// serialized internally (the VM calls one tool at a time).
    async fn call(&self, tool: &str, arguments: serde_json::Value) -> Result<McpOutput, McpError>;

    /// Shut the instance down: close its input, wait, and kill on a grace timeout. Best-effort.
    async fn shutdown(&self);
}

/// One configured MCP server (the `[mcp.<name>]` block, spec §Configuration). Its transport is either
/// a local stdio subprocess (`command`, an executable launched as argv — never shell-split) or a
/// remote streamable-HTTP endpoint (`url`); exactly one must be set, enforced by [`Self::transport`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct McpServerConfig {
    /// The stdio transport's executable, launched as argv. Mutually exclusive with `url`.
    pub command: String,
    pub args: Vec<String>,
    /// Serializes as its variable names only, never their values, so the config view
    /// (`GET /control/config`) cannot leak a secret an MCP server reads from the environment.
    #[serde(serialize_with = "redact_map_values")]
    pub env: BTreeMap<String, String>,
    pub cwd: Option<PathBuf>,
    /// The streamable-HTTP transport's endpoint URL of a remote MCP server. Mutually exclusive with
    /// `command`. Serializes with its userinfo and query redacted, so a credential an operator embedded
    /// in the URL (a `user:pass@` or a `?token=`) cannot leak through the config view — though
    /// `headers` is the intended place for one.
    #[serde(serialize_with = "redact_url")]
    pub url: Option<String>,
    /// Custom HTTP headers sent with every request to a `url` endpoint (e.g. `Authorization`).
    /// Serializes as its header names only, never their values, so the config view cannot leak a
    /// bearer token. Ignored for a stdio server.
    #[serde(serialize_with = "redact_map_values")]
    pub headers: BTreeMap<String, String>,
    /// Raw tool names to project; with `None`, the whole catalogue. Applied during MCP catalogue
    /// probing (`crate::agent::mcp_api`).
    pub allow: Option<Vec<String>>,
    /// Raw tool names to drop after `allow`. Applied during the same probe.
    pub deny: Option<Vec<String>>,
}

impl McpServerConfig {
    /// Resolve which transport this server uses, requiring exactly one of `command`/`url`. A block
    /// with neither, or both, is an operator misconfiguration the caller surfaces (config validation
    /// rejects it at load; the host maps it to a spawn failure as a backstop).
    pub(crate) fn transport(&self) -> Result<McpTransport<'_>, McpTransportError> {
        match (self.command.is_empty(), self.url.as_deref()) {
            (false, None) => Ok(McpTransport::Stdio {
                command: &self.command,
                args: &self.args,
                env: &self.env,
                cwd: self.cwd.as_deref(),
            }),
            (true, Some(url)) => Ok(McpTransport::Http {
                url,
                headers: &self.headers,
            }),
            (true, None) => Err(McpTransportError::Missing),
            (false, Some(_)) => Err(McpTransportError::Ambiguous),
        }
    }
}

/// The resolved transport of a configured MCP server: a local stdio subprocess, or a remote
/// streamable-HTTP endpoint. Borrows its config; produced by [`McpServerConfig::transport`].
pub(crate) enum McpTransport<'a> {
    Stdio {
        command: &'a str,
        args: &'a [String],
        env: &'a BTreeMap<String, String>,
        cwd: Option<&'a Path>,
    },
    Http {
        url: &'a str,
        headers: &'a BTreeMap<String, String>,
    },
}

/// Why a server's transport could not be resolved: it set neither `command` nor `url`, or set both.
pub(crate) enum McpTransportError {
    Missing,
    Ambiguous,
}

/// Serialize a secret-bearing string map (environment variables, HTTP headers) as the list of its
/// keys — the values (which may hold secrets) never cross the wire. Intrinsic to the type, so no
/// serialization leaks them.
fn redact_map_values<S: Serializer>(
    map: &BTreeMap<String, String>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(map.keys())
}

/// Serialize an endpoint URL with any embedded credential stripped — its userinfo cleared and its
/// query replaced with a marker — so the config view shows which server (scheme, host, path) without
/// leaking a `user:pass@` or a `?token=` secret. An unparseable URL serializes as a fixed placeholder
/// rather than risk leaking whatever it holds.
fn redact_url<S: Serializer>(url: &Option<String>, serializer: S) -> Result<S::Ok, S::Error> {
    let Some(raw) = url else {
        return serializer.serialize_none();
    };
    let shown = match url::Url::parse(raw) {
        Ok(mut parsed) => {
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            if parsed.query().is_some() {
                parsed.set_query(Some("redacted"));
            }
            parsed.to_string()
        }
        Err(_) => "<unparseable url>".to_owned(),
    };
    serializer.serialize_some(&shown)
}

/// One advertised tool: its raw name, description, and JSON-Schema input shape — rendered into the
/// system prompt and the basis of the `mcp.<server>.*` projection (`crate::agent::mcp_api`).
#[derive(Clone, Debug, PartialEq)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A successful tool result: its content blocks, plus any decoded `structuredContent` — the raw shape
/// the Lua projection (spec §External I/O via MCP → results) renders for the agent.
#[derive(Clone, Debug, PartialEq)]
pub struct McpOutput {
    pub content: Vec<ContentBlock>,
    pub structured: Option<serde_json::Value>,
}

/// One result content block. Text is decoded explicitly (the common case — `markdown` returns one
/// text block); every other block type is carried verbatim for the projection to shape.
#[derive(Clone, Debug, PartialEq)]
pub enum ContentBlock {
    Text { text: String },
    Other(serde_json::Value),
}

/// A catchable MCP failure. `Display` leads with an `mcp:` context prefix, per the error convention.
#[derive(Clone, Debug)]
pub enum McpError {
    /// The server could not be brought up (spawn, handshake, or unspeakable protocol version).
    Spawn(String),
    /// A JSON-RPC protocol error from the server (e.g. `-32601 Tool not found`).
    Protocol { code: i64, message: String },
    /// The server answered with `isError: true` — a tool-level failure the agent can adapt to.
    Tool(String),
    /// The instance is dead (subprocess exit, stdout EOF, a failed write, or non-JSON output).
    Dead(String),
    /// A call or the initial handshake exceeded its timeout.
    Timeout(String),
    /// The agent called `mcp.<server>.<tool>` for a tool the server does not advertise. Teachable:
    /// the agent misaddressed the call, so it is unprefixed prose naming the server and tool.
    UnknownTool { server: String, tool: String },
}

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpError::Spawn(message) => write!(f, "mcp: could not spawn the server: {message}"),
            McpError::Protocol { code, message } => {
                write!(f, "mcp: protocol error {code}: {message}")
            }
            // Teachable (the agent can adapt): unprefixed prose. The call's `server.tool` is packed
            // into `message` by [`McpError::with_call`], so the agent sees exactly which call erred.
            McpError::Tool(message) => write!(f, "the tool reported an error: {message}"),
            McpError::Dead(message) => {
                write!(f, "mcp: the server is no longer available: {message}")
            }
            McpError::Timeout(message) => write!(f, "mcp: timed out: {message}"),
            McpError::UnknownTool { server, tool } => {
                write!(f, "server {server:?} has no tool {tool:?}")
            }
        }
    }
}

impl std::error::Error for McpError {}

impl McpError {
    /// Pack the server name into a spawn failure, so the agent sees which server could not start.
    pub(crate) fn with_server(self, server: &str) -> Self {
        match self {
            McpError::Spawn(message) => McpError::Spawn(format!("{server}: {message}")),
            other => other,
        }
    }

    /// Pack the `mcp.<server>.<tool>` call into a per-call failure, so the agent sees exactly which
    /// call erred. `Dead` is server-level (the call merely discovered the server is gone), so it
    /// carries the server alone; `Spawn` never reaches here (it fails before a call is made).
    pub(crate) fn with_call(self, server: &str, tool: &str) -> Self {
        let call = format!("{server}.{tool}");
        match self {
            McpError::Tool(message) => McpError::Tool(format!("{call}: {message}")),
            McpError::Protocol { code, message } => McpError::Protocol {
                code,
                message: format!("{call}: {message}"),
            },
            McpError::Dead(message) => McpError::Dead(format!("{server}: {message}")),
            McpError::Timeout(message) => McpError::Timeout(format!("{call}: {message}")),
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{McpServerConfig, McpTransport, McpTransportError};

    /// Secrets never cross the config view: env values and header values serialize as their keys
    /// alone, so `GET /control/config` cannot leak an environment secret or a bearer token.
    #[test]
    fn env_and_header_values_are_redacted_on_serialization() {
        let config = McpServerConfig {
            command: "browser".to_owned(),
            env: [("TOKEN".to_owned(), "s3cret".to_owned())].into(),
            url: None,
            headers: [("Authorization".to_owned(), "Bearer s3cret".to_owned())].into(),
            ..McpServerConfig::default()
        };
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(json["env"], serde_json::json!(["TOKEN"]));
        assert_eq!(json["headers"], serde_json::json!(["Authorization"]));
        let rendered = serde_json::to_string(&config).unwrap();
        assert!(!rendered.contains("s3cret"), "a secret leaked: {rendered}");
    }

    /// A credential embedded in the endpoint URL — userinfo or a query token — is stripped on
    /// serialization, while the host and path that identify the server remain visible.
    #[test]
    fn a_url_embedded_credential_is_redacted_on_serialization() {
        let config = McpServerConfig {
            url: Some("https://user:s3cret@example.com/mcp?token=s3cret".to_owned()),
            ..McpServerConfig::default()
        };
        let rendered = serde_json::to_string(&config).unwrap();
        assert!(
            !rendered.contains("s3cret"),
            "a URL secret leaked: {rendered}"
        );
        assert!(
            rendered.contains("example.com/mcp"),
            "the host/path should stay: {rendered}"
        );
    }

    #[test]
    fn a_command_resolves_to_the_stdio_transport() {
        let config = McpServerConfig {
            command: "browser".to_owned(),
            ..McpServerConfig::default()
        };
        assert!(matches!(
            config.transport(),
            Ok(McpTransport::Stdio {
                command: "browser",
                ..
            })
        ));
    }

    #[test]
    fn a_url_resolves_to_the_http_transport() {
        let config = McpServerConfig {
            url: Some("https://example.com/mcp".to_owned()),
            ..McpServerConfig::default()
        };
        assert!(matches!(
            config.transport(),
            Ok(McpTransport::Http {
                url: "https://example.com/mcp",
                ..
            })
        ));
    }

    #[test]
    fn neither_or_both_transports_do_not_resolve() {
        assert!(matches!(
            McpServerConfig::default().transport(),
            Err(McpTransportError::Missing)
        ));
        let both = McpServerConfig {
            command: "browser".to_owned(),
            url: Some("https://example.com/mcp".to_owned()),
            ..McpServerConfig::default()
        };
        assert!(matches!(
            both.transport(),
            Err(McpTransportError::Ambiguous)
        ));
    }
}
