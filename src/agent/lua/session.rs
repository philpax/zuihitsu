//! Session construction and MCP lifecycle — the methods that set up and tear down the VM's
//! per-conversation state, distinct from the per-block execution in [`super::execute`].

use std::sync::Arc;

use crate::{InstanceFeatures, agent::api_doc::ApiEntry, ids::ConversationId};

use super::Session;

impl Session {
    pub fn new(conversation: ConversationId, features: InstanceFeatures) -> Session {
        let lua = super::sandboxed_lua();
        Session {
            lua,
            conversation,
            mcp: None,
            features,
        }
    }

    /// A VM with the `mcp.<server>.*` projection installed from `catalogue` (the probed, filtered tool
    /// set), with live instances spawned on demand through `host`. The projection global is installed
    /// once here and persists across the session's blocks (the server instances are session-scoped).
    /// Lua-table creation cannot realistically fail at construction, so installation is treated as
    /// infallible, like [`Session::new`].
    pub fn with_mcp(
        conversation: ConversationId,
        host: Arc<dyn crate::mcp::McpHost>,
        catalogue: super::super::mcp_api::McpCatalogue,
        features: InstanceFeatures,
    ) -> Session {
        let lua = super::sandboxed_lua();
        let mcp = Arc::new(super::super::mcp_api::McpSession::new(host, catalogue));
        super::super::mcp_api::install(&lua, &mcp).expect("installing the mcp projection global");
        Session {
            lua,
            conversation,
            mcp: Some(mcp),
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

    pub fn conversation(&self) -> ConversationId {
        self.conversation
    }
}
