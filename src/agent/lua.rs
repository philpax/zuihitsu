//! The Lua execution layer: one VM per session, and the block as an atomic transaction.
//!
//! A block runs a Lua script through the object/method memory API (spec §Lua API). Side-effect
//! events are *buffered* during execution and committed atomically at the end — appended to the log
//! (the durable commit point), then applied to the graph. Reads within a block see the graph
//! overlaid with the block's own pending writes (read-your-writes). The value of the script's final
//! expression is rendered to text and recorded on the block's `LuaExecuted` event, so faithful
//! replay feeds the model exactly the string it saw. A runtime error or an explicit
//! `block.abort(reason)` discards the buffer and records the terminal cause instead.
//!
//! The API is installed per block via `mlua`'s `scope`, so its functions can borrow the block's
//! [`MemoryBlock`] transaction for the block's duration. The transaction owns the buffer, the touched
//! set, and every write invariant; this layer is a thin wrapper that translates script calls into
//! method calls — it never touches the buffer, the events, or the visibility rules directly. Agent
//! scratchpad globals persist on the VM across blocks within the session; the API is re-installed
//! each block.

use std::{collections::HashMap, sync::Arc, time::Instant};

use mlua::{Lua, LuaSerdeExt, Table, Value};
use parking_lot::Mutex;
use serde::Deserialize;
use tokio::sync::OwnedMutexGuard;
use ulid::Ulid;

use crate::{
    engine::{Engine, MemoryLocks},
    event::{EventPayload, TerminalCause},
    graph::GraphError,
    ids::{ConversationId, EntryId, MemoryId, TurnId},
    memory::{
        memory_block::{AppendOptions, BlockEffects, EntryRef, MemoryBlock, MemoryError},
        search::{SearchQuery, search},
    },
    settings::Settings,
    store::StoreError,
    vocabulary::{RelationName, TagName},
};

use super::{
    BlockContext,
    api_doc::{ApiEntry, ApiType, enum_of, object},
};

/// One conversation's VM. Globals persist across the session's blocks; the memory API is installed
/// fresh per block, while the MCP projection (when configured) is installed once and persists like the
/// agent scratchpad.
pub struct Session {
    lua: Lua,
    conversation: ConversationId,
    /// The session's MCP state — the host, configured servers, and lazily-spawned instances backing the
    /// `mcp.<server>.*` projection — or `None` when no host is configured.
    mcp: Option<std::sync::Arc<super::mcp_api::McpSession>>,
}

/// The result of executing one block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockOutcome {
    /// The block committed; `result` is the rendered value of its final expression.
    Committed { result: String },
    /// The block ended without committing (its buffer was discarded), for this reason.
    Terminated(TerminalCause),
}

impl Session {
    pub fn new(conversation: ConversationId) -> Session {
        Session {
            lua: Lua::new(),
            conversation,
            mcp: None,
        }
    }

    /// A VM with the `mcp.<server>.*` projection installed from `catalogue` (the probed, filtered tool
    /// set), with live instances spawned on demand through `host`. The projection global is installed
    /// once here and persists across the session's blocks (the server instances are session-scoped).
    /// Lua-table creation cannot realistically fail at construction, so installation is treated as
    /// infallible, like [`Session::new`].
    pub fn with_mcp(
        conversation: ConversationId,
        host: std::sync::Arc<dyn crate::mcp::McpHost>,
        catalogue: super::mcp_api::McpCatalogue,
    ) -> Session {
        let lua = Lua::new();
        let mcp = std::sync::Arc::new(super::mcp_api::McpSession::new(host, catalogue));
        super::mcp_api::install(&lua, &mcp).expect("installing the mcp projection global");
        Session {
            lua,
            conversation,
            mcp: Some(mcp),
        }
    }

