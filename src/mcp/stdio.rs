//! The real MCP host: an operator-configured server run as a stdio subprocess, spoken to over
//! newline-delimited JSON-RPC (spec §External I/O via MCP → bare-minimum host).
//!
//! Spawn launches the argv (never shell-split), negotiates `initialize`, sends the mandatory
//! `notifications/initialized`, and snapshots the catalogue with one `tools/list` — all under an init
//! timeout. Calls are `tools/call` under a per-call timeout. The instance is dead on subprocess exit,
//! stdout EOF, a failed write, or non-JSON output; a server-initiated request is answered
//! `-32601 Method not found` so the read loop never blocks waiting on it, and notifications are ignored.
//!
//! The wire is modelled with typed request/response structs (below); `serde_json::Value` appears only
//! for the genuinely schemaless fields — caller-supplied tool arguments, a tool's input schema,
//! `structuredContent`, an unrecognized content block, and the polymorphic JSON-RPC id.

use std::{process::Stdio, time::Duration};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};

use crate::mcp::{
    ContentBlock, McpError, McpHost, McpInstance, McpOutput, McpServerConfig, McpTool,
};

/// The protocol versions this client can speak; the first is advertised in `initialize`. The server
/// echoes the version it will actually use, and the spawn fails if it is none of these.
const SUPPORTED_PROTOCOLS: &[&str] = &["2024-11-05"];
/// How long the spawn handshake (`initialize` + `tools/list`) may take, tolerating a slow cold start.
const INIT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a single `tools/call` may take before it is abandoned and the instance declared dead.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// How long `shutdown` waits for a clean exit after closing stdin before killing the subprocess.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// The real [`McpHost`]: spawns each configured server as its own stdio subprocess.
pub struct StdioHost;

#[async_trait]
impl McpHost for StdioHost {
    async fn spawn(
        &self,
        name: &str,
        config: &McpServerConfig,
    ) -> Result<Box<dyn McpInstance>, McpError> {
        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(cwd) = &config.cwd {
            command.current_dir(cwd);
        }
        let mut child = command
            .spawn()
            .map_err(|error| McpError::Spawn(format!("{}: {error}", config.command)))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Spawn("child has no stdin".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Spawn("child has no stdout".to_owned()))?;
        let mut io = StdioIo {
            child,
            stdin,
            reader: BufReader::new(stdout).lines(),
            next_id: 1,
            dead: None,
        };

        let tools = match tokio::time::timeout(INIT_TIMEOUT, handshake(&mut io)).await {
            Ok(result) => result?,
            Err(_) => {
                return Err(McpError::Spawn(format!(
                    "handshake exceeded {INIT_TIMEOUT:?}"
                )));
            }
        };
        tracing::debug!(server = name, tools = tools.len(), "spawned MCP server");
        Ok(Box::new(StdioInstance {
            tools,
            io: Mutex::new(io),
        }))
    }
}

/// A spawned subprocess server: the snapshotted catalogue, and the framed stdio behind a mutex so the
/// per-session instance can be shared while its I/O stays serialized.
struct StdioInstance {
    tools: Vec<McpTool>,
    io: Mutex<StdioIo>,
}

#[async_trait]
impl McpInstance for StdioInstance {
    fn tools(&self) -> &[McpTool] {
        &self.tools
    }

    async fn call(&self, tool: &str, arguments: Value) -> Result<McpOutput, McpError> {
        let mut io = self.io.lock().await;
        if let Some(reason) = &io.dead {
            return Err(McpError::Dead(reason.clone()));
        }
        let request = io.request::<ToolCallResult>(
            "tools/call",
            ToolCallParams {
                name: tool,
                arguments,
            },
        );
        let result = match tokio::time::timeout(CALL_TIMEOUT, request).await {
            Ok(result) => result?,
            Err(_) => {
                // The in-flight call is abandoned and the read stream is now desynced, so the
                // instance is unusable afterward (spec §No capture → timeout).
                let message = format!("call to {tool} exceeded {CALL_TIMEOUT:?}");
                io.dead = Some(message.clone());
                return Err(McpError::Timeout(message));
            }
        };
        Ok(result.into_output()?)
    }

