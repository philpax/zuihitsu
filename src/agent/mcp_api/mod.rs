//! The `mcp.<server>.<tool>{ ... }` projection: each configured MCP server's tools surfaced into the
//! session VM as one async Lua function apiece (spec §External I/O via MCP).
//!
//! The tool catalogue is learned once, up front, by [`McpCatalogue::probe`] (a startup spawn +
//! `tools/list` + `allow`/`deny` filter per server), so the per-session projection is a **pre-built
//! table of functions** and the same filtered set renders into the system prompt (spec §Allowlisting).
//! The live server *instance* is still spawned lazily on first actual use — most sessions never browse,
//! so most never spawn anything — and the `server → instance` map lives on the [`McpSession`] the VM
//! holds for its whole lifetime. The VM runs its blocks one at a time, but the session is shared across
//! the multi-thread runtime's worker threads, so the map is guarded by a `Mutex`. The `mcp` global is
//! installed once and persists across blocks, like the agent scratchpad.
//!
//! Following the block-API discipline ([`crate::agent::lua`]): the async functions and their futures
//! are `'static` and `Send` (mlua's `send`-feature `create_async_function` requires it), capturing
//! [`Arc`] clones of the session state, and **no `Mutex` guard is ever held across an `.await`**.

mod lua;
mod schema;

#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use mlua::{Lua, Table, Value};
use parking_lot::Mutex;

use crate::{
    agent::api_doc::ApiEntry,
    mcp::{McpError, McpHost, McpInstance, McpServerConfig, McpTool},
    metrics::{observe_mcp_call, observe_mcp_call_error},
};

pub(crate) use lua::{
    build_escape_map, lua_args_to_json, mcp_to_lua, project_output, tool_function,
};
pub(crate) use schema::{filter_tools, tool_to_api_entry};

// Re-export items the test module reaches for through `super::`.
#[cfg(test)]
pub(super) use crate::agent::api_doc::ApiType;
#[cfg(test)]
pub(super) use lua::escape_tool_name;
#[cfg(test)]
pub(super) use schema::InputSchema;

/// The probed, filtered tool catalogues for the configured MCP servers — the single source both the
/// Lua projection and the system-prompt rendering derive from (spec §Allowlisting). Built once by
/// [`McpCatalogue::probe`]; cloned cheaply into each session (the `Arc` entries share the snapshotted
/// tool lists, while the live instances stay per-session).
#[derive(Clone, Debug, Default)]
pub struct McpCatalogue {
    pub(super) servers: BTreeMap<String, Arc<ServerCatalogue>>,
}

/// One server's filtered catalogue: the launch config, the projected tools (post `allow`/`deny`), and
/// the escaped→raw tool-name map, all snapshotted at probe time.
#[derive(Debug)]
pub(super) struct ServerCatalogue {
    config: McpServerConfig,
    tools: Vec<McpTool>,
    escaped_to_raw: HashMap<String, String>,
}

impl McpCatalogue {
    /// Probe every configured server once: spawn it, snapshot `tools/list`, apply its `allow`/`deny`
    /// filter, build the escape map, then shut the probe instance down. A server that fails to spawn is
    /// dropped with a warning — its tools simply never appear (spec §dropped when unavailable). A
    /// filter entry matching no advertised tool, or two tools that escape to the same Lua name, is a
    /// hard error the operator must fix (spec §Allowlisting).
    pub async fn probe(
        host: &dyn McpHost,
        configs: &BTreeMap<String, McpServerConfig>,
    ) -> Result<McpCatalogue, McpError> {
        let mut servers = BTreeMap::new();
        for (name, config) in configs {
            let instance = match host.spawn(name, config).await {
                Ok(instance) => instance,
                Err(error) => {
                    tracing::warn!(server = %name, %error, "mcp: server failed to probe; dropping its tools");
                    continue;
                }
            };
            let context = |message: String| McpError::Spawn(format!("server {name:?}: {message}"));
            let tools = filter_tools(
                instance.tools(),
                config.allow.as_deref(),
                config.deny.as_deref(),
            )
            .map_err(context)?;
            let escaped_to_raw = build_escape_map(&tools).map_err(context)?;
            instance.shutdown().await;
            servers.insert(
                name.clone(),
                Arc::new(ServerCatalogue {
                    config: config.clone(),
                    tools,
                    escaped_to_raw,
                }),
            );
        }
        Ok(McpCatalogue { servers })
    }

