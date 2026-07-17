//! The real MCP host: an operator-configured server driven through the official [`rmcp`] SDK, over a
//! local stdio subprocess or a remote streamable-HTTP endpoint (spec §External I/O via MCP →
//! bare-minimum host).
//!
//! Spawn builds the configured transport — a child process launched from argv (never shell-split), or
//! a streamable-HTTP connection to a URL — lets rmcp negotiate `initialize` and the protocol version,
//! and snapshots the catalogue with one `tools/list`. The handshake and the catalogue fetch each ride
//! their own init timeout, tolerating a slow cold start. Calls are `tools/call` under a per-call
//! timeout. rmcp owns the JSON-RPC framing,
//! request/response correlation, and the mandatory `notifications/initialized`, so this module is only
//! the projection between the seam's types and rmcp's; once served, both transports yield the same
//! `RunningService`, so the instance is transport-agnostic.
//!
//! The seam's death contract is honoured by dropping the running service: a per-call timeout closes the
//! connection and clears the held service, so the instance's later calls fail [`McpError::Dead`] and the
//! next spawn starts fresh. rmcp's own transport failures (a subprocess exit, a closed connection)
//! surface as a [`ServiceError`] on the next call, mapped here to the seam's error space.

use std::{
    collections::{BTreeMap, HashMap},
    process::Stdio,
    time::Duration,
};

use async_trait::async_trait;
use reqwest::header::{HeaderName, HeaderValue};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult, ContentBlock as RmcpContent, Tool},
    service::{RunningService, ServiceError},
    transport::{
        IntoTransport, StreamableHttpClientTransport, TokioChildProcess,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::Value;
use tokio::{process::Command, sync::Mutex};

use crate::mcp::{
    ContentBlock, McpError, McpHost, McpInstance, McpOutput, McpServerConfig, McpTool,
    McpTransport, McpTransportError,
};

/// How long each spawn step (the `initialize` handshake, then the `tools/list` catalogue fetch) may
/// take, tolerating a slow cold start.
const INIT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a single `tools/call` may take before it is abandoned and the instance declared dead.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// The real [`McpHost`]: connects each configured server through rmcp, over its configured transport.
pub struct RmcpHost;

#[async_trait]
impl McpHost for RmcpHost {
    async fn spawn(
        &self,
        name: &str,
        config: &McpServerConfig,
    ) -> Result<Box<dyn McpInstance>, McpError> {
        // Config validation rejects a bad transport at load; this maps the same fault to a spawn
        // failure as a backstop, so a host driven with an unvalidated config still fails cleanly.
        let service = match config.transport().map_err(transport_error)? {
            McpTransport::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                let mut process = Command::new(command);
                process.args(args).envs(env);
                if let Some(cwd) = cwd {
                    process.current_dir(cwd);
                }
                // The builder inherits stderr by default; discard it so the server's diagnostics never
                // leak into ours and an unread pipe can never fill and stall the child. rmcp pipes
                // stdin/stdout and kills the child when the transport drops.
                let (transport, _stderr) = TokioChildProcess::builder(process)
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|error| McpError::Spawn(format!("{command}: {error}")))?;
                serve(transport, command).await?
            }
            McpTransport::Http { url, headers } => {
                let transport = http_transport(url, headers)?;
                serve(transport, url).await?
            }
        };

        let tools = match tokio::time::timeout(INIT_TIMEOUT, service.list_all_tools()).await {
            Ok(Ok(tools)) => tools.into_iter().map(tool_from).collect::<Vec<_>>(),
            Ok(Err(error)) => return Err(McpError::Spawn(format!("tools/list: {error}"))),
            Err(_) => {
                return Err(McpError::Spawn(format!(
                    "tools/list exceeded {INIT_TIMEOUT:?}"
                )));
            }
        };
        tracing::debug!(server = name, tools = tools.len(), "spawned MCP server");
        Ok(Box::new(RmcpInstance {
            tools,
            service: Mutex::new(Some(service)),
        }))
    }
}

/// A connected server: the snapshotted catalogue, and the running rmcp service behind a mutex so the
/// per-session instance can be shared while its calls stay serialized. The service is dropped to
/// `None` once the instance has died (a per-call timeout), so later calls fail fast.
struct RmcpInstance {
    tools: Vec<McpTool>,
    service: Mutex<Option<RunningService<RoleClient, ()>>>,
}

#[async_trait]
impl McpInstance for RmcpInstance {
    fn tools(&self) -> &[McpTool] {
        &self.tools
    }

    async fn call(&self, tool: &str, arguments: Value) -> Result<McpOutput, McpError> {
        let mut service = self.service.lock().await;
        let Some(running) = service.as_ref() else {
            return Err(McpError::Dead(
                "the server was closed after an earlier failure".to_owned(),
            ));
        };
        // The seam always marshals a JSON object; a non-object (an array from an array-like Lua table)
        // has no place in `tools/call`, so it is sent with no arguments and the server rejects it.
        let mut params = CallToolRequestParams::new(tool.to_owned());
        if let Value::Object(map) = arguments {
            params = params.with_arguments(map);
        }
        // `peer()` is a cheap handle clone, so the call does not borrow the guard — leaving it free to
        // clear the service on a timeout — while the held lock keeps calls serial and the service alive.
        let peer = running.peer().clone();
        let result = match tokio::time::timeout(CALL_TIMEOUT, peer.call_tool(params)).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => return Err(service_error(error)),
            Err(_) => {
                // The in-flight call is abandoned and the server's session-side state is now undefined,
                // so the instance is unusable afterward (spec §No capture → timeout). Close the
                // connection in the background and clear the service so later calls fail `Dead`.
                if let Some(running) = service.take() {
                    tokio::spawn(async move {
                        let _ = running.cancel().await;
                    });
                }
                return Err(McpError::Timeout(format!(
                    "call to {tool} exceeded {CALL_TIMEOUT:?}"
                )));
            }
        };
        into_output(result)
    }

    async fn shutdown(&self) {
        if let Some(running) = self.service.lock().await.take() {
            let _ = running.cancel().await;
        }
    }
}

