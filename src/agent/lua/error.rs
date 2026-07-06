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
    /// `calendar.upcoming`/`overdue` was given a window that is neither a duration string, an opts
    /// table, nor nil.
    NotAWindow { type_name: &'static str },
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
            CalendarError::NotAWindow { type_name } => write!(
                f,
                "the window is a duration — pass it directly (\"31 days\", \"2 weeks\") or as \
                 {{ within = \"…\" }}, or omit it for the default; got {type_name}"
            ),
        }
    }
}

impl std::error::Error for CalendarError {}

impl From<CalendarError> for LuaError {
    fn from(error: CalendarError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// A bad date value handed to a temporal surface — `calendar.on`, or the `occurred_at` option's `day`
/// and range positions — where a date object (from `calendar.today()` and its siblings) or a
/// `"YYYY-MM-DD"` string was wanted. Raised at the parsing seam that every `occurred_at` taker passes
/// through, so a date object stands in for a date string uniformly.
#[derive(Debug)]
pub(super) enum TemporalArgError {
    /// A value that is neither a date object nor a date string where a day was expected.
    NotADate { type_name: &'static str },
    /// A date string (or a date object's `day`) that is not a valid `YYYY-MM-DD` calendar date.
    InvalidDay { input: String },
}

impl std::fmt::Display for TemporalArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemporalArgError::NotADate { type_name } => write!(
                f,
                "expected a date object (from calendar.today(), calendar.next(...), …) or a \
                 \"YYYY-MM-DD\" string, got {type_name}"
            ),
            TemporalArgError::InvalidDay { input } => write!(
                f,
                "{input:?} is not a valid date; use YYYY-MM-DD, e.g. \"2026-06-03\""
            ),
        }
    }
}

impl std::error::Error for TemporalArgError {}

impl From<TemporalArgError> for LuaError {
    fn from(error: TemporalArgError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// A failure running `memory.search` — the embedder/vector backends, or the absence of retrieval on
/// the instance. The delegating variants nest their inner error's own `model:`/`search (…):` prefix.
#[derive(Debug)]
pub(super) enum MemorySearchError {
    /// No embedding endpoint is configured, so semantic search is unavailable.
    NoRetrieval,
    /// The query was empty or whitespace — semantic search needs something to match on. Caught
    /// before the embedder is called, so a degenerate "enumerate a namespace" query fails fast and
    /// teachably instead of embedding the empty string and grinding through every memory.
    EmptyQuery,
    Embed(ModelError),
    NoVector,
    Settings(StoreError),
    Search(SearchError),
}

impl std::fmt::Display for MemorySearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemorySearchError::EmptyQuery => write!(
                f,
                "memory.search needs a query to match on — an empty search cannot list a namespace. \
                 Search for what you are actually after in natural language, narrowing with the \
                 namespace option if you want to stay within one prefix, e.g. \
                 memory.search(\"deploy schedule\", {{ namespace = \"topic/\" }})."
            ),
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

/// A failure resolving a `convo.turn` transcript link. `InvalidTurnId`, `AudienceMismatch`, and
/// `NotFound` are teachable (the agent reads them and adapts); `Store` is an infrastructure read
/// failure surfaced to terminate the block cleanly, as `memory.search` does — turn resolution is
/// read-only, so a failure corrupts nothing. The two refusals are deliberately distinct:
/// `AudienceMismatch` says the id names a real moment the present audience did not all share (safe to
/// confirm, ULIDs being unguessable) and points at memory as the visibility-filtered channel;
/// `NotFound` says no such moment exists.
#[derive(Debug)]
pub(super) enum TurnResolveError {
    /// `convo.turn` was given a string that is not a ULID.
    InvalidTurnId { id: String, source: UlidError },
    /// The id names a real turn, but someone present here was not in that moment's audience.
    AudienceMismatch { id: String },
    /// No turn with this id exists anywhere in the log.
    NotFound { id: String },
    /// The event store could not be read.
    Store(StoreError),
}

impl std::fmt::Display for TurnResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TurnResolveError::InvalidTurnId { id, source } => {
                write!(f, "invalid turn id {id:?}: {source}")
            }
            TurnResolveError::AudienceMismatch { id } => write!(
                f,
                "turn {id:?} is a real moment, but its audience did not include everyone present \
                 here — so it cannot be replayed to this room. If its substance is worth relaying, \
                 recall it through memory — reconstruct it from every memory the moment plausibly \
                 touched, not the first hit — and honor each entry's own audience as you speak — \
                 the visibility rule governs what you say, not just what you look up."
            ),
            TurnResolveError::NotFound { id } => write!(
                f,
                "no turn {id:?} exists — the id must be one you were given (the value inside a \
                 [turn:<id>] token)"
            ),
            TurnResolveError::Store(error) => {
                write!(f, "could not read the conversation transcript: {error}")
            }
        }
    }
}

impl std::error::Error for TurnResolveError {}

impl From<TurnResolveError> for LuaError {
    fn from(error: TurnResolveError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// A bad argument to `memory.list`. The prefix is required and must be non-empty: an empty or
/// whitespace stem would list the whole graph, which is not what discovery-by-stem is for, so it is a
/// teachable error naming the shape and pointing at `memory.search` for recall-by-meaning.
#[derive(Debug)]
pub(super) enum ListError {
    /// `memory.list` was called with no prefix, or a blank one.
    EmptyPrefix,
}

impl std::fmt::Display for ListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ListError::EmptyPrefix => write!(
                f,
                "list finds handles by stem — pass a name prefix like \"person/\" or \"person/dav\"; \
                 to recall by meaning, memory.search"
            ),
        }
    }
}