    /// The configured MCP tools as system-prompt API entries — empty when no host is configured. The
    /// turn assembles these alongside the build-derived Lua API into the prompt's API description.
    pub fn mcp_api_entries(&self) -> Vec<super::api_doc::ApiEntry> {
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
    fn drop_in_flight_mcp(&self) {
        if let Some(mcp) = &self.mcp {
            mcp.drop_in_flight();
        }
    }

    /// Reset the per-attempt "this block made an MCP call" latch before an execution attempt.
    fn begin_mcp_block(&self) {
        if let Some(mcp) = &self.mcp {
            mcp.begin_block();
        }
    }

    /// Whether this block has made an MCP call this attempt — an external effect that cannot be rolled
    /// back, so its timeout is surfaced rather than retried (spec §645). Always `false` without a host.
    fn block_made_mcp_call(&self) -> bool {
        self.mcp.as_ref().is_some_and(|mcp| mcp.block_made_a_call())
    }

    pub fn conversation(&self) -> ConversationId {
        self.conversation
    }

    /// Execute one block as a transaction. On a clean run, the buffered side effects plus a
    /// `LuaExecuted` commit together; on error or abort, only a `LuaExecuted` recording the terminal
    /// cause is written. The graph is brought up to log-head afterward either way.
    ///
    /// The block acquires the lock on each memory it touches and holds it to block end (spec
    /// §Concurrency → per-memory mutual exclusion), so a concurrent block in another conversation
    /// serializes on a shared memory. If the block outruns its time budget — stuck on slow external
    /// I/O or on a lock-wait — it aborts, releases its locks, and **retries from scratch**, bounded by
    /// `max_block_attempts`; the exception is a block that has already made an MCP call, whose external
    /// effect cannot be rolled back, so its timeout surfaces as a terminal error with no retry (spec
    /// §645). A retried block re-runs the Lua from scratch; the VM globals (the scratchpad) persist and
    /// are not transactional, so a non-idempotent scratchpad write is observable across attempts.
    pub async fn execute(
        &self,
        engine: &Arc<Engine>,
        context: &BlockContext,
        script: &str,
    ) -> Result<BlockOutcome, LuaError> {
        let manager = engine.memory_locks.clone();
        let max_attempts = context.max_block_attempts.max(1);
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            // Each attempt is a fresh transaction over a fresh lock set, bundled with the infra-error
            // slot and the lock registry as the one [`BlockApi`] seam the install helpers and their
            // `'static` async closures share. The block owns the buffer and the write invariants;
            // `lock_set` holds the owned per-memory guards until block end.
            let api = BlockApi {
                block: Arc::new(Mutex::new(MemoryBlock::new(
                    engine.clone(),
                    context.teller.clone(),
                    context.authority,
                    self.conversation,
                    context.present_set.clone(),
                )?)),
                infra: Arc::new(Mutex::new(None)),
                lock_set: Arc::new(Mutex::new(LockSet::default())),
                manager: manager.clone(),
            };

            // The handle metatable and its methods table back every memory handle the API mints; the
            // entry metatable backs the addressable content-entry handles that `mem:append` /
            // `mem:entries` / `mem:history` return (text-rendering, so reading stays ergonomic).
            let methods = self.lua.create_table().map_err(LuaError::Vm)?;
            let metatable = self.lua.create_table().map_err(LuaError::Vm)?;
            metatable
                .set("__index", methods.clone())
                .map_err(LuaError::Vm)?;
            let entry_metatable = self.entry_metatable().map_err(LuaError::Vm)?;

            // Reset the per-attempt "made an MCP call" latch, so the no-retry decision below reflects
            // this attempt only.
            self.begin_mcp_block();

            // Installing the API is our-side setup: a failure here is a bug, not an agent-visible
            // outcome.
            self.install_block_api(&api, &methods, &metatable, &entry_metatable)
                .map_err(LuaError::Vm)?;

            // The agent-visible outcome: the rendered final value, or the runtime error/abort that
            // ended the script, bounded by the block's time budget. The block's memory functions only
            // hold their parking_lot guards transiently, never across this suspension point. `started`
            // times this attempt's eval for the debugger's turn timeline (the final attempt's, since a
            // retry restarts it).
            let started = Instant::now();
            let timed = tokio::time::timeout(
                context.block_timeout,
                self.lua.load(script).eval_async::<Value>(),
            )
            .await;

            let Ok(evaluated) = timed else {
                // Timed out. Release the locks (so a retry, or another conversation, can take them) and
                // drop the in-flight MCP instance — its session-side state is now undefined.
                release_locks(&api.lock_set);
                self.drop_in_flight_mcp();
                if self.block_made_mcp_call() {
                    // An external effect happened this attempt; surface the timeout, do not retry.
                    let cause = timed_out_cause(context.block_timeout, None);
                    return self
                        .commit_terminal(engine, context, script, &api.block, cause, started);
                }
                if attempt >= max_attempts {
                    let cause = timed_out_cause(context.block_timeout, Some(attempt));
                    return self
                        .commit_terminal(engine, context, script, &api.block, cause, started);
                }
                // Abort-and-retry: the buffer emitted nothing, so a fresh attempt is the only trace.
                continue;
            };
            let evaluated = evaluated.map(|value| render(&self.lua, &value));

            // An infrastructure failure during the block (a graph read) takes precedence over the
            // script's apparent outcome: it bubbles up, discarding the buffer and releasing the locks,
            // rather than reaching the agent.
            if let Some(graph_error) = api.infra.lock().take() {
                release_locks(&api.lock_set);
                return Err(LuaError::Graph(graph_error));
            }

            // Drain the effects through the lock and commit. Locks are held through the commit so a
            // concurrent block sees consistent state, then released once the block is done.
            let BlockEffects {
                events,
                touched,
                aborted,
            } = api.block.lock().take_effects();
            let outcome = match evaluated {
                Ok(result) => {
                    let mut events = events;
                    events.push(self.lua_executed(
                        context.turn_id,
                        script,
                        Some(result.clone()),
                        touched,
                        None,
                        started.elapsed().as_millis() as u64,
                    ));
                    self.finish(engine, events, BlockOutcome::Committed { result })
                }
                Err(error) => {
                    // Discard the buffer; record only what the agent saw — the terminal cause.
                    let cause = match aborted {
                        Some(reason) => TerminalCause::Aborted(reason),
                        None => TerminalCause::Error(error.to_string()),
                    };
                    let event = self.lua_executed(
                        context.turn_id,
                        script,
                        None,
                        touched,
                        Some(cause.clone()),
                        started.elapsed().as_millis() as u64,
                    );
                    self.finish(engine, vec![event], BlockOutcome::Terminated(cause))
                }
            };
            release_locks(&api.lock_set);
            return outcome;
        }
    }

    /// Commit a block's terminal record (a discarded buffer, the touched set kept for the audit) and
    /// bring the graph to head — the shared tail of the timeout-give-up and no-retry-after-MCP paths.
    fn commit_terminal(
        &self,
        engine: &Engine,
        context: &BlockContext,
        script: &str,
        block: &Arc<Mutex<MemoryBlock>>,
        cause: TerminalCause,
        started: Instant,
    ) -> Result<BlockOutcome, LuaError> {
        let BlockEffects { touched, .. } = block.lock().take_effects();
        let event = self.lua_executed(
            context.turn_id,
            script,
            None,
            touched,
            Some(cause.clone()),
            started.elapsed().as_millis() as u64,
        );
        self.finish(engine, vec![event], BlockOutcome::Terminated(cause))
    }

