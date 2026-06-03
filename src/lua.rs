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

use std::cell::RefCell;

use mlua::{Lua, LuaSerdeExt, Table, Value};
use ulid::Ulid;

use crate::{
    agent::{BlockContext, Engine},
    api_doc::{ApiEntry, ApiType, enum_of, object},
    event::{EventPayload, TerminalCause},
    graph::GraphError,
    ids::{ConversationId, MemoryId, RelationName, TurnId},
    memory_block::{AppendOptions, BlockEffects, MemoryBlock, MemoryError},
    store::StoreError,
};

/// One conversation's VM. Globals persist across the session's blocks; the memory API is installed
/// fresh per block.
pub struct Session {
    lua: Lua,
    conversation: ConversationId,
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
        }
    }

    pub fn conversation(&self) -> ConversationId {
        self.conversation
    }

    /// Execute one block as a transaction. On a clean run, the buffered side effects plus a
    /// `LuaExecuted` commit together; on error or abort, only a `LuaExecuted` recording the terminal
    /// cause is written. The graph is brought up to log-head afterward either way.
    pub fn execute(
        &self,
        engine: &mut Engine,
        context: &BlockContext,
        script: &str,
    ) -> Result<BlockOutcome, LuaError> {
        // The transaction owns the buffer, the touched set, and the write invariants. It borrows the
        // graph immutably for reads; the mutable commit happens after the scope ends and this borrow
        // is released.
        let block = RefCell::new(MemoryBlock::new(
            &*engine.graph,
            engine.clock,
            context.teller.clone(),
            context.authority,
            self.conversation,
        )?);

        // The handle metatable and its methods table are referenced by the scoped functions, so
        // they must outlive the scope — build them here, in the enclosing environment.
        let methods = self.lua.create_table().map_err(LuaError::Vm)?;
        let metatable = self.lua.create_table().map_err(LuaError::Vm)?;
        metatable
            .set("__index", methods.clone())
            .map_err(LuaError::Vm)?;

        // A graph read failing inside an operation is infrastructure, not the agent's doing — it is
        // stashed here and bubbled up as a `LuaError` after the scope, rather than shown to the agent
        // as a teachable block error.
        let infra: RefCell<Option<GraphError>> = RefCell::new(None);

        let block_ref = &block;
        let infra_ref = &infra;
        let metatable = &metatable;
        let methods = &methods;
        let evaluated = self.lua.scope(|scope| {
            let memory = self.lua.create_table()?;

            // mem:append(text[, opts]) — append a content entry to the handle's memory. `opts` is the
            // typed override struct, deserialized straight from the Lua table.
            methods.set(
                "append",
                scope.create_function(|lua, (this, text, opts): (Table, String, Value)| {
                    let id = handle_id(&this)?;
                    let opts: AppendOptions = if opts.is_nil() {
                        AppendOptions::default()
                    } else {
                        lua.from_value(opts)?
                    };
                    block_ref
                        .borrow_mut()
                        .append(id, &text, opts)
                        .map_err(|error| route_error(error, infra_ref))
                })?,
            )?;

            // mem:entries() — the memory's entry texts across its merged identity plus pending writes.
            methods.set(
                "entries",
                scope.create_function(|lua, this: Table| {
                    let id = handle_id(&this)?;
                    let texts = block_ref
                        .borrow_mut()
                        .entries(id)
                        .map_err(|error| route_error(error, infra_ref))?;
                    lua.create_sequence_from(texts)
                })?,
            )?;

            // mem:link(relation, other) / mem:unlink(relation, other) — flag (or clear) a relation
            // such as `active_in` between two memories. The script names the relation as a string;
            // it is recognized into its typed [`RelationName`] here, at the wrapper boundary.
            methods.set(
                "link",
                scope.create_function(|_, (this, relation, other): (Table, String, Table)| {
                    block_ref
                        .borrow_mut()
                        .link(
                            handle_id(&this)?,
                            handle_id(&other)?,
                            RelationName::new(relation),
                        )
                        .map_err(|error| route_error(error, infra_ref))
                })?,
            )?;
            methods.set(
                "unlink",
                scope.create_function(|_, (this, relation, other): (Table, String, Table)| {
                    block_ref
                        .borrow_mut()
                        .unlink(
                            handle_id(&this)?,
                            handle_id(&other)?,
                            RelationName::new(relation),
                        )
                        .map_err(|error| route_error(error, infra_ref))
                })?,
            )?;

            // memory.create(name[, content]) — create a memory and optionally its first entry.
            memory.set(
                "create",
                scope.create_function(|lua, (name, content): (String, Option<String>)| {
                    let id = block_ref
                        .borrow_mut()
                        .create(&name, content.as_deref())
                        .map_err(|error| route_error(error, infra_ref))?;
                    make_handle(lua, id, metatable)
                })?,
            )?;

            // memory.get(name) — resolve through the block's pending creates, then the graph.
            memory.set(
                "get",
                scope.create_function(|lua, name: String| {
                    match block_ref
                        .borrow_mut()
                        .get(&name)
                        .map_err(|error| route_error(error, infra_ref))?
                    {
                        Some(id) => Ok(Value::Table(make_handle(lua, id, metatable)?)),
                        None => Ok(Value::Nil),
                    }
                })?,
            )?;

            // block.abort(reason) — discard the buffer and end the block, recorded as an abort.
            let block_tbl = self.lua.create_table()?;
            block_tbl.set(
                "abort",
                scope.create_function(|_, reason: Option<String>| {
                    block_ref.borrow_mut().abort(reason);
                    Err::<(), _>(mlua::Error::RuntimeError("block aborted".to_owned()))
                })?,
            )?;

            // context.current() — the current conversation's context/* memory (its #confidential tag
            // tells the agent whether the room is confidential), or nil if there is none.
            let context = self.lua.create_table()?;
            context.set(
                "current",
                scope.create_function(|lua, ()| {
                    match block_ref.borrow_mut().current_context() {
                        Some(id) => Ok(Value::Table(make_handle(lua, id, metatable)?)),
                        None => Ok(Value::Nil),
                    }
                })?,
            )?;

            self.lua.globals().set("memory", memory)?;
            self.lua.globals().set("block", block_tbl)?;
            self.lua.globals().set("context", context)?;

            // Inner result: the agent-visible outcome (a value, or a runtime error).
            Ok(self
                .lua
                .load(script)
                .eval::<Value>()
                .map(|value| render(&value)))
        });

        // An infrastructure failure during the block (a graph read) takes precedence over the
        // script's apparent outcome: it bubbles up, discarding the buffer, rather than reaching the
        // agent.
        if let Some(graph_error) = infra.into_inner() {
            return Err(LuaError::Graph(graph_error));
        }

        let BlockEffects {
            events,
            touched,
            aborted,
        } = block.into_inner().into_effects();

        match evaluated {
            // Setup failed — a bug on our side, not an agent-visible outcome.
            Err(error) => Err(LuaError::Vm(error)),
            Ok(Ok(result)) => {
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
            Ok(Err(error)) => {
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
        engine: &mut Engine,
        events: Vec<EventPayload>,
        outcome: BlockOutcome,
    ) -> Result<BlockOutcome, LuaError> {
        engine.store.append(engine.clock.now(), events)?;
        engine.graph.materialize_from(&*engine.store)?;
        Ok(outcome)
    }
}

/// The agent-facing Lua API, as a typed catalogue. Defined here, beside the functions installed in
/// [`Session::execute`], so the prompt and the implementation cannot drift: changing a function
/// means changing its entry right next to it. Rendered into the system prompt's API description
/// through [`crate::api_doc::render`] — the same renderer MCP tools project through (spec §System
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
             aside about someone else defaults private to that speaker.",
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
                    "force the visibility instead of the write-time default",
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

    vec![create, get, append, entries, link, unlink, context, abort]
}

/// Render [`api_reference`] as the system prompt's API-description block.
pub fn render_api_reference() -> String {
    crate::api_doc::render(&api_reference())
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

fn handle_id(handle: &Table) -> mlua::Result<MemoryId> {
    let id: String = handle.get("id")?;
    Ulid::from_string(&id)
        .map(MemoryId)
        .map_err(|e| mlua::Error::RuntimeError(format!("invalid memory handle id {id:?}: {e}")))
}

/// Route a memory operation's error. A teachable violation (a duplicate name, an unknown relation)
/// becomes the Lua runtime error the agent sees as the block's terminal cause. A graph read failure
/// is infrastructure, not the agent's doing: it is stashed in `infra` for `execute` to bubble up as a
/// [`LuaError`], and the returned Lua error only serves to stop the script.
fn route_error(error: MemoryError, infra: &RefCell<Option<GraphError>>) -> mlua::Error {
    match error {
        MemoryError::Graph(graph_error) => {
            *infra.borrow_mut() = Some(graph_error);
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
