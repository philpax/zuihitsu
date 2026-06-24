//! The typed errors the Lua interface raises — calendar arguments, memory-search failures, handle
//! resolution, and block-consistency invariants. Each carries the offending value and the constraint,
//! and converts to `mlua::Error::RuntimeError` via its `Display`, so the agent-facing wording lives in
//! one place alongside the structured context (CONTRIBUTING: structured error types). The
//! agent-facing teachable messages are deliberately unprefixed prose — the agent reads them, not an
//! operator — while the delegating variants (search, embed) nest their inner error's own prefix.

use mlua::Error as LuaError;
use ulid::DecodeError as UlidError;

use crate::{memory::search::SearchError, model::ModelError, store::StoreError};

/// A bad argument to a `calendar.*` constructor or a date-arithmetic method.
#[derive(Debug)]
pub(super) enum CalendarError {
    /// `calendar.next` was given a string that is not a full weekday name.
    NotAWeekday { input: String },
    /// `calendar.in_days`/`in_weeks` shifted past the representable date range.
    DateOutOfRange { days: i64 },
    /// `calendar.date` was given a string that is not `YYYY-MM-DD`.
    InvalidDate { input: String },
    /// A date object's `day` field could not be interpreted — only reachable if one was corrupted,
    /// since the constructors validate before minting a date.
    InvalidDay { input: String },
}

impl std::fmt::Display for CalendarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CalendarError::NotAWeekday { input } => write!(
                f,
                "{input:?} is not a weekday; use a full name like \"monday\" or \"friday\""
            ),
            CalendarError::DateOutOfRange { days } => write!(
                f,
                "the date {days} days from today is out of range; use a smaller offset"
            ),
            CalendarError::InvalidDate { input } => write!(
                f,
                "{input:?} is not a valid date; use YYYY-MM-DD, e.g. \"2026-06-03\""
            ),
            CalendarError::InvalidDay { input } => write!(f, "{input:?} is not a valid day"),
        }
    }
}

impl std::error::Error for CalendarError {}

impl From<CalendarError> for LuaError {
    fn from(error: CalendarError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// A failure running `memory.search` — the embedder/vector backends, or the absence of retrieval on
/// the instance. The delegating variants nest their inner error's own `model:`/`search (…):` prefix.
#[derive(Debug)]
pub(super) enum MemorySearchError {
    /// No embedding endpoint is configured, so semantic search is unavailable.
    NoRetrieval,
    Embed(ModelError),
    NoVector,
    Settings(StoreError),
    Search(SearchError),
}

impl std::fmt::Display for MemorySearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemorySearchError::NoRetrieval => write!(
                f,
                "semantic search is unavailable on this instance (no embedding endpoint configured)"
            ),
            MemorySearchError::Embed(error) => {
                write!(f, "embedding the search query failed: {error}")
            }
            MemorySearchError::NoVector => write!(f, "the embedder returned no vector"),
            MemorySearchError::Settings(error) => {
                write!(f, "could not read the search settings: {error}")
            }
            // `SearchError` already carries a `search (…):` layer prefix.
            MemorySearchError::Search(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for MemorySearchError {}

impl From<MemorySearchError> for LuaError {
    fn from(error: MemorySearchError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// A bad handle or link target passed to a memory operation.
#[derive(Debug)]
pub(super) enum HandleError {
    /// A memory handle's `id` is not a ULID.
    InvalidMemoryHandle { id: String, source: UlidError },
    /// An entry handle's `id` is not a ULID.
    InvalidEntryHandle { id: String, source: UlidError },
    /// `:link`/`:unlink` was given a name string that is not a known memory.
    UnknownLinkTarget { name: String },
    /// `:link`/`:unlink` was given a value that is neither a handle nor a name string.
    WrongLinkTargetType { type_name: &'static str },
}

impl std::fmt::Display for HandleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandleError::InvalidMemoryHandle { id, source } => {
                write!(f, "invalid memory handle id {id:?}: {source}")
            }
            HandleError::InvalidEntryHandle { id, source } => {
                write!(f, "invalid entry handle id {id:?}: {source}")
            }
            HandleError::UnknownLinkTarget { name } => write!(
                f,
                "link target \"{name}\" is not a known memory — pass a handle from memory.get or \
                 memory.create, or an existing memory's name"
            ),
            HandleError::WrongLinkTargetType { type_name } => write!(
                f,
                "link target must be a memory handle (from memory.get/create) or a memory name, \
                 got {type_name}"
            ),
        }
    }
}

impl std::error::Error for HandleError {}

impl From<HandleError> for LuaError {
    fn from(error: HandleError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// A block-consistency invariant: an entry the block just buffered could not be read back. A bug,
/// not the agent's doing — surfaced as a catchable error rather than a panic so a misbehaving build
/// does not tear the whole session down.
#[derive(Debug)]
pub(super) enum BlockConsistencyError {
    AppendedEntryMissing,
    RevisedEntryMissing,
}

impl std::fmt::Display for BlockConsistencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockConsistencyError::AppendedEntryMissing => {
                write!(f, "the appended entry was not found in the block buffer")
            }
            BlockConsistencyError::RevisedEntryMissing => {
                write!(f, "the revised entry was not found in the block buffer")
            }
        }
    }
}

impl std::error::Error for BlockConsistencyError {}

impl From<BlockConsistencyError> for LuaError {
    fn from(error: BlockConsistencyError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}
