//! Lua block-transactionality tests: a block's writes commit atomically and project to the graph;
//! reads see the block's own pending writes; scratchpad globals persist across the session's
//! blocks; and an abort or runtime error discards the buffer while recording the terminal cause
//! (spec §Lua API → block transactionality).
//!
//! The suite is partitioned by API surface into sibling modules; shared imports, helpers, and the
//! block-budget constants live here and reach each module through `use super::*`.

#[path = "../common/mod.rs"]
mod common;

pub(crate) use std::{sync::Arc, time::Duration};

pub(crate) use common::Harness;
pub(crate) use zuihitsu::{
    Authority, BEFORE_AFTER_EPSILON_MILLIS, BlockContext, BlockOutcome, Cardinality, CivilDate,
    Clock, Completion, ConversationLocator, Engine, Graph, InstanceFeatures, ManualClock, MemoryId,
    MemoryName, MemoryStore, Namespace, PromptTemplateName, RelationName, ScriptedModel, Session,
    SessionId, Store, TagName, Teller, TemporalRef, TerminalCause, Timestamp, TurnId, TurnRole,
    Visibility,
    event::{ArbitrationResolution, EventPayload, EventSource, Initiation},
    ids::ConversationId,
    resolve_or_mint_conversation, turn_ref,
};

/// A block-duration budget generous enough that these in-memory blocks never trip it.
pub(crate) const TEST_BLOCK_TIMEOUT: Duration = Duration::from_secs(30);
/// The per-block lock-wait retry bound for these tests.
pub(crate) const TEST_MAX_BLOCK_ATTEMPTS: u32 = 3;

mod block;
mod calendar;
mod convo;
mod dates;
mod handles;
mod honesty;
mod links;
mod merge;
mod occurred_at;
mod rename;
mod search;
mod tags;