impl std::error::Error for ListError {}

impl From<ListError> for LuaError {
    fn from(error: ListError) -> Self {
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
    /// A handle method was reached with a dot (`memory.append(...)`) rather than a colon
    /// (`memory:append(...)`), so the first argument bound to `self` — a string or number where the
    /// handle was wanted. Raised at the `self` extractor (the method's leftmost argument, converted
    /// first), so the agent sees this fix rather than mlua's opaque "error converting Lua string to
    /// table".
    MethodCalledWithDot { type_name: &'static str },
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
                "no memory named \"{name}\" — create it first, or check the casing"
            ),
            HandleError::WrongLinkTargetType { type_name } => write!(
                f,
                "link target must be a memory handle (from memory.get/create) or a memory name, \
                 got {type_name}"
            ),
            HandleError::MethodCalledWithDot { type_name } => write!(
                f,
                "this is a method — call it with a colon (handle:method(...)), not a dot \
                 (handle.method(...)); the colon passes the handle as self, but the dot bound \
                 {type_name} there instead"
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

/// A bad argument to `table.concat`, reworded from Luau's opaque native error. Stock Luau `table.concat`
/// joins only strings and numbers, so a reader's handle list — `mem:entries()`, `hub:links()` — fails
/// it with the unhelpful "invalid value (table) at index … in table for 'concat'". The thin wrapper in
/// [`super::runtime::install_table_concat`] keeps stock semantics (it delegates the join untouched) and
/// only replaces that error: these two variants redirect the two observed slips — a whole reader
/// *method* passed in place of its result, and a handle list that has no join, which now points at
/// string interpolation as the way to compose text.
#[derive(Debug)]
pub(super) enum ConcatError {
    /// The list argument was not a table at all — most often a reader *method* passed in place of its
    /// result (`hub.links`, a function, rather than `hub:links()`).
    NotAList { type_name: &'static str },
    /// The list held a value `table.concat` cannot join — a handle list, most likely. Composing text
    /// from handles is interpolation's job now, not concat's.
    NonJoinable,
}

impl std::fmt::Display for ConcatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConcatError::NotAList { type_name } => write!(
                f,
                "table.concat joins a list, but its first argument is {type_name}, not a table. If \
                 you meant a memory reader like hub:links() or mem:entries(), call it with a colon \
                 and parentheses — hub.links is the method itself, hub:links() is the list it returns"
            ),
            ConcatError::NonJoinable => write!(
                f,
                "table.concat joins only strings and numbers, but this list holds values it cannot — \
                 a handle list like mem:entries() or hub:links(), most likely. Composing text from \
                 handles is interpolation's job: interpolate one into a backtick string — \
                 `latest: {{es[1]}}` renders it as its text — or, since a handle concatenates \
                 directly (\"- \" .. e renders one), build the string with .. in a loop."
            ),
        }
    }
}

impl std::error::Error for ConcatError {}

impl From<ConcatError> for LuaError {
    fn from(error: ConcatError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// An attempt to assign to a field on a read-only handle (a memory, an entry, a date, or a search
/// result). A handle is a view, not a mutable record, so a field assignment silently did nothing —
/// the footgun behind the stale-date thrash, where `entry.occurred_at = ...` looked like it landed a
/// date but stored nothing. The message names the operation that actually persists the change.
#[derive(Debug)]
pub(super) enum HandleAssignmentError {
    /// Assigning `occurred_at` — a fact's date lives on an entry and is set when it is recorded, not
    /// by mutating a handle's field.
    OccurredAt { kind: HandleKind },
    /// Assigning any other field.
    Other { kind: HandleKind, field: String },
}

/// Which read-only handle an assignment was attempted on, for the assignment error's wording.
#[derive(Debug, Clone, Copy)]
pub(super) enum HandleKind {
    Memory,
    Entry,
    Date,
    SearchResult,
}

impl HandleKind {
    fn label(self) -> &'static str {
        match self {
            HandleKind::Memory => "memory handle",
            HandleKind::Entry => "entry",
            HandleKind::Date => "date object",
            HandleKind::SearchResult => "search result",
        }
    }
}

impl std::fmt::Display for HandleAssignmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandleAssignmentError::OccurredAt { kind } => write!(
                f,
                "occurred_at is not assignable on a {}: a fact's date is set when you record it, not \
                 by writing the field. Append the dated entry with occurred_at in its opts \
                 (mem:append(text, {{ occurred_at = calendar.date(\"YYYY-MM-DD\") }})), or revise the \
                 dated entry (mem:revise(entry, text, {{ occurred_at = ... }}))",
                kind.label()
            ),
            HandleAssignmentError::Other { kind, field } => write!(
                f,
                "{field} is not assignable: a {} is a read-only view, so writing its field does \
                 nothing. To change what is stored, use the memory methods (mem:append, mem:revise, \
                 mem:supersede, mem:rename, …) — a field assignment does not persist",
                kind.label()
            ),
        }
    }
}

impl std::error::Error for HandleAssignmentError {}

impl From<HandleAssignmentError> for LuaError {
    fn from(error: HandleAssignmentError) -> Self {
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