    /// Install the per-block memory API as `'static` async Lua functions over the shared [`BlockApi`]
    /// seam. Before its operation, each function acquires the lock on every memory it touches and holds
    /// the owned guard (in `api.lock_set`) to block end, so a concurrent block in another conversation
    /// serializes on a shared memory (spec §Concurrency). A graph-read failure is routed to `api.infra`
    /// (infrastructure, bubbled up); a teachable violation becomes the Lua runtime error the agent sees.
    /// The handle `metatable`/`methods` tables back every minted memory handle. The registration is
    /// split table by table so each group stays legible.
    fn install_block_api(
        &self,
        api: &BlockApi,
        methods: &Table,
        metatable: &Table,
        entry_metatable: &Table,
    ) -> mlua::Result<()> {
        self.install_handle_methods(api, methods, entry_metatable)?;
        let globals = self.lua.globals();
        globals.set("memory", self.memory_table(api, metatable)?)?;
        globals.set("block", self.block_table(api)?)?;
        globals.set("context", self.context_table(api, metatable)?)?;
        globals.set("calendar", self.calendar_table(api, metatable)?)?;
        Ok(())
    }

    /// The `mem:*` handle methods (`append`, `entries`, `history`, `supersede`, `link`, `unlink`) on
    /// the metatable's `methods` table. Each acts on the handle passed as `this`. `entry_metatable`
    /// backs the entry handles the content reads and `append` return.
    fn install_handle_methods(
        &self,
        api: &BlockApi,
        methods: &Table,
        entry_metatable: &Table,
    ) -> mlua::Result<()> {
        // mem:append(text[, opts]) — `opts` is the typed override struct, deserialized from the table.
        // Locks the target memory before writing it. Returns the new entry as an addressable handle.
        methods.set(
            "append",
            self.lua.create_async_function({
                let api = api.clone();
                let entry_metatable = entry_metatable.clone();
                move |lua, (this, text, opts): (Table, String, Value)| {
                    let api = api.clone();
                    let entry_metatable = entry_metatable.clone();
                    async move {
                        let id = handle_id(&this)?;
                        api.lock(id).await;
                        let opts: AppendOptions = if opts.is_nil() {
                            AppendOptions::default()
                        } else {
                            lua.from_value(opts)?
                        };
                        let entry_id = api
                            .block
                            .lock()
                            .append(id, &text, opts)
                            .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                        make_entry_handle(&lua, entry_id, &text, &entry_metatable)
                    }
                }
            })?,
        )?;

        // mem:entries() — the memory's live entries across its merged identity plus pending writes,
        // each an addressable entry handle that renders as its text. A traversing read, so it locks the
        // whole `same_as` class before reading (spec §Concurrency → class-wide locking).
        methods.set(
            "entries",
            self.lua.create_async_function({
                let api = api.clone();
                let entry_metatable = entry_metatable.clone();
                move |lua, this: Table| {
                    let api = api.clone();
                    let entry_metatable = entry_metatable.clone();
                    async move {
                        let id = handle_id(&this)?;
                        api.lock_class(id).await?;
                        let entries = api
                            .block
                            .lock()
                            .entries(id)
                            .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                        make_entry_handle_list(&lua, entries, &entry_metatable)
                    }
                }
            })?,
        )?;

        // mem:history() — the memory's entries including superseded ones (spec §Per-memory history),
        // the read where history is the point and the live filter is bypassed. Like `entries`, a
        // class-traversing read.
        methods.set(
            "history",
            self.lua.create_async_function({
                let api = api.clone();
                let entry_metatable = entry_metatable.clone();
                move |lua, this: Table| {
                    let api = api.clone();
                    let entry_metatable = entry_metatable.clone();
                    async move {
                        let id = handle_id(&this)?;
                        api.lock_class(id).await?;
                        let entries = api
                            .block
                            .lock()
                            .history(id)
                            .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                        make_entry_handle_list(&lua, entries, &entry_metatable)
                    }
                }
            })?,
        )?;

        // mem:supersede(old, new) — correct or retract a fact: mark `old` superseded by `new` (both
        // entry handles read from this memory). Locks the whole class, since it validates against and
        // mutates the merged identity's entries.
        methods.set(
            "supersede",
            self.lua.create_async_function({
                let api = api.clone();
                move |_, (this, old, new): (Table, Table, Table)| {
                    let api = api.clone();
                    async move {
                        let id = handle_id(&this)?;
                        let (old, new) = (entry_handle_id(&old)?, entry_handle_id(&new)?);
                        api.lock_class(id).await?;
                        api.block
                            .lock()
                            .supersede(id, old, new)
                            .map_err(|error| route_error(error, &mut api.infra.lock()))
                    }
                }
            })?,
        )?;

        // mem:link(relation, other) / mem:unlink(relation, other) — flag (or clear) a relation such
        // as `active_in`, locking both endpoints. The script names the relation as a string; it is
        // recognized into its typed [`RelationName`] here, at the wrapper boundary.
        methods.set(
            "link",
            self.lua.create_async_function({
                let api = api.clone();
                move |_, (this, relation, other): (Table, String, Table)| {
                    let api = api.clone();
                    async move {
                        let (from, to) = (handle_id(&this)?, handle_id(&other)?);
                        api.lock_all([from, to]).await;
                        api.block
                            .lock()
                            .link(from, to, RelationName::new(relation))
                            .map_err(|error| route_error(error, &mut api.infra.lock()))
                    }
                }
            })?,
        )?;
        methods.set(
            "unlink",
            self.lua.create_async_function({
                let api = api.clone();
                move |_, (this, relation, other): (Table, String, Table)| {
                    let api = api.clone();
                    async move {
                        let (from, to) = (handle_id(&this)?, handle_id(&other)?);
                        api.lock_all([from, to]).await;
                        api.block
                            .lock()
                            .unlink(from, to, RelationName::new(relation))
                            .map_err(|error| route_error(error, &mut api.infra.lock()))
                    }
                }
            })?,
        )?;

        Ok(())
    }

    /// The metatable backing entry handles: `__tostring` and `__concat` render the handle as its
    /// `text`, so a content read stays ergonomic (printable, concatenable) while the handle remains an
    /// addressable entry for `mem:supersede`.
    fn entry_metatable(&self) -> mlua::Result<Table> {
        let metatable = self.lua.create_table()?;
        metatable.set(
            "__tostring",
            self.lua
                .create_function(|_, this: Table| this.get::<String>("text"))?,
        )?;
        metatable.set(
            "__concat",
            self.lua
                .create_function(|lua, (left, right): (Value, Value)| {
                    Ok(format!(
                        "{}{}",
                        value_text(lua, &left)?,
                        value_text(lua, &right)?
                    ))
                })?,
        )?;
        Ok(metatable)
    }

