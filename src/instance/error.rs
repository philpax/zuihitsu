//! The instance-side error type — the failures an [`crate::instance::Instance`] operation surfaces, delegating
//! to the underlying subsystem error (store, graph, turn, MCP, index, search, Lua).

use crate::{
    agent::lua::LuaError,
    graph::GraphError,
    ids::ConversationId,
    memory::{
        brief::BriefError, identity::IdentityError, memory_block::MemoryError,
        scheduler::SchedulerError,
    },
    model::index::IndexError,
    store::StoreError,
};

use crate::{agent::TurnError, mcp::McpError, memory::search::SearchError};

/// An instance-side failure, delegating its message to the underlying error.
#[derive(Debug)]
pub enum InstanceError {
    Store(StoreError),
    Graph(GraphError),
    /// A turn (the agent loop) failed while routing a message. `conversation` is `Some` for a
    /// routed turn or flush (the common case) and `None` for a background catch-up (describe), which
    /// spans all conversations rather than one.
    Turn {
        conversation: Option<ConversationId>,
        error: TurnError,
    },
    /// Connecting the MCP servers failed (a probe-level hard error, e.g. a stale `allow`/`deny`).
    Mcp(McpError),
    /// Writing a graph snapshot failed (creating the directory, or the `VACUUM INTO` itself).
    Snapshot(String),
    /// Catching the vector index up to the log failed (embedding, the vector store, or the log read).
    Index(IndexError),
    /// A semantic search failed (the graph projection or the vector index).
    Search(SearchError),
    /// An operator Lua console block failed at the VM level (a script error reaches the operator as
    /// a result, not this; this is an infrastructure failure running the block). `conversation` is the
    /// dedicated console conversation, packed at the call boundary.
    Lua {
        conversation: Option<ConversationId>,
        error: LuaError,
    },
    /// A direct operator memory write (the console `self` edit) failed inside the block. The
    /// operator-input cases the edit anticipates are returned as outcomes, not this; this carries a
    /// genuinely unexpected block failure that should not arise under operator authority.
    Memory(MemoryError),
}

impl std::fmt::Display for InstanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstanceError::Store(error) => write!(f, "instance (store): {error}"),
            InstanceError::Graph(error) => write!(f, "instance (graph): {error}"),
            InstanceError::Turn {
                conversation,
                error,
            } => match conversation {
                Some(id) => write!(f, "instance (turn {}): {error}", id.0),
                None => write!(f, "instance (turn): {error}"),
            },
            InstanceError::Mcp(error) => write!(f, "instance (mcp): {error}"),
            InstanceError::Snapshot(message) => write!(f, "instance (snapshot): {message}"),
            InstanceError::Index(error) => write!(f, "instance (index): {error}"),
            InstanceError::Search(error) => write!(f, "instance (search): {error}"),
            InstanceError::Lua {
                conversation,
                error,
            } => match conversation {
                Some(id) => write!(f, "instance (lua {}): {error}", id.0),
                None => write!(f, "instance (lua): {error}"),
            },
            InstanceError::Memory(error) => write!(f, "instance (memory): {error}"),
        }
    }
}

impl std::error::Error for InstanceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            InstanceError::Store(error) => Some(error),
            InstanceError::Graph(error) => Some(error),
            InstanceError::Turn { error, .. } => Some(error),
            InstanceError::Mcp(error) => Some(error),
            InstanceError::Snapshot(_) => None,
            InstanceError::Index(error) => Some(error),
            InstanceError::Search(error) => Some(error),
            InstanceError::Lua { error, .. } => Some(error),
            InstanceError::Memory(error) => Some(error),
        }
    }
}

impl From<SearchError> for InstanceError {
    fn from(error: SearchError) -> Self {
        InstanceError::Search(error)
    }
}

impl From<IndexError> for InstanceError {
    fn from(error: IndexError) -> Self {
        InstanceError::Index(error)
    }
}

impl From<McpError> for InstanceError {
    fn from(error: McpError) -> Self {
        InstanceError::Mcp(error)
    }
}

impl From<StoreError> for InstanceError {
    fn from(error: StoreError) -> Self {
        InstanceError::Store(error)
    }
}

impl From<GraphError> for InstanceError {
    fn from(error: GraphError) -> Self {
        InstanceError::Graph(error)
    }
}

// Identity and brief resolution fail only into store/graph errors, so they map onto the existing
// variants rather than widening the enum; the agent loop's richer `TurnError` keeps its own.
impl From<IdentityError> for InstanceError {
    fn from(error: IdentityError) -> Self {
        match error {
            IdentityError::Store { source, .. } => InstanceError::Store(source),
            IdentityError::Graph { source, .. } => InstanceError::Graph(source),
        }
    }
}

impl From<BriefError> for InstanceError {
    fn from(error: BriefError) -> Self {
        match error {
            BriefError::Graph(error) => InstanceError::Graph(error),
        }
    }
}

impl From<SchedulerError> for InstanceError {
    fn from(error: SchedulerError) -> Self {
        match error {
            SchedulerError::Store(error) => InstanceError::Store(error),
            SchedulerError::Graph(error) => InstanceError::Graph(error),
        }
    }
}

impl From<TurnError> for InstanceError {
    fn from(error: TurnError) -> Self {
        InstanceError::Turn {
            conversation: None,
            error,
        }
    }
}

impl From<LuaError> for InstanceError {
    fn from(error: LuaError) -> Self {
        InstanceError::Lua {
            conversation: None,
            error,
        }
    }
}
