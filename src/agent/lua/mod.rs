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

mod error;
mod reference;
mod runtime;
mod tables;

use std::{collections::BTreeMap, sync::Arc, time::Instant};

use mlua::{Lua, LuaOptions, StdLib, Value};
use parking_lot::Mutex;

use crate::{
    InstanceFeatures,
    engine::Engine,
    event::{EventPayload, TerminalCause},
    graph::GraphError,
    ids::{ConversationId, MemoryId, TurnId},
    memory::memory_block::{BlockEffects, MemoryBlock},
    store::StoreError,
};

use super::BlockContext;
use runtime::{
    BlockApi, LockSet, combine_output, install_inspect, install_table_concat, release_locks,
    render, timed_out_cause,
};

pub use reference::{api_reference, render_api_reference};

/// One conversation's VM. Globals persist across the session's blocks; the memory API is installed
/// fresh per block, while the MCP projection (when configured) is installed once and persists like the
/// agent scratchpad.
pub struct Session {
    lua: Lua,
    conversation: ConversationId,
    /// The session's MCP state — the host, configured servers, and lazily-spawned instances backing the
    /// `mcp.<server>.*` projection — or `None` when no host is configured.
    mcp: Option<std::sync::Arc<super::mcp_api::McpSession>>,
    /// Which API features this session enables — gates the Lua functions installed per block, the API
    /// reference rendered into the system prompt, and (at genesis) the scaffold dotpoints. Read fresh
    /// each block so it always reflects the instance's current features.
    features: InstanceFeatures,
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
    pub fn new(conversation: ConversationId, features: InstanceFeatures) -> Session {
        let lua = sandboxed_lua();
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
        host: std::sync::Arc<dyn crate::mcp::McpHost>,
        catalogue: super::mcp_api::McpCatalogue,
        features: InstanceFeatures,
    ) -> Session {
        let lua = sandboxed_lua();
        let mcp = std::sync::Arc::new(super::mcp_api::McpSession::new(host, catalogue));
        super::mcp_api::install(&lua, &mcp).expect("installing the mcp projection global");
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
                printed: Arc::new(Mutex::new(String::new())),
            };

            // The handle metatable and its methods table back every memory handle the API mints; the
            // entry metatable backs the addressable content-entry handles that `mem:append` /
            // `mem:entries` / `mem:history` return (text-rendering, so reading stays ergonomic).
            let methods = self.lua.create_table().map_err(LuaError::Vm)?;
            let metatable = self.lua.create_table().map_err(LuaError::Vm)?;
            // `__index` is wired in `install_block_api`: it resolves `handle.name` / `handle.description`
            // lazily from the id and otherwise dispatches to `methods`.
            let entry_metatable = tables::entry_metatable(&self.lua).map_err(LuaError::Vm)?;

            // Reset the per-attempt "made an MCP call" latch, so the no-retry decision below reflects
            // this attempt only.
            self.begin_mcp_block();

            // Installing the API is our-side setup: a failure here is a bug, not an agent-visible
            // outcome.
            tables::install_block_api(
                &self.lua,
                &api,
                &methods,
                &metatable,
                &entry_metatable,
                &self.features,
            )
            .map_err(LuaError::Vm)?;