    /// The metatable backing `memory.search` result objects: `__tostring` renders one as a readable
    /// line (name, score, description, and any teller-private marker), so returning the result list
    /// reads back as text rather than `<table>` while each result keeps its fields for the agent to
    /// inspect (`result.name` to fetch, `result.score` to weigh).
    fn search_result_metatable(&self) -> mlua::Result<Table> {
        let metatable = self.lua.create_table()?;
        metatable.set(
            "__tostring",
            self.lua.create_function(|_, this: Table| {
                let name: String = this.get("name")?;
                let description: String = this.get("description")?;
                let score: f32 = this.get("score")?;
                let marker: Option<String> = this.get("marker")?;
                let mut line = format!("{name} (score {score:.2})");
                if !description.is_empty() {
                    line.push_str(" — ");
                    line.push_str(&description);
                }
                if let Some(marker) = marker {
                    line.push(' ');
                    line.push_str(&marker);
                }
                Ok(line)
            })?,
        )?;
        Ok(metatable)
    }

    /// The `memory` global: `create` and `get`, both of which mint handles (hence the metatable).
    fn memory_table(&self, api: &BlockApi, metatable: &Table) -> mlua::Result<Table> {
        let memory = self.lua.create_table()?;
        // memory.create(name[, content]) — create a memory and optionally its first entry, then lock
        // the freshly-minted id (uncontended — no other block knows it yet).
        memory.set(
            "create",
            self.lua.create_async_function({
                let api = api.clone();
                let metatable = metatable.clone();
                move |lua, (name, content): (String, Option<String>)| {
                    let api = api.clone();
                    let metatable = metatable.clone();
                    async move {
                        let id = api
                            .block
                            .lock()
                            .create(&name, content.as_deref())
                            .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                        api.lock(id).await;
                        make_handle(&lua, id, &metatable)
                    }
                }
            })?,
        )?;
        // memory.get(name) — resolve through the block's pending creates, then the graph, locking the
        // resolved stub.
        memory.set(
            "get",
            self.lua.create_async_function({
                let api = api.clone();
                let metatable = metatable.clone();
                move |lua, name: String| {
                    let api = api.clone();
                    let metatable = metatable.clone();
                    async move {
                        let resolved = api
                            .block
                            .lock()
                            .get(&name)
                            .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                        match resolved {
                            Some(id) => {
                                api.lock(id).await;
                                Ok(Value::Table(make_handle(&lua, id, &metatable)?))
                            }
                            None => Ok(Value::Nil),
                        }
                    }
                }
            })?,
        )?;
        // memory.search(query[, opts]) — semantic + lexical recall over the agent's whole memory,
        // visibility-filtered against who is present (a teller-private hit only surfaces while its
        // teller is here, with a marker). Embeds the query off any lock, then ranks under a brief read
        // lock. Returns a list of result objects (`{ name, description, score, marker? }`), best first;
        // each prints as a readable line so `return memory.search(...)` reads back the results rather
        // than `<table>`.
        let result_metatable = self.search_result_metatable()?;
        memory.set(
            "search",
            self.lua.create_async_function({
                let api = api.clone();
                let result_metatable = result_metatable.clone();
                move |lua, (query, opts): (String, Value)| {
                    let api = api.clone();
                    let result_metatable = result_metatable.clone();
                    async move {
                        let (engine, present_set) = api.block.lock().retrieval_handle();
                        let opts: SearchOpts = if opts.is_nil() {
                            SearchOpts::default()
                        } else {
                            lua.from_value(opts)?
                        };
                        let rows = run_memory_search(&engine, &present_set, &query, &opts)
                            .await
                            .map_err(mlua::Error::RuntimeError)?;
                        let list = lua.create_table()?;
                        for (index, row) in rows.into_iter().enumerate() {
                            let table = lua.create_table()?;
                            table.set("name", row.name)?;
                            table.set("description", row.description)?;
                            table.set("score", row.score)?;
                            if let Some(marker) = row.marker {
                                table.set("marker", marker)?;
                            }
                            table.set_metatable(Some(result_metatable.clone()))?;
                            list.set(index + 1, table)?;
                        }
                        Ok(Value::Table(list))
                    }
                }
            })?,
        )?;
        Ok(memory)
    }

    /// The `block` global: `abort(reason)`, which discards the buffer and ends the block. It touches no
    /// memory, so it stays a synchronous function and takes no lock.
    fn block_table(&self, api: &BlockApi) -> mlua::Result<Table> {
        let block_tbl = self.lua.create_table()?;
        block_tbl.set(
            "abort",
            self.lua.create_function({
                let block = api.block.clone();
                move |_, reason: Option<String>| {
                    block.lock().abort(reason);
                    Err::<(), _>(mlua::Error::RuntimeError("block aborted".to_owned()))
                }
            })?,
        )?;
        Ok(block_tbl)
    }

    /// The `context` global: `current()`, the current conversation's `context/*` memory (its
    /// `#confidential` tag tells the agent whether the room is confidential), or nil if there is none.
    /// The resolved context memory is locked like any other touched memory.
    fn context_table(&self, api: &BlockApi, metatable: &Table) -> mlua::Result<Table> {
        let context = self.lua.create_table()?;
        context.set(
            "current",
            self.lua.create_async_function({
                let api = api.clone();
                let metatable = metatable.clone();
                move |lua, ()| {
                    let api = api.clone();
                    let metatable = metatable.clone();
                    async move {
                        let current = api.block.lock().current_context();
                        match current {
                            Some(id) => {
                                api.lock(id).await;
                                Ok(Value::Table(make_handle(&lua, id, &metatable)?))
                            }
                            None => Ok(Value::Nil),
                        }
                    }
                }
            })?,
        )?;
        Ok(context)
    }