    /// How many servers were brought up (the `mcp_servers_up` metric).
    pub fn server_count(&self) -> usize {
        self.servers.len()
    }

    /// The total projected tool count across every server (the `mcp_tools_total` metric).
    pub fn tool_count(&self) -> usize {
        self.servers
            .values()
            .map(|catalogue| catalogue.tools.len())
            .sum()
    }

    /// Each server's name and projected tool count, for the boot log.
    pub fn server_tool_counts(&self) -> Vec<(String, usize)> {
        self.servers
            .iter()
            .map(|(name, catalogue)| (name.clone(), catalogue.tools.len()))
            .collect()
    }

    /// The projected tools as system-prompt API entries (spec §Projected into the system prompt), one
    /// per filtered tool: the escaped Lua call form, its arguments from the JSON-Schema input, and the
    /// tool's own description.
    pub(crate) fn api_entries(&self) -> Vec<ApiEntry> {
        self.servers
            .iter()
            .flat_map(|(server, catalogue)| {
                catalogue
                    .tools
                    .iter()
                    .map(move |tool| tool_to_api_entry(server, tool))
            })
            .collect()
    }
}

/// The VM's MCP state: the host that spawns servers, the probed catalogue, and the lazily-spawned live
/// instances. Held behind an [`Arc`] so the projected Lua functions can share it across threads.
pub(crate) struct McpSession {
    host: Arc<dyn McpHost>,
    catalogue: McpCatalogue,
    instances: Mutex<HashMap<String, Arc<dyn McpInstance>>>,
    /// The server whose tool call is mid-`await`, set for the duration of [`McpSession::call`]'s
    /// network round-trip. If that `await` is cancelled by a block timeout the field is left set, so
    /// [`McpSession::drop_in_flight`] knows which instance the abandoned call left in an undefined
    /// state and must discard.
    in_flight: Mutex<Option<String>>,
    /// Whether the current block attempt has made an MCP call — an un-rollback-able external effect.
    /// Set when [`McpSession::call`] invokes the tool and reset by [`McpSession::begin_block`] at the
    /// start of each attempt, so a timed-out block that touched MCP is surfaced rather than retried
    /// (spec §645).
    block_made_a_call: AtomicBool,
}

impl McpSession {
    pub(crate) fn new(host: Arc<dyn McpHost>, catalogue: McpCatalogue) -> McpSession {
        McpSession {
            host,
            catalogue,
            instances: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(None),
            block_made_a_call: AtomicBool::new(false),
        }
    }

    /// Reset the per-attempt "made a call" latch at the start of a block execution attempt.
    pub(crate) fn begin_block(&self) {
        self.block_made_a_call.store(false, Ordering::SeqCst);
    }

    /// Whether this block attempt has invoked an MCP tool.
    pub(crate) fn block_made_a_call(&self) -> bool {
        self.block_made_a_call.load(Ordering::SeqCst)
    }

    /// The projected tools as system-prompt API entries (forwards to the catalogue).
    pub(crate) fn api_entries(&self) -> Vec<ApiEntry> {
        self.catalogue.api_entries()
    }

    /// Shut every spawned instance down (best-effort), draining the map first so no lock guard is held
    /// across the awaits. Called at session end.
    pub(crate) async fn shutdown(&self) {
        let instances: Vec<Arc<dyn McpInstance>> = self
            .instances
            .lock()
            .drain()
            .map(|(_, instance)| instance)
            .collect();
        for instance in instances {
            instance.shutdown().await;
        }
    }

