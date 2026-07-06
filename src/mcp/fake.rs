//! A scriptable in-memory MCP host for tests — the seam fake (spec §Testability). A test scripts
//! each server's tool catalogue and per-tool results, plus the lifecycle knobs the real host can
//! exhibit (injected latency, spawn failure, and death), so a `mcp.<server>.*` test needs no
//! subprocess or network. Mirrors `ScriptedModel`/`FakeEmbedder`.

use std::{collections::HashMap, time::Duration};

use async_trait::async_trait;

use super::{McpError, McpHost, McpInstance, McpOutput, McpServerConfig, McpTool};

/// A scriptable [`McpHost`]: a set of named servers, each scripted via [`FakeServer`].
#[derive(Clone, Default)]
pub struct FakeMcpHost {
    servers: HashMap<String, FakeServer>,
}

impl FakeMcpHost {
    pub fn new() -> FakeMcpHost {
        FakeMcpHost::default()
    }

    /// Register `server` under `name`, chainably.
    pub fn with(mut self, name: impl Into<String>, server: FakeServer) -> FakeMcpHost {
        self.servers.insert(name.into(), server);
        self
    }
}

#[async_trait]
impl McpHost for FakeMcpHost {
    async fn spawn(
        &self,
        name: &str,
        _config: &McpServerConfig,
    ) -> Result<Box<dyn McpInstance>, McpError> {
        let Some(server) = self.servers.get(name) else {
            return Err(McpError::Spawn(format!("no scripted server {name:?}")));
        };
        if let Some(message) = &server.spawn_error {
            return Err(McpError::Spawn(message.clone()));
        }
        Ok(Box::new(FakeMcpInstance {
            tools: server.tools.clone(),
            results: server.results.clone(),
            latency: server.latency,
            dead: server.dead,
        }))
    }
}

/// How one fake server behaves: its advertised tools, the canned result per tool, an optional latency
/// every call incurs, and whether it spawns dead or fails to spawn at all.
#[derive(Clone, Default)]
pub struct FakeServer {
    tools: Vec<McpTool>,
    results: HashMap<String, Result<McpOutput, McpError>>,
    latency: Option<Duration>,
    dead: bool,
    spawn_error: Option<String>,
}

impl FakeServer {
    /// A server advertising `tools`.
    pub fn new(tools: Vec<McpTool>) -> FakeServer {
        FakeServer {
            tools,
            ..FakeServer::default()
        }
    }

    /// A server whose `spawn` fails with `message` (never yields an instance).
    pub fn spawn_failure(message: impl Into<String>) -> FakeServer {
        FakeServer {
            spawn_error: Some(message.into()),
            ..FakeServer::default()
        }
    }

    /// Script `tool` to return `output`.
    pub fn returns(mut self, tool: impl Into<String>, output: McpOutput) -> FakeServer {
        self.results.insert(tool.into(), Ok(output));
        self
    }

    /// Script `tool` to fail with `error`.
    pub fn fails(mut self, tool: impl Into<String>, error: McpError) -> FakeServer {
        self.results.insert(tool.into(), Err(error));
        self
    }

    /// Make every call incur `latency` before answering (exercises the per-block timeout later).
    pub fn with_latency(mut self, latency: Duration) -> FakeServer {
        self.latency = Some(latency);
        self
    }

    /// Spawn an instance that is already dead — every call returns [`McpError::Dead`].
    pub fn born_dead(mut self) -> FakeServer {
        self.dead = true;
        self
    }
}

struct FakeMcpInstance {
    tools: Vec<McpTool>,
    results: HashMap<String, Result<McpOutput, McpError>>,
    latency: Option<Duration>,
    dead: bool,
}

#[async_trait]
impl McpInstance for FakeMcpInstance {
    fn tools(&self) -> &[McpTool] {
        &self.tools
    }

    async fn call(&self, tool: &str, _arguments: serde_json::Value) -> Result<McpOutput, McpError> {
        if let Some(latency) = self.latency {
            tokio::time::sleep(latency).await;
        }
        if self.dead {
            return Err(McpError::Dead("scripted server died".to_owned()));
        }
        match self.results.get(tool) {
            Some(result) => result.clone(),
            None => Err(McpError::Protocol {
                code: -32601,
                message: format!("tool not found: {tool}"),
            }),
        }
    }