    /// The `calendar` global: `upcoming`, `on`, and `recurring`, each returning a list of memory
    /// handles, soonest first. Unlike the brief's `<upcoming/>` block these are the agent's own
    /// queries and are not visibility-filtered (like `mem:entries`, the agent sees its whole memory).
    /// Strict locking: each returned memory is locked, since the query read (and touched) it.
    fn calendar_table(&self, api: &BlockApi, metatable: &Table) -> mlua::Result<Table> {
        let calendar = self.lua.create_table()?;
        calendar.set(
            "upcoming",
            self.lua.create_async_function({
                let api = api.clone();
                let metatable = metatable.clone();
                move |lua, opts: Option<Table>| {
                    let api = api.clone();
                    let metatable = metatable.clone();
                    async move {
                        let within: Option<String> = match opts {
                            Some(table) => table.get("within")?,
                            None => None,
                        };
                        let ids = api
                            .block
                            .lock()
                            .upcoming(within.as_deref())
                            .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                        api.lock_all(ids.iter().copied()).await;
                        make_handle_list(&lua, ids, &metatable)
                    }
                }
            })?,
        )?;
        calendar.set(
            "on",
            self.lua.create_async_function({
                let api = api.clone();
                let metatable = metatable.clone();
                move |lua, date: String| {
                    let api = api.clone();
                    let metatable = metatable.clone();
                    async move {
                        let ids = api
                            .block
                            .lock()
                            .on(&date)
                            .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                        api.lock_all(ids.iter().copied()).await;
                        make_handle_list(&lua, ids, &metatable)
                    }
                }
            })?,
        )?;
        calendar.set(
            "recurring",
            self.lua.create_async_function({
                let api = api.clone();
                let metatable = metatable.clone();
                move |lua, ()| {
                    let api = api.clone();
                    let metatable = metatable.clone();
                    async move {
                        let ids = api
                            .block
                            .lock()
                            .recurring()
                            .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                        api.lock_all(ids.iter().copied()).await;
                        make_handle_list(&lua, ids, &metatable)
                    }
                }
            })?,
        )?;
        Ok(calendar)
    }

    fn lua_executed(
        &self,
        turn_id: TurnId,
        script: &str,
        result: Option<String>,
        touched: Vec<MemoryId>,
        terminal_cause: Option<TerminalCause>,
        duration_ms: u64,
    ) -> EventPayload {
        EventPayload::LuaExecuted {
            conversation: self.conversation,
            turn_id,
            script: script.to_owned(),
            result,
            touched,
            terminal_cause,
            duration_ms,
        }
    }

    /// Append the block's events (the durable commit point), bring the graph up to head, and return.
    fn finish(
        &self,
        engine: &Engine,
        events: Vec<EventPayload>,
        outcome: BlockOutcome,
    ) -> Result<BlockOutcome, LuaError> {
        let now = engine.clock.now();
        engine.store.lock().append(now, events)?;
        // Two guards at once: graph (written) before store (read), per the lock-ordering rule.
        let mut graph = engine.graph.lock();
        graph.materialize_from(engine.store.lock().as_ref())?;
        Ok(outcome)
    }
}