    async fn shutdown(&self) {
        let mut io = self.io.lock().await;
        let _ = io.stdin.shutdown().await;
        if tokio::time::timeout(SHUTDOWN_GRACE, io.child.wait())
            .await
            .is_err()
        {
            let _ = io.child.kill().await;
        }
    }
}

/// The framed stdio of one subprocess: the child, its piped stdin, a line reader over its stdout, the
/// JSON-RPC id counter, and a death reason once it has failed.
struct StdioIo {
    child: Child,
    stdin: ChildStdin,
    reader: Lines<BufReader<ChildStdout>>,
    next_id: i64,
    dead: Option<String>,
}

impl StdioIo {
    /// Send a JSON-RPC request with typed `params` and read until its matching response, deserializing
    /// the result into `R`. Answers any server-initiated request with `-32601` and ignores
    /// notifications along the way, so neither blocks the response.
    async fn request<R: serde::de::DeserializeOwned>(
        &mut self,
        method: &str,
        params: impl Serialize,
    ) -> Result<R, McpError> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&Request {
            jsonrpc: JSONRPC,
            id,
            method,
            params,
        })
        .await?;
        loop {
            let incoming: Incoming = self.read().await?;
            // A message carrying `method` is a server-initiated request (if it also has an `id`) or a
            // notification (if it doesn't). Answer the former "method not found" without blocking, and
            // ignore the latter (incl. `tools/list_changed`).
            if incoming.method.is_some() {
                if let Some(request_id) = &incoming.id {
                    self.write(&ErrorReply {
                        jsonrpc: JSONRPC,
                        id: request_id,
                        error: RpcError {
                            code: -32601,
                            message: "method not found".to_owned(),
                        },
                    })
                    .await?;
                }
                continue;
            }
            if incoming.id.as_ref() == Some(&Value::from(id)) {
                if let Some(error) = incoming.error {
                    return Err(McpError::Protocol {
                        code: error.code,
                        message: error.message,
                    });
                }
                let result = incoming.result.unwrap_or(Value::Null);
                return serde_json::from_value(result)
                    .map_err(|error| self.die(format!("malformed result: {error}")));
            }
            // A response to some other id — impossible with serial requests; ignore it.
        }
    }

    /// Send a notification with typed `params` (no response is read).
    async fn notify(&mut self, method: &str, params: impl Serialize) -> Result<(), McpError> {
        self.write(&Notification {
            jsonrpc: JSONRPC,
            method,
            params,
        })
        .await
    }

    async fn write(&mut self, message: &impl Serialize) -> Result<(), McpError> {
        let mut line = serde_json::to_string(message).map_err(|error| self.die(error))?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|error| self.die(error))?;
        self.stdin.flush().await.map_err(|error| self.die(error))
    }

    async fn read<T: serde::de::DeserializeOwned>(&mut self) -> Result<T, McpError> {
        match self.reader.next_line().await {
            Ok(Some(line)) => serde_json::from_str(&line)
                .map_err(|error| self.die(format!("non-JSON output: {error}"))),
            Ok(None) => Err(self.die("stdout closed")),
            Err(error) => Err(self.die(error)),
        }
    }

    /// Record the death reason and return it as a [`McpError::Dead`], so later calls fail fast.
    fn die(&mut self, reason: impl std::fmt::Display) -> McpError {
        let reason = reason.to_string();
        if self.dead.is_none() {
            self.dead = Some(reason.clone());
        }
        McpError::Dead(reason)
    }
}

/// The spawn handshake: negotiate `initialize`, confirm the echoed protocol version, send
/// `notifications/initialized`, then snapshot the catalogue with one `tools/list`.
async fn handshake(io: &mut StdioIo) -> Result<Vec<McpTool>, McpError> {
    let initialized: InitializeResult = io
        .request(
            "initialize",
            InitializeParams {
                protocol_version: SUPPORTED_PROTOCOLS[0],
                capabilities: ClientCapabilities {},
                client_info: ClientInfo {
                    name: "zuihitsu",
                    version: env!("CARGO_PKG_VERSION"),
                },
            },
        )
        .await?;
    if !SUPPORTED_PROTOCOLS.contains(&initialized.protocol_version.as_str()) {
        return Err(McpError::Spawn(format!(
            "server speaks unsupported protocol version {:?}",
            initialized.protocol_version
        )));
    }
    io.notify("notifications/initialized", Empty {}).await?;
    let listed: ToolsList = io.request("tools/list", Empty {}).await?;
    Ok(listed.tools.into_iter().map(McpTool::from).collect())
}

