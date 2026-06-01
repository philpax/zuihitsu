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
//! buffer and the graph for the block's duration. Agent scratchpad globals persist on the VM across
//! blocks within the session; the API is re-installed each block.

use std::{cell::RefCell, collections::BTreeSet};

use mlua::{Lua, Table, Value};
use ulid::Ulid;

use crate::{
    clock::Clock,
    event::{EventPayload, TerminalCause},
    graph::{Graph, GraphError},
    ids::{ConversationId, EntryId, MemoryId, MemoryName, TurnId},
    store::{Store, StoreError},
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
        store: &mut dyn Store,
        graph: &mut Graph,
        clock: &dyn Clock,
        turn_id: TurnId,
        script: &str,
    ) -> Result<BlockOutcome, LuaError> {
        let block = RefCell::new(BlockState::default());

        // The handle metatable and its methods table are referenced by the scoped functions, so
        // they must outlive the scope — build them here, in the enclosing environment.
        let methods = self.lua.create_table().map_err(LuaError::Vm)?;
        let metatable = self.lua.create_table().map_err(LuaError::Vm)?;
        metatable
            .set("__index", methods.clone())
            .map_err(LuaError::Vm)?;

        // Borrow the graph immutably for reads during execution; the mutable commit happens after
        // the scope ends and this borrow is released.
        let graph_ref: &Graph = graph;
        let block_ref = &block;
        let metatable = &metatable;
        let methods = &methods;
        let evaluated = self.lua.scope(|scope| {
            let memory = self.lua.create_table()?;

            // mem:append(text[, opts]) — buffer a content entry on the handle's memory.
            methods.set(
                "append",
                scope.create_function(
                    |_, (this, text, _opts): (Table, String, Option<Table>)| {
                        let id = handle_id(&this)?;
                        let mut state = block_ref.borrow_mut();
                        state.touched.insert(id);
                        state.buffer.push(EventPayload::MemoryContentAppended {
                            id,
                            entry_id: EntryId::generate(),
                            asserted_at: clock.now(),
                            text,
                        });
                        Ok(())
                    },
                )?,
            )?;

            // mem:entries() — the memory's entry texts, graph plus this block's pending appends.
            methods.set(
                "entries",
                scope.create_function(|lua, this: Table| {
                    let id = handle_id(&this)?;
                    block_ref.borrow_mut().touched.insert(id);
                    let mut texts: Vec<String> = graph_ref
                        .entries(id)
                        .map_err(to_lua_err)?
                        .into_iter()
                        .map(|entry| entry.text)
                        .collect();
                    for event in &block_ref.borrow().buffer {
                        if let EventPayload::MemoryContentAppended {
                            id: entry_id, text, ..
                        } = event
                            && *entry_id == id
                        {
                            texts.push(text.clone());
                        }
                    }
                    lua.create_sequence_from(texts)
                })?,
            )?;

            // memory.create(name[, content]) — create a memory and optionally its first entry.
            memory.set(
                "create",
                scope.create_function(|lua, (name, content): (String, Option<String>)| {
                    let id = MemoryId::generate();
                    let mut state = block_ref.borrow_mut();
                    state.touched.insert(id);
                    state.buffer.push(EventPayload::MemoryCreated {
                        id,
                        name: MemoryName::new(name),
                    });
                    if let Some(text) = content {
                        state.buffer.push(EventPayload::MemoryContentAppended {
                            id,
                            entry_id: EntryId::generate(),
                            asserted_at: clock.now(),
                            text,
                        });
                    }
                    drop(state);
                    make_handle(lua, id, metatable)
                })?,
            )?;

            // memory.get(name) — resolve through the block's pending creates, then the graph.
            memory.set(
                "get",
                scope.create_function(|lua, name: String| {
                    let resolved =
                        resolve_name(&block_ref.borrow(), graph_ref, &name).map_err(to_lua_err)?;
                    match resolved {
                        Some(id) => {
                            block_ref.borrow_mut().touched.insert(id);
                            Ok(Value::Table(make_handle(lua, id, metatable)?))
                        }
                        None => Ok(Value::Nil),
                    }
                })?,
            )?;

            // block.abort(reason) — discard the buffer and end the block, recorded as an abort.
            let block_tbl = self.lua.create_table()?;
            block_tbl.set(
                "abort",
                scope.create_function(|_, reason: Option<String>| {
                    block_ref.borrow_mut().aborted = Some(reason.unwrap_or_default());
                    Err::<(), _>(mlua::Error::RuntimeError("block aborted".to_owned()))
                })?,
            )?;

            self.lua.globals().set("memory", memory)?;
            self.lua.globals().set("block", block_tbl)?;

            // Inner result: the agent-visible outcome (a value, or a runtime error).
            Ok(self
                .lua
                .load(script)
                .eval::<Value>()
                .map(|value| render(&value)))
        });

        let BlockState {
            buffer,
            touched,
            aborted,
        } = block.into_inner();
        let touched: Vec<MemoryId> = touched.into_iter().collect();

        match evaluated {
            // Setup failed — a bug on our side, not an agent-visible outcome.
            Err(error) => Err(LuaError::Vm(error)),
            Ok(Ok(result)) => {
                // Commit the buffered side effects plus the LuaExecuted record, atomically.
                let mut events = buffer;
                events.push(self.lua_executed(
                    turn_id,
                    script,
                    Some(result.clone()),
                    touched,
                    None,
                ));
                self.finish(
                    store,
                    graph,
                    clock,
                    events,
                    BlockOutcome::Committed { result },
                )
            }
            Ok(Err(error)) => {
                // Discard the buffer; record only what the agent saw — the terminal cause.
                let cause = match aborted {
                    Some(reason) => TerminalCause::Aborted(reason),
                    None => TerminalCause::Error(error.to_string()),
                };
                let event = self.lua_executed(turn_id, script, None, touched, Some(cause.clone()));
                self.finish(
                    store,
                    graph,
                    clock,
                    vec![event],
                    BlockOutcome::Terminated(cause),
                )
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
        store: &mut dyn Store,
        graph: &mut Graph,
        clock: &dyn Clock,
        events: Vec<EventPayload>,
        outcome: BlockOutcome,
    ) -> Result<BlockOutcome, LuaError> {
        store.append(clock.now(), events)?;
        graph.materialize_from(store)?;
        Ok(outcome)
    }
}

/// Side effects and reads accumulated during one block, committed or discarded atomically.
#[derive(Default)]
struct BlockState {
    buffer: Vec<EventPayload>,
    touched: BTreeSet<MemoryId>,
    aborted: Option<String>,
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
            LuaError::Vm(error) => write!(f, "Lua VM error: {error}"),
            LuaError::Store(error) => write!(f, "{error}"),
            LuaError::Graph(error) => write!(f, "{error}"),
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

/// Resolve a name to a memory id, consulting the block's pending creates/renames before the graph.
fn resolve_name(
    state: &BlockState,
    graph: &Graph,
    name: &str,
) -> Result<Option<MemoryId>, GraphError> {
    for event in &state.buffer {
        match event {
            EventPayload::MemoryCreated { id, name: created } if created.as_str() == name => {
                return Ok(Some(*id));
            }
            EventPayload::MemoryRenamed { id, new_name, .. } if new_name.as_str() == name => {
                return Ok(Some(*id));
            }
            _ => {}
        }
    }
    Ok(graph.memory_by_name(name)?.map(|memory| memory.id))
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

fn to_lua_err(error: GraphError) -> mlua::Error {
    mlua::Error::RuntimeError(error.to_string())
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