/// The agent-facing Lua API, as a typed catalogue. Defined here, beside the functions installed in
/// [`Session::execute`], so the prompt and the implementation cannot drift: changing a function
/// means changing its entry right next to it. Rendered into the system prompt's API description
/// through [`crate::agent::api_doc::render`] — the same renderer MCP tools project through (spec §System
/// prompt → API description).
pub fn api_reference() -> Vec<ApiEntry> {
    use ApiEntry as AE;
    use ApiType as AT;

    let create = AE::new("memory.create")
        .description("Create a memory, optionally with a first content entry.")
        .required(
            "name",
            AT::String,
            "the namespaced handle, e.g. \"person/<name>\" or \"topic/<subject>\". Names match \
             exactly (case-sensitive), so prefer lowercase — \"person/dave\", not \"person/Dave\" — \
             to avoid splitting one subject across casings",
        )
        .optional("content", AT::String, "an optional first content entry")
        .returns(AT::Handle);

    let get = AE::new("memory.get")
        .description(
            "Fetch a memory by name. Read a merged identity through its canonical person/ handle, \
             not a per-platform stub. The name must match exactly (case-sensitive); if a lookup \
             returns nil, suspect the casing before creating a new memory.",
        )
        .required("name", AT::String, "the memory's handle")
        .returns(AT::Handle.optional());

    let search = AE::new("memory.search")
        .description(
            "Recall memories by meaning and wording, across your whole memory, ranked best-first. \
             Results are filtered to what may surface to who is present, so a teller-private aside \
             appears only while its teller is here (with a marker noting it). Each result is a table \
             { name, description, score, marker? } — fetch a name with memory.get to read more.",
        )
        .required("query", AT::String, "what to look for, in natural language")
        .optional(
            "opts",
            object()
                .optional(
                    "namespace",
                    AT::String,
                    "restrict to a name prefix, e.g. \"person/\"",
                )
                .optional(
                    "tags",
                    AT::String.list(),
                    "tags to prefer; a result carrying more of them ranks higher",
                )
                .optional("limit", AT::Integer, "how many results to return (default 8)"),
            "options",
        )
        .returns(AT::Object(Vec::new()).list());

    let append = AE::new("mem:append")
        .description(
            "Append a content entry. By default it is attributed to the current speaker, and an \
             aside about someone else defaults private to that speaker. When you record an entry \
             about a person as your own observation (a synthesis or a flush), there is no default — \
             you must set its visibility yourself, public or private.",
        )
        .required("text", AT::String, "the entry text")
        .optional(
            "opts",
            object()
                .optional(
                    "by_agent",
                    AT::Boolean,
                    "record it as your own observation instead of the speaker's",
                )
                .optional(
                    "visibility",
                    enum_of(["public", "private"]),
                    "force the visibility; required for an entry you author about a person",
                )
                .optional(
                    "occurred_at",
                    object(),
                    "when the fact is about a real-world time (distinct from now): a tagged table, \
                     one of { instant = <ms> }, { day = \"YYYY-MM-DD\" }, \
                     { range = { start = <ms>, end = <ms> } }, \
                     { approx = { center = <ms>, fuzz_days = <n> } }, { recurring = \"<rrule>\" }, \
                     or { before_after = { dir = \"before\" | \"after\", anchor = \"event/...\" } }",
                ),
            "overrides",
        )
        .returns(AT::Entry);

    let entries = AE::new("mem:entries")
        .description(
            "The memory's live content entries, across its whole merged identity. Each is an entry \
             object — read its text with entry.text (it also prints as its text), and pass the \
             object itself to mem:supersede to replace it. Hold onto the object if you intend to \
             supersede it.",
        )
        .returns(AT::Entry.list());

    let history = AE::new("mem:history")
        .description(
            "The memory's entries including superseded ones, oldest first — the full record, where \
             mem:entries shows only the live ones. Each is an entry object (entry.text for its \
             text).",
        )
        .returns(AT::Entry.list());

    let supersede = AE::new("mem:supersede")
        .description(
            "Correct or retract a fact: mark an old entry superseded by a new one. Append the \
             correction first to get the new entry object, then call supersede with the old entry \
             object (from mem:entries) and the new one. The old entry drops from live reads but \
             stays in mem:history.",
        )
        .required(
            "old",
            AT::Entry,
            "the entry object being replaced (from mem:entries)",
        )
        .required(
            "new",
            AT::Entry,
            "the entry object that replaces it (from mem:append)",
        );

    let link = AE::new("mem:link")
        .description(
            "Link this memory to another under a registered relation. Use it to flag a still-open \
             thread active_in the current context, so it carries into the next session across a \
             compaction.",
        )
        .required("relation", AT::String, "the relation, e.g. \"active_in\"")
        .required(
            "other",
            AT::Handle,
            "the memory to link to, e.g. context.current()",
        );

    let unlink = AE::new("mem:unlink")
        .description(
            "Remove a link made with mem:link, e.g. clear active_in on a thread that has closed.",
        )
        .required("relation", AT::String, "the relation")
        .required("other", AT::Handle, "the memory the link points to");

    let context = AE::new("context.current")
        .description(
            "The context/* memory for the current conversation. Check its #confidential tag to \
             know whether the room is confidential.",
        )
        .returns(AT::Handle.optional());

    let abort = AE::new("block.abort")
        .description("Discard everything this block buffered and end it, recording the reason.")
        .optional("reason", AT::String, "why the block was abandoned");

    let upcoming = AE::new("calendar.upcoming")
        .description(
            "Memories with something happening soon, soonest first — read each for detail.",
        )
        .optional(
            "opts",
            object().optional(
                "within",
                AT::String,
                "how far ahead to look, e.g. \"7 days\" or \"2 weeks\"; defaults to 7 days",
            ),
            "options",
        )
        .returns(AT::Handle.list());

    let on = AE::new("calendar.on")
        .description("Memories with something happening on a given day.")
        .required("date", AT::String, "the day as \"YYYY-MM-DD\"")
        .returns(AT::Handle.list());

    let recurring = AE::new("calendar.recurring")
        .description("Memories with a recurring occurrence.")
        .returns(AT::Handle.list());

    vec![
        create, get, search, append, entries, history, supersede, link, unlink, context, abort,
        upcoming, on, recurring,
    ]
}

/// Render [`api_reference`] as the system prompt's API-description block.
pub fn render_api_reference() -> String {
    super::api_doc::render(&api_reference())
}

/// An infrastructure failure executing a block (not an agent-visible terminal outcome, which is a
/// [`BlockOutcome::Terminated`]).
#[derive(Debug)]
pub enum LuaError {
    /// The VM could not be set up.
    Vm(mlua::Error),
    Store(StoreError),
    Graph(GraphError),
}

impl std::fmt::Display for LuaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LuaError::Vm(error) => write!(f, "lua (vm): {error}"),
            LuaError::Store(error) => write!(f, "lua (store): {error}"),
            LuaError::Graph(error) => write!(f, "lua (graph): {error}"),
        }
    }
}

impl std::error::Error for LuaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LuaError::Vm(error) => Some(error),
            LuaError::Store(error) => Some(error),
            LuaError::Graph(error) => Some(error),
        }
    }
}

impl From<StoreError> for LuaError {
    fn from(error: StoreError) -> Self {
        LuaError::Store(error)
    }
}

impl From<GraphError> for LuaError {
    fn from(error: GraphError) -> Self {
        LuaError::Graph(error)
    }
}

/// The block-scoped handles every memory-API closure captures: the transaction (`block`), the
/// infrastructure-error slot (`infra`), the per-block lock set (`lock_set`), and the server-wide lock
/// registry (`manager`). Bundled so the install helpers pass one seam rather than four parallel
/// arguments, and the `'static` async closures clone one value. `Clone` clones the inner `Arc`s.
#[derive(Clone)]
struct BlockApi {
    block: Arc<Mutex<MemoryBlock>>,
    infra: Arc<Mutex<Option<GraphError>>>,
    lock_set: Arc<Mutex<LockSet>>,
    manager: Arc<MemoryLocks>,
}

impl BlockApi {
    /// Acquire `id`'s lock (unless already held), holding the owned guard in the lock set to block end.
    async fn lock(&self, id: MemoryId) {
        ensure_locked(&self.lock_set, &self.manager, [id]).await;
    }

