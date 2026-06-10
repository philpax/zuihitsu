//! The MCP (Model Context Protocol) client seam: the agent's only outward reach (spec §External I/O
//! via MCP).
//!
//! An operator-configured server hosts a capability — driving a browser, calling a tool, querying a
//! source — and the integration spawns it, snapshots its tool catalogue, and calls those tools on
//! demand. The seam is at the *instance* level so lifecycle is testable, not just calls: the real
//! [`StdioHost`] drives a subprocess over newline-delimited JSON-RPC, while the scriptable
//! [`FakeMcpHost`] returns canned results with no subprocess (spec §Testability). This module is the
//! client itself; the Lua projection that exposes these as `mcp.<server>.*` lives in
//! `crate::agent::mcp_api`.

mod fake;
mod stdio;

pub use fake::{FakeMcpHost, FakeServer};
pub use stdio::StdioHost;

use std::{collections::BTreeMap, path::PathBuf};

use async_trait::async_trait;
use serde::Deserialize;

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

/// One configured MCP server (the `[mcp.<name>]` block, spec §Configuration). `command` is an
/// executable launched as argv — never shell-split.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct McpServerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: Option<PathBuf>,
    /// Raw tool names to project; with `None`, the whole catalogue. Applied during MCP catalogue
    /// probing (`crate::agent::mcp_api`).
    pub allow: Option<Vec<String>>,
    /// Raw tool names to drop after `allow`. Applied during the same probe.
    pub deny: Option<Vec<String>>,
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
}

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpError::Spawn(message) => write!(f, "mcp: could not spawn the server: {message}"),
            McpError::Protocol { code, message } => {
                write!(f, "mcp: protocol error {code}: {message}")
            }
            McpError::Tool(message) => write!(f, "mcp: the tool reported an error: {message}"),
            McpError::Dead(message) => {
                write!(f, "mcp: the server is no longer available: {message}")
            }
            McpError::Timeout(message) => write!(f, "mcp: timed out: {message}"),
        }
    }
}

impl std::error::Error for McpError {}