    /// Run one `mcp.<server>.<escaped_tool>(args)` call: resolve the escaped name to the raw tool from
    /// the catalogue, spawn the server if it is not yet live, marshal the argument table, call, and
    /// project the result. Every failure is a catchable Lua error.
    async fn call(
        &self,
        lua: &Lua,
        server: &str,
        escaped_tool: &str,
        args: Table,
    ) -> mlua::Result<Value> {
        // The projection only installs functions for configured servers, so the catalogue entry exists.
        let catalogue = &self.catalogue.servers[server];
        let raw = catalogue.escaped_to_raw.get(escaped_tool).ok_or_else(|| {
            mcp_to_lua(McpError::UnknownTool {
                server: server.to_string(),
                tool: escaped_tool.to_string(),
            })
        })?;
        let arguments = lua_args_to_json(lua, args)?;
        let instance = self
            .ensure_spawned(server)
            .await
            .map_err(|error| mcp_to_lua(error.with_server(server)))?;
        // Latch that this block engaged MCP — an external effect that bars a silent retry on timeout —
        // and mark this server in-flight across the network round-trip. On a clean return the in-flight
        // marker is cleared below; if a block timeout cancels this `await`, it is left set so the
        // timeout handler can drop the now-undefined instance (see [`drop_in_flight`]).
        self.block_made_a_call.store(true, Ordering::SeqCst);
        *self.in_flight.lock() = Some(server.to_owned());
        let started = std::time::Instant::now();
        let result = instance.call(raw, arguments).await;
        let elapsed = started.elapsed();
        self.in_flight.lock().take();
        // Observe the MCP call's latency/throughput at the chokepoint, so "where did the turn's
        // time go" separates a slow tool from a slow inference (spec §Observability). A failure is
        // still a call (counted), and counted again as an error.
        observe_mcp_call(elapsed);
        let output = result.map_err(|error| {
            observe_mcp_call_error();
            mcp_to_lua(error.with_call(server, escaped_tool))
        })?;
        project_output(lua, output)
    }

    /// Discard the instance whose call was cut off by a block timeout, if any. The abandoned call
    /// left the server's session-side state — a browser page, an open cursor — undefined, so the
    /// instance must not be reused: removing it from the map drops the last `Arc`, closing the
    /// subprocess, and the next call to that server spawns a fresh one. A no-op when nothing is in
    /// flight (a clean call clears the marker itself).
    pub(crate) fn drop_in_flight(&self) {
        if let Some(server) = self.in_flight.lock().take() {
            self.instances.lock().remove(&server);
        }
    }

    /// The live instance for `server`, spawning it on first use. The lock that checks the map is dropped
    /// before the spawn `await` (a `parking_lot` guard is not held across a suspension point).
    async fn ensure_spawned(&self, server: &str) -> Result<Arc<dyn McpInstance>, McpError> {
        let existing = self.instances.lock().get(server).cloned();
        if let Some(instance) = existing {
            return Ok(instance);
        }
        let instance: Arc<dyn McpInstance> = Arc::from(
            self.host
                .spawn(server, &self.catalogue.servers[server].config)
                .await?,
        );
        self.instances
            .lock()
            .insert(server.to_owned(), instance.clone());
        Ok(instance)
    }
}

/// Install the `mcp` global: a table per configured server, each holding one async function per filtered
/// tool keyed by its escaped name. An unconfigured `mcp.<x>` is `nil`, so calling it is a plain "attempt
/// to call a nil value"; an unfiltered tool simply has no function.
pub(crate) fn install(lua: &Lua, mcp: &Arc<McpSession>) -> mlua::Result<()> {
    let mcp_table = lua.create_table()?;
    for (server, catalogue) in &mcp.catalogue.servers {
        let server_table = lua.create_table()?;
        for escaped in catalogue.escaped_to_raw.keys() {
            server_table.set(
                escaped.as_str(),
                tool_function(lua, mcp, server, escaped.clone())?,
            )?;
        }
        mcp_table.set(server.as_str(), server_table)?;
    }
    lua.globals().set("mcp", mcp_table)?;
    Ok(())
}