            // The agent-visible outcome: the rendered final value, or the runtime error/abort that
            // ended the script, bounded by the block's time budget. The block's memory functions only
            // hold their parking_lot guards transiently, never across this suspension point. `started`
            // times this attempt's eval for the console's turn timeline (the final attempt's, since a
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
            // The agent-visible result is the rendered final value, prefixed by anything the block
            // printed (so an agent that prints a query result instead of returning it still sees it).
            let evaluated = evaluated.map(|value| {
                let rendered = render(&self.lua, &value);
                let printed = std::mem::take(&mut *api.printed.lock());
                combine_output(printed, rendered)
            });

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
                    // A dry run discards the whole buffer — including the `LuaExecuted` record — and
                    // commits nothing; the operator gets the rendered result over a clean log.
                    if context.dry_run {
                        Ok(BlockOutcome::Committed { result })
                    } else {
                        let mut events = events;
                        // Tell the agent what the block actually changed, so it sees its writes landed
                        // and does not re-issue them next turn for want of confirmation. Folded into the
                        // result the agent reads and the `LuaExecuted` record, so the log shows what was
                        // shown.
                        let result =
                            with_commit_summary(result, summarize_committed(engine, &events));
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
                }
                Err(error) => {
                    // Discard the buffer; record only what the agent saw — the terminal cause.
                    let cause = match aborted {
                        Some(reason) => TerminalCause::Aborted(reason),
                        None => TerminalCause::Error(error.to_string()),
                    };
                    if context.dry_run {
                        Ok(BlockOutcome::Terminated(cause))
                    } else {
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
        // A dry run commits nothing — not even the terminal record.
        if context.dry_run {
            return Ok(BlockOutcome::Terminated(cause));
        }
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

/// Construct the block VM with a deliberately narrow surface: a memory block is an orchestration
/// script over the projected API (`memory`, `block`, `context`, `mcp`, …), never a host program, so it
/// must not reach the filesystem, the environment, the process, or arbitrary code on disk. MCP is the
/// only sanctioned outward reach (spec §External I/O via MCP).
///
/// Only the pure libraries are loaded — string, table, math, utf8, and coroutine — so `os`, `io`,
/// `package` (and thus `require`), `debug`, and the FFI/JIT escapes are never present. The base library
/// is always loaded, so the code-loading globals it still carries (`load`, `loadfile`, `dofile`,
/// `loadstring`, `require`) are then removed by hand. Dropping `os` also keeps blocks deterministic
/// under replay: there is no wall-clock `os.time`/`os.date`, so time only ever comes from the injected
/// clock. `print` and `inspect` are installed per block; here we only fix the global environment.
fn sandboxed_lua() -> Lua {
    let lua = Lua::new_with(
        StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE,
        LuaOptions::default(),
    )
    .expect("constructing the sandboxed Lua VM");
    let globals = lua.globals();
    for unsafe_global in ["load", "loadfile", "dofile", "loadstring", "require"] {
        globals
            .set(unsafe_global, Value::Nil)
            .expect("removing an unsafe base-library global");
    }
    drop(globals);
    install_inspect(&lua).expect("installing the inspect global");
    install_table_concat(&lua).expect("installing the lenient table.concat");
    lua
}

/// Fold a block's committed-effects summary into the result the agent reads. A write-only block returns
/// `nil`, which on its own tells the agent nothing about whether its create or append landed — so the
/// summary stands in for a bare `nil`/empty result, and trails a genuine returned value otherwise.
fn with_commit_summary(rendered: String, summary: Option<String>) -> String {
    let Some(summary) = summary else {
        return rendered;
    };
    let trimmed = rendered.trim();
    if trimmed.is_empty() || trimmed == "nil" {
        summary
    } else {
        format!("{rendered}\n\n{summary}")
    }
}

/// A concise, agent-facing summary of what a block committed — "Committed: created topic/q3_plan;
/// appended 2 entries to topic/q3_plan." — built from the block's effect events. `None` when the block
/// changed nothing the agent should be told about (a read-only query), so a pure read keeps its own
/// rendered result unchanged. Names of memories created in this very block are read from the events
/// (they are not in the graph yet); existing targets of an append or link resolve through the graph.
fn summarize_committed(engine: &Engine, events: &[EventPayload]) -> Option<String> {
    let fresh: BTreeMap<MemoryId, String> = events
        .iter()
        .filter_map(|event| match event {
            EventPayload::MemoryCreated { id, name } => Some((*id, name.as_str().to_owned())),
            _ => None,
        })
        .collect();
    let name_of = |id: MemoryId| -> String {
        fresh
            .get(&id)
            .cloned()
            .or_else(|| {
                engine
                    .graph
                    .lock()
                    .memory_by_id(id)
                    .ok()
                    .flatten()
                    .map(|memory| memory.name.as_str().to_owned())
            })
            .unwrap_or_else(|| "a memory".to_owned())
    };

    let mut created: Vec<String> = Vec::new();
    let mut appended: BTreeMap<MemoryId, usize> = BTreeMap::new();
    let mut superseded: Vec<MemoryId> = Vec::new();
    let mut other: Vec<String> = Vec::new();
    for event in events {
        match event {
            EventPayload::MemoryCreated { name, .. } => created.push(name.as_str().to_owned()),
            EventPayload::MemoryContentAppended { id, .. } => {
                *appended.entry(*id).or_default() += 1
            }
            EventPayload::MemorySuperseded { id, .. } => superseded.push(*id),
            EventPayload::LinkCreated {
                from, to, relation, ..
            } => other.push(format!(
                "linked {} {} {}",
                name_of(*from),
                relation.as_str(),
                name_of(*to)
            )),
            EventPayload::LinkRemoved {
                from, to, relation, ..
            } => other.push(format!(
                "removed the {} link between {} and {}",
                relation.as_str(),
                name_of(*from),
                name_of(*to)
            )),
            EventPayload::TagAppliedToMemory { memory, tag } => {
                other.push(format!("tagged {} #{}", name_of(*memory), tag.as_str()))
            }
            EventPayload::TagRemovedFromMemory { memory, tag } => other.push(format!(
                "removed #{} from {}",
                tag.as_str(),
                name_of(*memory)
            )),
            EventPayload::TagCreated { name, .. } => {
                other.push(format!("created the tag #{}", name.as_str()))
            }
            EventPayload::MemoryDeleted { id } => other.push(format!("deleted {}", name_of(*id))),
            EventPayload::MemoryRenamed {
                old_name, new_name, ..
            } => other.push(format!(
                "renamed {} to {}",
                old_name.as_str(),
                new_name.as_str()
            )),
            EventPayload::LinkTypeRegistered { name, .. } => {
                other.push(format!("registered the {} relation", name.as_str()))
            }
            _ => {}
        }
    }

    let mut summary: Vec<String> = Vec::new();
    if !created.is_empty() {
        summary.push(format!("created {}", created.join(", ")));
    }
    for (id, count) in &appended {
        let entries = if *count == 1 { "entry" } else { "entries" };
        summary.push(format!("appended {count} {entries} to {}", name_of(*id)));
    }
    for id in &superseded {
        summary.push(format!("superseded an entry on {}", name_of(*id)));
    }
    summary.extend(other);

    (!summary.is_empty()).then(|| format!("Committed: {}.", summary.join("; ")))
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
