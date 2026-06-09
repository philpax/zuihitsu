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

use std::sync::Arc;

use mlua::{Lua, LuaSerdeExt, Table, Value};
use parking_lot::Mutex;
use ulid::Ulid;

use crate::{
    engine::Engine,
    event::{EventPayload, TerminalCause},
    graph::GraphError,
    ids::{ConversationId, MemoryId, TurnId},
    memory::memory_block::{AppendOptions, BlockEffects, MemoryBlock, MemoryError},
    store::StoreError,
    vocabulary::RelationName,
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
    #[cfg(feature = "mcp")]
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
            #[cfg(feature = "mcp")]
            mcp: None,
        }
    }

    /// A VM with the `mcp.<server>.*` projection installed from `catalogue` (the probed, filtered tool
    /// set), with live instances spawned on demand through `host`. The projection global is installed
    /// once here and persists across the session's blocks (the server instances are session-scoped).
    /// Lua-table creation cannot realistically fail at construction, so installation is treated as
    /// infallible, like [`Session::new`].
    #[cfg(feature = "mcp")]
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
    #[cfg(feature = "mcp")]
    pub fn mcp_api_entries(&self) -> Vec<super::api_doc::ApiEntry> {
        self.mcp
            .as_ref()
            .map(|mcp| mcp.api_entries())
            .unwrap_or_default()
    }

    /// Tear down the session's MCP instances (close stdin, wait, kill on a grace timeout), best-effort.
    /// A no-op when no MCP host is configured. Called when the session ends.
    #[cfg(feature = "mcp")]
    pub async fn shutdown_mcp(&self) {
        if let Some(mcp) = &self.mcp {
            mcp.shutdown().await;
        }
    }

    /// Drop the MCP instance whose call a block timeout just cut off, if any (the abandoned call left
    /// its server-side state undefined). A no-op when no host is configured or nothing was in flight.
    #[cfg(feature = "mcp")]
    fn drop_in_flight_mcp(&self) {
        if let Some(mcp) = &self.mcp {
            mcp.drop_in_flight();
        }
    }

    pub fn conversation(&self) -> ConversationId {
        self.conversation
    }

    /// Execute one block as a transaction. On a clean run, the buffered side effects plus a
    /// `LuaExecuted` commit together; on error or abort, only a `LuaExecuted` recording the terminal
    /// cause is written. The graph is brought up to log-head afterward either way.
    pub async fn execute(
        &self,
        engine: &Arc<Engine>,
        context: &BlockContext,
        script: &str,
    ) -> Result<BlockOutcome, LuaError> {
        // The transaction owns the buffer, the touched set, and the write invariants. It holds a
        // shared handle to the `engine` (locking the graph transiently per read) and lives behind an
        // `Arc<Mutex<…>>` so the `'static` Lua functions installed below can drive it across the
        // script's `eval_async`; the commit happens after the script finishes and the effects are
        // drained back out through the lock.
        let block = Arc::new(Mutex::new(MemoryBlock::new(
            engine.clone(),
            context.teller.clone(),
            context.authority,
            self.conversation,
        )?));

        // The handle metatable and its methods table back every memory handle the API mints.
        let methods = self.lua.create_table().map_err(LuaError::Vm)?;
        let metatable = self.lua.create_table().map_err(LuaError::Vm)?;
        metatable
            .set("__index", methods.clone())
            .map_err(LuaError::Vm)?;

        // A graph read failing inside an operation is infrastructure, not the agent's doing — it is
        // stashed here and bubbled up as a `LuaError` after the script, rather than shown to the agent
        // as a teachable block error.
        let infra: Arc<Mutex<Option<GraphError>>> = Arc::new(Mutex::new(None));

        // Installing the API is our-side setup: a failure here is a bug, not an agent-visible outcome.
        self.install_block_api(&block, &infra, &methods, &metatable)
            .map_err(LuaError::Vm)?;

        // The agent-visible outcome: the rendered final value, or the runtime error/abort that ended
        // the script, bounded by the block's time budget (spec §Concurrency → lock acquisition). A
        // block stuck on slow external I/O is cut here, emitting nothing. The block's memory functions
        // are synchronous, so they never hold the block lock across this suspension point.
        let evaluated = match tokio::time::timeout(
            context.block_timeout,
            self.lua.load(script).eval_async::<Value>(),
        )
        .await
        {
            Ok(evaluated) => evaluated.map(|value| render(&value)),
            Err(_elapsed) => return self.abort_timed_out(engine, context, script, &block),
        };

        // An infrastructure failure during the block (a graph read) takes precedence over the
        // script's apparent outcome: it bubbles up, discarding the buffer, rather than reaching the
        // agent.
        if let Some(graph_error) = infra.lock().take() {
            return Err(LuaError::Graph(graph_error));
        }

        // Drain the effects through the lock: the Lua functions still hold `Arc` clones of the block,
        // so it cannot be reclaimed by ownership, but those references are inert now the script has
        // finished and are overwritten when the next block re-installs the API.
        let BlockEffects {
            events,
            touched,
            aborted,
        } = block.lock().take_effects();

        match evaluated {
            Ok(result) => {
                // Commit the buffered side effects plus the LuaExecuted record, atomically.
                let mut events = events;
                events.push(self.lua_executed(
                    context.turn_id,
                    script,
                    Some(result.clone()),
                    touched,
                    None,
                ));
                self.finish(engine, events, BlockOutcome::Committed { result })
            }
            Err(error) => {
                // Discard the buffer; record only what the agent saw — the terminal cause.
                let cause = match aborted {
                    Some(reason) => TerminalCause::Aborted(reason),
                    None => TerminalCause::Error(error.to_string()),
                };
                let event =
                    self.lua_executed(context.turn_id, script, None, touched, Some(cause.clone()));
                self.finish(engine, vec![event], BlockOutcome::Terminated(cause))
            }
        }
    }

    /// End a block that exceeded its time budget (spec §Concurrency → lock acquisition). The buffer is
    /// discarded — the block emits nothing — and only a `LuaExecuted` recording the terminal cause is
    /// committed, so the timeout is auditable and the agent sees what happened. The in-flight MCP
    /// instance, if any, is dropped: its session-side state is now undefined after the abandoned call.
    fn abort_timed_out(
        &self,
        engine: &Engine,
        context: &BlockContext,
        script: &str,
        block: &Arc<Mutex<MemoryBlock>>,
    ) -> Result<BlockOutcome, LuaError> {
        #[cfg(feature = "mcp")]
        self.drop_in_flight_mcp();
        // Drain the touched set for the audit record; the buffered events are discarded.
        let BlockEffects { touched, .. } = block.lock().take_effects();
        let cause = TerminalCause::Error(format!(
            "the block exceeded its time budget of {}s and was aborted",
            context.block_timeout.as_secs()
        ));
        let event = self.lua_executed(context.turn_id, script, None, touched, Some(cause.clone()));
        self.finish(engine, vec![event], BlockOutcome::Terminated(cause))
    }

    /// Install the per-block memory API as `'static` Lua functions over the shared `block`. Each
    /// function locks the block transiently for its operation; a graph-read failure is routed to
    /// `infra` (infrastructure, bubbled up) while a teachable violation becomes the Lua runtime error
    /// the agent sees. The handle `metatable`/`methods` tables back every minted memory handle. The
    /// registration is split table by table so each group stays legible.
    fn install_block_api(
        &self,
        block: &Arc<Mutex<MemoryBlock>>,
        infra: &Arc<Mutex<Option<GraphError>>>,
        methods: &Table,
        metatable: &Table,
    ) -> mlua::Result<()> {
        self.install_handle_methods(block, infra, methods)?;
        let globals = self.lua.globals();
        globals.set("memory", self.memory_table(block, infra, metatable)?)?;
        globals.set("block", self.block_table(block)?)?;
        globals.set("context", self.context_table(block, metatable)?)?;
        globals.set("calendar", self.calendar_table(block, infra, metatable)?)?;
        Ok(())
    }

    /// The `mem:*` handle methods (`append`, `entries`, `link`, `unlink`) on the metatable's `methods`
    /// table. Each acts on the handle passed as `this` and mints nothing, so it needs no metatable.
    fn install_handle_methods(
        &self,
        block: &Arc<Mutex<MemoryBlock>>,
        infra: &Arc<Mutex<Option<GraphError>>>,
        methods: &Table,
    ) -> mlua::Result<()> {
        // mem:append(text[, opts]) — `opts` is the typed override struct, deserialized from the table.
        methods.set(
            "append",
            self.lua.create_function({
                let block = block.clone();
                let infra = infra.clone();
                move |lua, (this, text, opts): (Table, String, Value)| {
                    let id = handle_id(&this)?;
                    let opts: AppendOptions = if opts.is_nil() {
                        AppendOptions::default()
                    } else {
                        lua.from_value(opts)?
                    };
                    block
                        .lock()
                        .append(id, &text, opts)
                        .map_err(|error| route_error(error, &mut infra.lock()))
                }
            })?,
        )?;

        // mem:entries() — the memory's entry texts across its merged identity plus pending writes.
        methods.set(
            "entries",
            self.lua.create_function({
                let block = block.clone();
                let infra = infra.clone();
                move |lua, this: Table| {
                    let id = handle_id(&this)?;
                    let texts = block
                        .lock()
                        .entries(id)
                        .map_err(|error| route_error(error, &mut infra.lock()))?;
                    lua.create_sequence_from(texts)
                }
            })?,
        )?;

        // mem:link(relation, other) / mem:unlink(relation, other) — flag (or clear) a relation such
        // as `active_in`. The script names the relation as a string; it is recognized into its typed
        // [`RelationName`] here, at the wrapper boundary.
        methods.set(
            "link",
            self.lua.create_function({
                let block = block.clone();
                let infra = infra.clone();
                move |_, (this, relation, other): (Table, String, Table)| {
                    block
                        .lock()
                        .link(
                            handle_id(&this)?,
                            handle_id(&other)?,
                            RelationName::new(relation),
                        )
                        .map_err(|error| route_error(error, &mut infra.lock()))
                }
            })?,
        )?;
        methods.set(
            "unlink",
            self.lua.create_function({
                let block = block.clone();
                let infra = infra.clone();
                move |_, (this, relation, other): (Table, String, Table)| {
                    block
                        .lock()
                        .unlink(
                            handle_id(&this)?,
                            handle_id(&other)?,
                            RelationName::new(relation),
                        )
                        .map_err(|error| route_error(error, &mut infra.lock()))
                }
            })?,
        )?;
        Ok(())
    }

    /// The `memory` global: `create` and `get`, both of which mint handles (hence the metatable).
    fn memory_table(
        &self,
        block: &Arc<Mutex<MemoryBlock>>,
        infra: &Arc<Mutex<Option<GraphError>>>,
        metatable: &Table,
    ) -> mlua::Result<Table> {
        let memory = self.lua.create_table()?;
        // memory.create(name[, content]) — create a memory and optionally its first entry.
        memory.set(
            "create",
            self.lua.create_function({
                let block = block.clone();
                let infra = infra.clone();
                let metatable = metatable.clone();
                move |lua, (name, content): (String, Option<String>)| {
                    let id = block
                        .lock()
                        .create(&name, content.as_deref())
                        .map_err(|error| route_error(error, &mut infra.lock()))?;
                    make_handle(lua, id, &metatable)
                }
            })?,
        )?;
        // memory.get(name) — resolve through the block's pending creates, then the graph.
        memory.set(
            "get",
            self.lua.create_function({
                let block = block.clone();
                let infra = infra.clone();
                let metatable = metatable.clone();
                move |lua, name: String| match block
                    .lock()
                    .get(&name)
                    .map_err(|error| route_error(error, &mut infra.lock()))?
                {
                    Some(id) => Ok(Value::Table(make_handle(lua, id, &metatable)?)),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;
        Ok(memory)
    }

    /// The `block` global: `abort(reason)`, which discards the buffer and ends the block.
    fn block_table(&self, block: &Arc<Mutex<MemoryBlock>>) -> mlua::Result<Table> {
        let block_tbl = self.lua.create_table()?;
        block_tbl.set(
            "abort",
            self.lua.create_function({
                let block = block.clone();
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
    fn context_table(
        &self,
        block: &Arc<Mutex<MemoryBlock>>,
        metatable: &Table,
    ) -> mlua::Result<Table> {
        let context = self.lua.create_table()?;
        context.set(
            "current",
            self.lua.create_function({
                let block = block.clone();
                let metatable = metatable.clone();
                move |lua, ()| match block.lock().current_context() {
                    Some(id) => Ok(Value::Table(make_handle(lua, id, &metatable)?)),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;
        Ok(context)
    }

    /// The `calendar` global: `upcoming`, `on`, and `recurring`, each returning a list of memory
    /// handles, soonest first. Unlike the brief's `<upcoming/>` block these are the agent's own
    /// queries and are not visibility-filtered (like `mem:entries`, the agent sees its whole memory).
    fn calendar_table(
        &self,
        block: &Arc<Mutex<MemoryBlock>>,
        infra: &Arc<Mutex<Option<GraphError>>>,
        metatable: &Table,
    ) -> mlua::Result<Table> {
        let calendar = self.lua.create_table()?;
        calendar.set(
            "upcoming",
            self.lua.create_function({
                let block = block.clone();
                let infra = infra.clone();
                let metatable = metatable.clone();
                move |lua, opts: Option<Table>| {
                    let within: Option<String> = match opts {
                        Some(table) => table.get("within")?,
                        None => None,
                    };
                    let ids = block
                        .lock()
                        .upcoming(within.as_deref())
                        .map_err(|error| route_error(error, &mut infra.lock()))?;
                    make_handle_list(lua, ids, &metatable)
                }
            })?,
        )?;
        calendar.set(
            "on",
            self.lua.create_function({
                let block = block.clone();
                let infra = infra.clone();
                let metatable = metatable.clone();
                move |lua, date: String| {
                    let ids = block
                        .lock()
                        .on(&date)
                        .map_err(|error| route_error(error, &mut infra.lock()))?;
                    make_handle_list(lua, ids, &metatable)
                }
            })?,
        )?;
        calendar.set(
            "recurring",
            self.lua.create_function({
                let block = block.clone();
                let infra = infra.clone();
                let metatable = metatable.clone();
                move |lua, ()| {
                    let ids = block
                        .lock()
                        .recurring()
                        .map_err(|error| route_error(error, &mut infra.lock()))?;
                    make_handle_list(lua, ids, &metatable)
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
    ) -> EventPayload {
        EventPayload::LuaExecuted {
            conversation: self.conversation,
            turn_id,
            script: script.to_owned(),
            result,
            touched,
            terminal_cause,
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
            "the namespaced handle, e.g. \"person/<name>\" or \"topic/<subject>\"",
        )
        .optional("content", AT::String, "an optional first content entry")
        .returns(AT::Handle);

    let get = AE::new("memory.get")
        .description(
            "Fetch a memory by name. Read a merged identity through its canonical person/ handle, \
             not a per-platform stub.",
        )
        .required("name", AT::String, "the memory's handle")
        .returns(AT::Handle.optional());

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
        );

    let entries = AE::new("mem:entries")
        .description("The memory's content entries as text, across its whole merged identity.")
        .returns(AT::String.list());

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
        create, get, append, entries, link, unlink, context, abort, upcoming, on, recurring,
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

/// Render a script's final value to the text the agent sees back (REPL-style).
fn render(value: &Value) -> String {
    match value {
        Value::Nil => "nil".to_owned(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.to_string_lossy(),
        Value::Table(t) => render_table(t),
        other => format!("<{}>", other.type_name()),
    }
}

/// Render a table as its array part joined by newlines (e.g. a list of entry texts), else generic.
fn render_table(table: &Table) -> String {
    let items: Vec<String> = table
        .clone()
        .sequence_values::<Value>()
        .filter_map(Result::ok)
        .map(|value| render(&value))
        .collect();
    if items.is_empty() {
        "<table>".to_owned()
    } else {
        items.join("\n")
    }
}