    /// Acquire the locks for `ids` (skipping any already held) — the multi-memory operations (a link's
    /// two endpoints, a calendar query's whole result set).
    async fn lock_all(&self, ids: impl IntoIterator<Item = MemoryId>) {
        ensure_locked(&self.lock_set, &self.manager, ids).await;
    }

    /// Lock the whole `same_as` class of `id` (plus `id` itself) before a traversing read, so a
    /// concurrent write to a sibling stub cannot tear the merged view (spec §Concurrency → class-wide
    /// locking). The class membership is read lock-free through the block; a graph failure routes to
    /// `infra`. The class boundary is read-then-locked, so a concurrent operator merge can shift it —
    /// an accepted edge the timeout backstops (a platform turn cannot merge).
    async fn lock_class(&self, id: MemoryId) -> mlua::Result<()> {
        let members = self
            .block
            .lock()
            .class_members(id)
            .map_err(|error| route_error(error, &mut self.infra.lock()))?;
        ensure_locked(
            &self.lock_set,
            &self.manager,
            std::iter::once(id).chain(members),
        )
        .await;
        Ok(())
    }
}

/// The per-memory locks a block holds, keyed by memory and released together at block end (spec
/// §Concurrency → lifetime is the code block). The owned guards live here, not in the closures, so
/// [`release_locks`] can drop them deterministically at the end of `execute`.
#[derive(Default)]
struct LockSet {
    held: HashMap<MemoryId, OwnedMutexGuard<()>>,
}

impl LockSet {
    fn holds(&self, id: MemoryId) -> bool {
        self.held.contains_key(&id)
    }

    fn insert(&mut self, id: MemoryId, guard: OwnedMutexGuard<()>) {
        self.held.insert(id, guard);
    }

    fn take(&mut self) -> Vec<OwnedMutexGuard<()>> {
        std::mem::take(&mut self.held).into_values().collect()
    }
}

/// Acquire the registry lock for each id not already held by `lock_set`, recording each owned guard.
/// The `lock_set` `parking_lot` guard is taken only to test membership and to insert, never held across
/// the acquire `.await`; the only long-held locks are the per-memory ones, so two blocks acquiring in
/// opposite orders deadlock only until the per-block timeout breaks and retries them (spec §Concurrency
/// → timeout-and-retry, not an ordering protocol). Within one block the calls are sequential (Lua runs
/// one operation at a time), so the membership test is race-free and never double-acquires an id.
async fn ensure_locked(
    lock_set: &Arc<Mutex<LockSet>>,
    manager: &Arc<MemoryLocks>,
    ids: impl IntoIterator<Item = MemoryId>,
) {
    for id in ids {
        if lock_set.lock().holds(id) {
            continue;
        }
        let guard = manager.acquire(id).await;
        lock_set.lock().insert(id, guard);
    }
}

/// Drain and drop the block's lock guards, releasing the per-memory locks so the next block (here or in
/// another conversation) can take them. The `'static` Lua closures still hold `Arc` clones of the
/// now-empty lock set, but no longer any guard — a leaked guard would deadlock the next block touching
/// that memory, so this is called on every exit path of `execute`.
fn release_locks(lock_set: &Arc<Mutex<LockSet>>) {
    let guards = lock_set.lock().take();
    drop(guards);
}

/// The terminal cause for a block that blew its time budget: the budget in seconds, plus — when the
/// block exhausted its retries without an MCP call — the attempt count, so the give-up is auditable.
fn timed_out_cause(budget: std::time::Duration, attempts: Option<u32>) -> TerminalCause {
    let secs = budget.as_secs();
    let message = match attempts {
        Some(attempts) => format!(
            "the block exceeded its time budget of {secs}s on each of {attempts} attempts and was aborted"
        ),
        None => format!("the block exceeded its time budget of {secs}s and was aborted"),
    };
    TerminalCause::Error(message)
}

/// Build a Lua handle table `{ id = "<ulid>" }` with the memory methods as its metatable index.
fn make_handle(lua: &Lua, id: MemoryId, metatable: &Table) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    handle.set("id", id.0.to_string())?;
    handle.set_metatable(Some(metatable.clone()))?;
    Ok(handle)
}

/// Wrap a list of memory ids as a Lua sequence of handles, in order — the `calendar.*` return shape.
fn make_handle_list(lua: &Lua, ids: Vec<MemoryId>, metatable: &Table) -> mlua::Result<Value> {
    let list = lua.create_table()?;
    for (index, id) in ids.into_iter().enumerate() {
        list.set(index + 1, make_handle(lua, id, metatable)?)?;
    }
    Ok(Value::Table(list))
}

fn handle_id(handle: &Table) -> mlua::Result<MemoryId> {
    let id: String = handle.get("id")?;
    Ulid::from_string(&id)
        .map(MemoryId)
        .map_err(|e| mlua::Error::RuntimeError(format!("invalid memory handle id {id:?}: {e}")))
}

/// Build an entry handle `{ id = "<ulid>", text = "..." }` backed by the entry metatable, so it
/// renders as its text (`__tostring` / `__concat`) yet stays addressable for `mem:supersede`.
fn make_entry_handle(
    lua: &Lua,
    entry_id: EntryId,
    text: &str,
    entry_metatable: &Table,
) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    handle.set("id", entry_id.0.to_string())?;
    handle.set("text", text)?;
    handle.set_metatable(Some(entry_metatable.clone()))?;
    Ok(handle)
}

/// Wrap a list of entry refs as a Lua sequence of entry handles, in order — the `mem:entries()` /
/// `mem:history()` return shape.
fn make_entry_handle_list(
    lua: &Lua,
    entries: Vec<EntryRef>,
    entry_metatable: &Table,
) -> mlua::Result<Value> {
    let list = lua.create_table()?;
    for (index, entry) in entries.into_iter().enumerate() {
        list.set(
            index + 1,
            make_entry_handle(lua, entry.entry_id, &entry.text, entry_metatable)?,
        )?;
    }
    Ok(Value::Table(list))
}

