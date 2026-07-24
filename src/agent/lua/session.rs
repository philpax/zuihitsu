//! Session construction and MCP lifecycle — the methods that set up and tear down the VM's
//! per-conversation state, distinct from the per-block execution in [`crate::agent::lua::execute`].

use std::sync::Arc;

use crate::{
    InstanceFeatures,
    agent::{
        api_doc::ApiEntry,
        lua::{Session, sandboxed_lua},
        mcp_api::{McpCatalogue, McpSession, install},
    },
    ids::ConversationId,
    web::WebClient,
};

impl Session {
    pub fn new(conversation: Option<ConversationId>, features: InstanceFeatures) -> Session {
        let lua = sandboxed_lua();
        Session {
            lua,
            conversation,
            mcp: None,
            web: None,
            features,
        }
    }

    /// Attach the web fetcher to this session, chainably — the `web.markdown` projection is installed
    /// per block when it is set and the `browsing` feature is on. Called by the instance's `mint_vm`
    /// after choosing the base constructor, so a session with or without MCP both gain web the same way.
    pub fn with_web(mut self, web: Option<WebClient>) -> Session {
        self.web = web;
        self
    }

    /// A VM with the `mcp.<server>.*` projection installed from `catalogue` (the probed, filtered tool
    /// set), with live instances spawned on demand through `host`. The projection global is installed
    /// once here and persists across the session's blocks (the server instances are session-scoped).
    /// Lua-table creation cannot realistically fail at construction, so installation is treated as
    /// infallible, like [`Session::new`].
    pub fn with_mcp(
        conversation: Option<ConversationId>,
        host: Arc<dyn crate::mcp::McpHost>,
        catalogue: McpCatalogue,
        features: InstanceFeatures,
    ) -> Session {
        let lua = sandboxed_lua();
        let mcp = Arc::new(McpSession::new(host, catalogue));
        install(&lua, &mcp).expect("installing the mcp projection global");
        Session {
            lua,
            conversation,
            mcp: Some(mcp),
            web: None,
            features,
        }
    }

    /// The features this session enables — the gate the block API registration and the API reference
    /// both read, so the runtime surface and the prompt's description stay in lockstep.
    pub fn features(&self) -> InstanceFeatures {
        self.features
    }

    /// The configured MCP tools as system-prompt API entries — empty when no host is configured. The
    /// turn assembles these alongside the build-derived Lua API into the prompt's API description.
    pub fn mcp_api_entries(&self) -> Vec<ApiEntry> {
        self.mcp
            .as_ref()
            .map(|mcp| mcp.api_entries())
            .unwrap_or_default()
    }

    /// Tear down the session's MCP instances (close stdin, wait, kill on a grace timeout), best-effort.
    /// A no-op when no MCP host is configured. Called when the session ends.
    pub async fn shutdown_mcp(&self) {
        if let Some(mcp) = &self.mcp {
            mcp.shutdown().await;
        }
    }

    /// Drop the MCP instance whose call a block timeout just cut off, if any (the abandoned call left
    /// its server-side state undefined). A no-op when no host is configured or nothing was in flight.
    pub(super) fn drop_in_flight_mcp(&self) {
        if let Some(mcp) = &self.mcp {
            mcp.drop_in_flight();
        }
    }

    /// Reset the per-attempt "this block made an MCP call" latch before an execution attempt.
    pub(super) fn begin_mcp_block(&self) {
        if let Some(mcp) = &self.mcp {
            mcp.begin_block();
        }
    }

    /// Whether this block has made an MCP call this attempt — an external effect that cannot be rolled
    /// back, so its timeout is surfaced rather than retried (spec §645). Always `false` without a host.
    pub(super) fn block_made_mcp_call(&self) -> bool {
        self.mcp.as_ref().is_some_and(|mcp| mcp.block_made_a_call())
    }

    /// The conversation this session's blocks write in, or `None` for the console sandbox. A live turn
    /// always has one, so the turn runner unwraps it.
    pub fn conversation(&self) -> Option<ConversationId> {
        self.conversation
    }
}