    async fn shutdown(&self) {}
}

#[cfg(test)]
mod tests {
    use super::{FakeMcpHost, FakeServer};
    use crate::mcp::{ContentBlock, McpError, McpHost, McpOutput, McpServerConfig, McpTool};
    use std::time::Duration;

    fn tool(name: &str) -> McpTool {
        McpTool {
            name: name.to_owned(),
            description: format!("the {name} tool"),
            input_schema: serde_json::json!({ "type": "object" }),
        }
    }

    fn text(body: &str) -> McpOutput {
        McpOutput {
            content: vec![ContentBlock::Text {
                text: body.to_owned(),
            }],
            structured: None,
        }
    }

    #[tokio::test]
    async fn spawns_an_instance_with_the_scripted_catalogue_and_returns_canned_results() {
        let host = FakeMcpHost::new().with(
            "browser",
            FakeServer::new(vec![tool("navigate"), tool("markdown")])
                .returns("markdown", text("# Hello")),
        );
        let instance = host
            .spawn("browser", &McpServerConfig::default())
            .await
            .unwrap();

        let names: Vec<&str> = instance.tools().iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["navigate", "markdown"]);
        assert_eq!(
            instance
                .call("markdown", serde_json::json!({}))
                .await
                .unwrap(),
            text("# Hello")
        );
    }

    #[tokio::test]
    async fn an_unscripted_or_failing_tool_returns_an_error() {
        let host = FakeMcpHost::new().with(
            "browser",
            FakeServer::new(vec![tool("navigate")])
                .fails("navigate", McpError::Tool("navigation failed".to_owned())),
        );
        let instance = host
            .spawn("browser", &McpServerConfig::default())
            .await
            .unwrap();

        // A scripted failure surfaces as its error.
        assert!(matches!(
            instance.call("navigate", serde_json::json!({})).await,
            Err(McpError::Tool(_))
        ));
        // An unscripted tool is a protocol "not found".
        assert!(matches!(
            instance.call("evaluate", serde_json::json!({})).await,
            Err(McpError::Protocol { code: -32601, .. })
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn a_latency_scripted_call_resolves_after_its_delay() {
        let host = FakeMcpHost::new().with(
            "slow",
            FakeServer::new(vec![tool("markdown")])
                .returns("markdown", text("done"))
                .with_latency(Duration::from_secs(5)),
        );
        let instance = host
            .spawn("slow", &McpServerConfig::default())
            .await
            .unwrap();
        // With the clock paused, the call only completes once time advances past the latency.
        assert_eq!(
            instance
                .call("markdown", serde_json::json!({}))
                .await
                .unwrap(),
            text("done")
        );
    }

    #[tokio::test]
    async fn a_dead_instance_fails_every_call() {
        let host = FakeMcpHost::new().with(
            "browser",
            FakeServer::new(vec![tool("markdown")]).born_dead(),
        );
        let instance = host
            .spawn("browser", &McpServerConfig::default())
            .await
            .unwrap();
        assert!(matches!(
            instance.call("markdown", serde_json::json!({})).await,
            Err(McpError::Dead(_))
        ));
    }

    #[tokio::test]
    async fn a_spawn_failure_yields_an_error() {
        let host = FakeMcpHost::new().with("broken", FakeServer::spawn_failure("boom"));
        assert!(matches!(
            host.spawn("broken", &McpServerConfig::default()).await,
            Err(McpError::Spawn(_))
        ));
        // An unregistered server is also a spawn error.
        assert!(matches!(
            host.spawn("absent", &McpServerConfig::default()).await,
            Err(McpError::Spawn(_))
        ));
    }

    #[test]
    fn errors_lead_with_the_mcp_context_prefix() {
        assert!(
            McpError::Dead("x".to_owned())
                .to_string()
                .starts_with("mcp:")
        );
        assert!(
            McpError::Protocol {
                code: -32601,
                message: "nope".to_owned()
            }
            .to_string()
            .starts_with("mcp:")
        );
    }
}