fn entry_handle_id(handle: &Table) -> mlua::Result<EntryId> {
    let id: String = handle.get("id")?;
    Ulid::from_string(&id)
        .map(EntryId)
        .map_err(|e| mlua::Error::RuntimeError(format!("invalid entry handle id {id:?}: {e}")))
}

/// Route a memory operation's error. A teachable violation (a duplicate name, an unknown relation)
/// becomes the Lua runtime error the agent sees as the block's terminal cause. A graph read failure
/// is infrastructure, not the agent's doing: it is stashed in the caller's `infra` slot for `execute`
/// to bubble up as a [`LuaError`], and the returned Lua error only serves to stop the script.
fn route_error(error: MemoryError, infra: &mut Option<GraphError>) -> mlua::Error {
    match error {
        MemoryError::Graph(graph_error) => {
            *infra = Some(graph_error);
            mlua::Error::RuntimeError("internal graph error".to_owned())
        }
        teachable => mlua::Error::RuntimeError(teachable.to_string()),
    }
}

/// The default number of `memory.search` results when the caller gives no `limit`.
const DEFAULT_SEARCH_LIMIT: usize = 8;

/// The `opts` table `memory.search` accepts, deserialized from Lua.
#[derive(Default, Deserialize)]
#[serde(default)]
struct SearchOpts {
    namespace: Option<String>,
    tags: Vec<String>,
    limit: Option<usize>,
}

/// One ranked search result handed back to Lua as `{ name, description, score, marker? }`.
struct SearchRow {
    name: String,
    description: String,
    score: f32,
    marker: Option<String>,
}

/// Run a `memory.search`: embed the query off every lock, read the search settings, then rank under a
/// brief graph + vector-index read lock (spec §Time → search scoring, §Visibility). The `Err` is the
/// agent-facing failure message — search is read-only, so a failure (no embedder, a transient embed or
/// backend error) terminates the block without corrupting anything.
async fn run_memory_search(
    engine: &Engine,
    present_set: &[MemoryId],
    query: &str,
    opts: &SearchOpts,
) -> Result<Vec<SearchRow>, String> {
    let Some(retrieval) = &engine.retrieval else {
        return Err(
            "memory.search is unavailable on this instance (no embedding endpoint configured)"
                .to_owned(),
        );
    };
    let embedding = retrieval
        .embedder
        .embed(&[query.to_owned()])
        .await
        .map_err(|error| format!("memory.search: embedding the query failed: {error}"))?
        .into_iter()
        .next()
        .ok_or_else(|| "memory.search: the embedder returned no vector".to_owned())?;
    let settings = Settings::from_store(engine.store.lock().as_ref())
        .map_err(|error| format!("memory.search: {error}"))?
        .search;
    let now = engine.clock.now();
    let tags: Vec<TagName> = opts.tags.iter().map(TagName::new).collect();
    let request = SearchQuery {
        text: query,
        embedding: &embedding,
        namespace: opts.namespace.as_deref(),
        tags: &tags,
        present_set,
    };
    let limit = opts.limit.unwrap_or(DEFAULT_SEARCH_LIMIT);
    let hits = {
        // Graph before the vector index — the lock order `memory.search` and the indexer share. Both
        // are held only across the synchronous ranking, never an `.await`.
        let graph = engine.graph.lock();
        let vectors = retrieval.vectors.lock();
        search(&graph, vectors.as_ref(), &request, &settings, now, limit)
            .map_err(|error| format!("memory.search: {error}"))?
    };
    Ok(hits
        .into_iter()
        .map(|hit| SearchRow {
            name: hit.memory.name.as_str().to_owned(),
            description: hit.memory.description,
            score: hit.score,
            marker: hit.marker,
        })
        .collect())
}

/// Render a script's final value to the text the agent sees back (REPL-style).
fn render(lua: &Lua, value: &Value) -> String {
    match value {
        Value::Nil => "nil".to_owned(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.to_string_lossy(),
        // A table with a `__tostring` metamethod (an entry handle) renders through it, so a returned
        // entry — or a list of them — reads as its text rather than `<table>`. `coerce_string` would
        // not do this (it ignores `__tostring`), so call the `tostring` builtin, which honors it.
        Value::Table(t) => match tostring_via_metamethod(lua, value, t) {
            Some(text) => text,
            None => render_table(lua, t),
        },
        other => format!("<{}>", other.type_name()),
    }
}

/// Render a table through its `__tostring` metamethod, if it has one — the entry-handle case. `None`
/// for a plain table (no metamethod), so the caller falls back to the array rendering.
fn tostring_via_metamethod(lua: &Lua, value: &Value, table: &Table) -> Option<String> {
    let has_tostring = table
        .metatable()
        .is_some_and(|mt| mt.contains_key("__tostring").unwrap_or(false));
    if !has_tostring {
        return None;
    }
    lua.globals()
        .get::<mlua::Function>("tostring")
        .and_then(|tostring| tostring.call::<String>(value.clone()))
        .ok()
}

/// Render a table as its array part joined by newlines (e.g. a list of entry handles), else generic.
fn render_table(lua: &Lua, table: &Table) -> String {
    let items: Vec<String> = table
        .clone()
        .sequence_values::<Value>()
        .filter_map(Result::ok)
        .map(|value| render(lua, &value))
        .collect();
    if items.is_empty() {
        "<table>".to_owned()
    } else {
        items.join("\n")
    }
}

/// Render a value to its text for entry-handle `__concat`: an entry handle yields its `text`; any
/// other value coerces as Lua's `tostring` would (strings and numbers directly, otherwise empty).
fn value_text(lua: &Lua, value: &Value) -> mlua::Result<String> {
    if let Value::Table(table) = value
        && let Ok(text) = table.get::<String>("text")
    {
        return Ok(text);
    }
    Ok(lua
        .coerce_string(value.clone())?
        .map(|s| s.to_string_lossy())
        .unwrap_or_default())
}