// --- The JSON-RPC wire, typed. `Value` only for genuinely schemaless payloads. ---

/// The JSON-RPC version every frame carries.
const JSONRPC: &str = "2.0";

#[derive(Serialize)]
struct Request<'a, P> {
    jsonrpc: &'static str,
    id: i64,
    method: &'a str,
    params: P,
}

#[derive(Serialize)]
struct Notification<'a, P> {
    jsonrpc: &'static str,
    method: &'a str,
    params: P,
}

/// Our reply to an unsupported server-initiated request.
#[derive(Serialize)]
struct ErrorReply<'a> {
    jsonrpc: &'static str,
    id: &'a Value,
    error: RpcError,
}

#[derive(Serialize, Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

/// One incoming frame, before we know which it is: a response (`result`/`error` keyed to our `id`), a
/// server-initiated request (`method` + `id`), or a notification (`method`, no `id`). `id` is the
/// polymorphic JSON-RPC id and `result` is the not-yet-typed payload — both decoded further by caller.
#[derive(Deserialize)]
struct Incoming {
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<RpcError>,
}

/// An empty params object (`{}`) — `initialize`'s capabilities, `tools/list`, `initialized`.
#[derive(Serialize)]
struct Empty {}

#[derive(Serialize)]
struct InitializeParams<'a> {
    #[serde(rename = "protocolVersion")]
    protocol_version: &'a str,
    capabilities: ClientCapabilities,
    #[serde(rename = "clientInfo")]
    client_info: ClientInfo<'a>,
}

/// No `sampling` / `elicitation` / `roots` — the bare-minimum client advertises nothing.
#[derive(Serialize)]
struct ClientCapabilities {}

#[derive(Serialize)]
struct ClientInfo<'a> {
    name: &'a str,
    version: &'a str,
}

#[derive(Deserialize)]
struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
}

#[derive(Deserialize)]
struct ToolsList {
    #[serde(default)]
    tools: Vec<ToolDef>,
}

#[derive(Deserialize)]
struct ToolDef {
    name: String,
    #[serde(default)]
    description: String,
    /// The tool's JSON-Schema input shape — genuinely schemaless, so it stays a `Value`.
    #[serde(rename = "inputSchema", default = "empty_schema")]
    input_schema: Value,
}

impl From<ToolDef> for McpTool {
    fn from(tool: ToolDef) -> McpTool {
        McpTool {
            name: tool.name,
            description: tool.description,
            input_schema: tool.input_schema,
        }
    }
}

fn empty_schema() -> Value {
    serde_json::json!({ "type": "object" })
}

#[derive(Serialize)]
struct ToolCallParams<'a> {
    name: &'a str,
    /// The caller-supplied argument object — passed through untyped, as the server validates it.
    arguments: Value,
}

#[derive(Deserialize)]
struct ToolCallResult {
    #[serde(default)]
    content: Vec<Value>,
    /// The server's decoded `structuredContent`, if any — server-defined, so it stays a `Value`.
    #[serde(rename = "structuredContent", default)]
    structured: Option<Value>,
    #[serde(rename = "isError", default)]
    is_error: bool,
}

impl ToolCallResult {
    /// Project a tool result into the seam's [`McpOutput`], or a [`McpError::Tool`] when `isError`.
    fn into_output(self) -> Result<McpOutput, McpError> {
        let content: Vec<ContentBlock> = self.content.into_iter().map(content_block).collect();
        if self.is_error {
            let text = content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    ContentBlock::Other(_) => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            return Err(McpError::Tool(text));
        }
        Ok(McpOutput {
            content,
            structured: self.structured,
        })
    }
}

/// One result content block as the server sends it; only the common `text` block is decoded, every
/// other type carried verbatim for a later projection to shape.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireBlock {
    Text { text: String },
}

/// Decode a content block by its `type`: a `text` block becomes [`ContentBlock::Text`]; anything else
/// is carried verbatim as [`ContentBlock::Other`].
fn content_block(block: Value) -> ContentBlock {
    match serde_json::from_value::<WireBlock>(block.clone()) {
        Ok(WireBlock::Text { text }) => ContentBlock::Text { text },
        Err(_) => ContentBlock::Other(block),
    }
}
