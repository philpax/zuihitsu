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

mod commit;
mod error;
mod execute;
mod reference;
mod runtime;
mod session;
mod tables;

use mlua::{Lua, LuaOptions, StdLib, Value};

use crate::{InstanceFeatures, graph::GraphError, ids::ConversationId, store::StoreError};

pub use reference::{api_reference, render_api_reference};

/// One conversation's VM. Globals persist across the session's blocks; the memory API is installed
/// fresh per block, while the MCP projection (when configured) is installed once and persists like the
/// agent scratchpad.
pub struct Session {
    pub(super) lua: Lua,
    /// The conversation this session's blocks write in, or `None` for the operator Lua console: a
    /// throwaway sandbox that has no room, so its (discarded) writes attribute to no conversation and
    /// `context.current` is nil, matching the other conversation-less operator paths (self-edit,
    /// retraction). A live turn always has one.
    pub(super) conversation: Option<ConversationId>,
    /// The session's MCP state — the host, configured servers, and lazily-spawned instances backing the
    /// `mcp.<server>.*` projection — or `None` when no host is configured.
    pub(super) mcp: Option<std::sync::Arc<crate::agent::mcp_api::McpSession>>,
    /// The web fetcher and its Markdown cap backing `web.markdown`, or `None` when no fetcher is
    /// connected. Installed per block, gated on the `browsing` feature, like the memory API — not
    /// once like the MCP projection, since it holds no per-session state (a fetch is stateless).
    pub(super) web: Option<crate::web::WebClient>,
    /// Which API features this session enables — gates the Lua functions installed per block, the API
    /// reference rendered into the system prompt, and (at genesis) the scaffold dotpoints. Read fresh
    /// each block so it always reflects the instance's current features.
    pub(super) features: InstanceFeatures,
}

/// The result of executing one block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockOutcome {
    /// The block committed; `result` is the rendered value of its final expression.
    Committed { result: String },
    /// The block ended without committing (its buffer was discarded), for this reason.
    Terminated(crate::event::TerminalCause),
    /// The block committed its buffered writes, but a `turn.skip()` signalled the turn should end
    /// silently. Unlike `Terminated`, the writes are durable; the skip only suppresses the reply.
    Skipped(Option<String>),
}

/// Construct the block VM with a deliberately narrow surface: a memory block is an orchestration
/// script over the projected API (`memory`, `block`, `context`, `mcp`, …), never a host program, so it
/// must not reach the filesystem, the environment, the process, or arbitrary code on disk. MCP is the
/// only sanctioned outward reach (spec §External I/O via MCP).
///
/// The VM is Luau, whose sandboxing-first design suits executing model-written code. Only the pure
/// libraries are loaded — string, table, math, utf8, and coroutine — so `os`, `io`, `package` (and thus
/// `require`), `debug`, and the FFI/JIT escapes are never present. Dropping `os` also keeps blocks
/// deterministic under replay: there is no wall-clock `os.time`/`os.date`, so time only ever comes from
/// the injected clock. Luau already omits the dynamic code-loading globals (`load`, `loadstring`,
/// `dofile`, `loadfile`) and has no `require`, so the surface is narrow to begin with; we defensively
/// clear any a future revision might reintroduce.
///
/// After the environment is fixed, [`Lua::sandbox`] freezes it: the global table and the standard
/// libraries become read-only to scripts, so a block cannot monkey-patch `string`/`table` to reshape
/// the API or smuggle state across blocks through a mutated library. Host-side installs still write
/// globals freely (the read-only bar is on scripts, not the Rust API), so the per-block memory API and
/// the persistent scratchpad both keep working — a script's own new globals persist across the session's
/// blocks as before. `print` and `inspect` are installed per block; the lenient-error `table.concat`
/// shell and `inspect` are installed here, before the freeze, so they are part of the frozen surface.
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
    runtime::install_inspect(&lua).expect("installing the inspect global");
    runtime::install_table_concat(&lua).expect("installing the table.concat error shell");
    lua.sandbox(true)
        .expect("enabling the Luau sandbox for the global environment");
    lua
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