/// Run rmcp's `initialize` handshake over `transport` under the init timeout, yielding the running
/// service or a [`McpError::Spawn`] contextualised with the server's `context` (its command or URL).
/// Generic over the transport so the stdio child process and the streamable-HTTP connection converge
/// on the one `RunningService` type.
async fn serve<T, E, A>(
    transport: T,
    context: &str,
) -> Result<RunningService<RoleClient, ()>, McpError>
where
    T: IntoTransport<RoleClient, E, A>,
    E: std::error::Error + Send + Sync + 'static,
{
    match tokio::time::timeout(INIT_TIMEOUT, ().serve(transport)).await {
        Ok(Ok(service)) => Ok(service),
        Ok(Err(error)) => Err(McpError::Spawn(format!("{context}: {error}"))),
        Err(_) => Err(McpError::Spawn(format!(
            "{context}: handshake exceeded {INIT_TIMEOUT:?}"
        ))),
    }
}

/// Build the streamable-HTTP transport for a `url` endpoint, threading `headers` through as custom
/// HTTP headers on every request (an operator sets `Authorization` here). A header name or value the
/// HTTP layer rejects is a spawn failure the operator must fix.
fn http_transport(
    url: &str,
    headers: &BTreeMap<String, String>,
) -> Result<StreamableHttpClientTransport<reqwest::Client>, McpError> {
    let mut custom = HashMap::with_capacity(headers.len());
    for (name, value) in headers {
        let header = HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
            McpError::Spawn(format!("{url}: invalid header name {name:?}: {error}"))
        })?;
        let header_value = HeaderValue::from_str(value).map_err(|error| {
            McpError::Spawn(format!("{url}: invalid value for header {name:?}: {error}"))
        })?;
        custom.insert(header, header_value);
    }
    let config =
        StreamableHttpClientTransportConfig::with_uri(url.to_owned()).custom_headers(custom);
    Ok(StreamableHttpClientTransport::from_config(config))
}

/// Project an rmcp [`Tool`] into the seam's [`McpTool`]: the raw name, its description (absent becomes
/// empty), and the JSON-Schema input shape.
fn tool_from(tool: Tool) -> McpTool {
    let input_schema = tool.schema_as_json_value();
    McpTool {
        name: tool.name.into_owned(),
        description: tool.description.unwrap_or_default().into_owned(),
        input_schema,
    }
}

/// Project a tool result into the seam's [`McpOutput`], or a [`McpError::Tool`] when `isError` — the
/// tool-level failure the agent can adapt to, carrying the joined text of its content blocks.
fn into_output(result: CallToolResult) -> Result<McpOutput, McpError> {
    let content: Vec<ContentBlock> = result.content.into_iter().map(content_block).collect();
    if result.is_error.unwrap_or(false) {
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
        structured: result.structured_content,
    })
}

/// Decode one result content block: the common `text` block becomes [`ContentBlock::Text`]; every
/// other type rmcp models (image, audio, resource, resource_link) is carried verbatim as
/// [`ContentBlock::Other`] in its wire shape for a later projection to render.
///
/// A content type rmcp does *not* model is a narrower case than the old hand-rolled client, which
/// carried an unrecognized block through as `Other`: rmcp's `ContentBlock` is a closed enum, so an
/// unknown `type` fails to deserialize the whole `CallToolResult` upstream of here, surfacing as a
/// call error. This is a deliberate trade of that graceful pass-through for the typed SDK — the MCP
/// spec defines exactly the block types rmcp models, so only a non-conformant server hits it.
fn content_block(block: RmcpContent) -> ContentBlock {
    match block {
        RmcpContent::Text(text) => ContentBlock::Text { text: text.text },
        other => ContentBlock::Other(serde_json::to_value(other).unwrap_or(Value::Null)),
    }
}

/// Map an rmcp [`ServiceError`] into the seam's error space: a server-sent JSON-RPC error is a
/// [`McpError::Protocol`], a request timeout is a [`McpError::Timeout`], and every transport or
/// cancellation failure means the connection is gone — a [`McpError::Dead`].
fn service_error(error: ServiceError) -> McpError {
    match error {
        ServiceError::McpError(data) => McpError::Protocol {
            code: i64::from(data.code.0),
            message: data.message.into_owned(),
        },
        ServiceError::Timeout { timeout } => {
            McpError::Timeout(format!("the request timed out after {timeout:?}"))
        }
        other => McpError::Dead(other.to_string()),
    }
}

/// Map an unresolved transport into a [`McpError::Spawn`] — the backstop when a host is driven with a
/// config that was not validated at load (config validation rejects the same fault earlier).
fn transport_error(error: McpTransportError) -> McpError {
    let message = match error {
        McpTransportError::Missing => "no transport configured: set either `command` or `url`",
        McpTransportError::Ambiguous => "both `command` and `url` set: a server has one transport",
    };
    McpError::Spawn(message.to_owned())
}
