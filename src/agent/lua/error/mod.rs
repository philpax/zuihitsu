//! The typed errors the Lua interface raises — calendar arguments, memory-search failures, handle
//! resolution, and block-consistency invariants. Each carries the offending value and the constraint,
//! and converts to `mlua::Error::RuntimeError` via its `Display`, so the agent-facing wording lives in
//! one place alongside the structured context (CONTRIBUTING: structured error types). The
//! agent-facing teachable messages are deliberately unprefixed prose — the agent reads them, not an
//! operator — while the delegating variants (search, embed) nest their inner error's own prefix.

mod args;
mod block;
mod handle;
mod lookup;

pub(super) use args::{ArgError, CalendarError, TemporalArgError};
pub(super) use block::{
    BlockConsistencyError, ConcatError, MissingReturnError, SearchWriteError, TaintedWriteError,
};
pub(super) use handle::{
    FindEntryError, HandleAssignmentError, HandleError, HandleKind, PlaceholderError,
};
pub(super) use lookup::{ListError, MemorySearchError, TurnResolveError};
