//! Search and reference-resolution failures for the Lua interface: `memory.search` backend errors,
//! `convo.turn` transcript-link resolution, and `memory.list` argument validation. The delegating
//! variants nest their inner error's own prefix, while the teachable variants stay unprefixed prose.

use mlua::Error as LuaError;
use ulid::DecodeError as UlidError;

use crate::{
    agent::{body_of, render_placeholders},
    memory::search::SearchError,
    model::ModelError,
    store::StoreError,
};

/// A failure running `memory.search` — the embedder/vector backends, or the absence of retrieval on
/// the instance. The delegating variants nest their inner error's own `model:`/`search (…):` prefix.
#[derive(Debug)]
pub(in crate::agent::lua) enum MemorySearchError {
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
            MemorySearchError::EmptyQuery => {
                f.write_str(&body_of(include_str!("prose/lookup/search_empty.md")))
            }
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
pub(in crate::agent::lua) enum TurnResolveError {
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
            TurnResolveError::AudienceMismatch { id } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/lookup/audience_mismatch.md")),
                &[("id", &format!("{id:?}"))],
            )),
            TurnResolveError::NotFound { id } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/lookup/not_found.md")),
                &[("id", &format!("{id:?}"))],
            )),
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
pub(in crate::agent::lua) enum ListError {
    /// `memory.list` was called with no prefix, or a blank one.
    EmptyPrefix,
}

impl std::fmt::Display for ListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ListError::EmptyPrefix => {
                f.write_str(&body_of(include_str!("prose/lookup/empty_prefix.md")))
            }
        }
    }
}

impl std::error::Error for ListError {}

impl From<ListError> for LuaError {
    fn from(error: ListError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}
