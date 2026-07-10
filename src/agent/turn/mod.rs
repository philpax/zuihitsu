//! The agent loop: a turn is a loop of model steps (spec §Agent loop).
//!
//! Each step the model emits either `run_lua` tool calls or a terminal (a reply or a stay-silent),
//! never both. Tool calls execute as blocks (Stage 4a), their rendered results feed back into the
//! next step, and the loop continues until the model replies, stays silent, or hits `max_steps`.
//! Exactly one `role = agent` `ConversationTurn` is recorded per cycle, however it ends — a reply,
//! an empty silent terminal, or a surfaced `max_steps` error — so "the agent saw this and chose
//! its outcome" is always auditable. The inbound message is its own `role = participant` turn.

mod adjudicate;
mod describe;
mod error;
mod link_inference;

pub use adjudicate::run_adjudicate_catch_up;
pub use describe::{run_describe_catch_up, run_describe_catch_up_for};
pub use error::TurnError;
pub use link_inference::{
    InferredLink, LinkInferenceArgs, NewRelationSpec, run_link_inference_catch_up,
};

mod buffer;
mod record;
mod recording;
mod resolve;
mod run;
mod tools;

pub use buffer::{
    ToolStep, TurnView, bounded_buffer_turns, buffer_turns, carryover_start, flushed_up_to,
    session_touched,
};
pub use resolve::{ResolvedTurn, TurnResolution, TurnWindow, resolve_turn};
pub(crate) use run::run_flush;
pub use run::run_turn;

// Re-exports for the submodules that reference these via `super::`.
pub(super) use recording::{Recording, collect_written_memories};
pub(super) use run::participant_names;
pub(super) use tools::ToolError;

// Imports re-exported for submodules that use `use super::*` (buffer.rs, resolve.rs).
// Re-exports for submodules that use `super::*` (buffer.rs, resolve.rs).
#[allow(unused_imports)]
pub(super) use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
    time::Duration,
};

#[allow(unused_imports)]
pub(super) use schemars::JsonSchema;
pub(super) use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
pub(super) use sha2::Digest;

#[allow(unused_imports)]
pub(super) use crate::{
    clock::Clock,
    engine::Engine,
    event::{EventPayload, Initiation, ProducedBy, PromptTemplateName, Teller, TurnRole},
    ids::{ConversationId, MemoryId, MemoryName, Seq, SessionId, TurnId},
    memory::memory_block::Authority,
    model::{Message, ModelClient},
    prompt::PromptSectionSpan,
    settings::CaptureLevel,
    store::{Store, StoreError},
    time::Timestamp,
    turn_ref,
};

#[allow(unused_imports)]
pub(super) use super::{lua::Session, templates};

/// What a completed turn delivers to the platform client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum TurnOutcome {
    /// A reply to post back.
    Reply(String),
    /// The stay-silent terminal — nothing to post.
    Silent,
    /// The step budget was exhausted without a terminal; recorded for the agent to reason about.
    MaxStepsExceeded,
    /// The inbound message was delivered and durably recorded, but the model backend was
    /// unreachable (transient failure with retries exhausted, or an open circuit), so no response
    /// cycle ran. Nothing is lost, and catch-up is passive by design: the next inbound message's
    /// turn replays the buffer — which includes every deferred inbound — so one response cycle
    /// covers them all. There is no active on-recovery push, because replies have no delivery
    /// channel to platform clients besides the message-response path, and agent-initiated contact
    /// is a deliberately deferred design area.
    Deferred,
}

/// What a completed turn reports to the platform: its conversational `outcome` and the peak
/// `prompt_tokens` observed across the turn's generation steps — the largest the buffer reached, and
/// what the next turn would build on. `None` when no step reported usage (the platform then falls
/// back to a deterministic estimate). The platform compares this against the compaction budget.
/// `steps` and `blocks` carry the per-turn model-call and Lua-block counts the observability span
/// records (spec §Observability), so an operator reading the log can place a turn's latency without
/// re-reading the raw event log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnReport {
    pub outcome: TurnOutcome,
    pub prompt_tokens: Option<u32>,
    /// How many model `generate` steps the turn ran.
    pub steps: usize,
    /// How many `run_lua` blocks the turn executed.
    pub blocks: usize,
    /// The agent's response-cycle turn id — the durable key an operator uses to find this turn's
    /// events in the log. (The participant's inbound message carries its own earlier turn id.)
    pub turn_id: TurnId,
}

/// The write context one block — or a whole step loop — runs under: who its content is attributed
/// to (`teller`), the authority it writes with (gating `self` and the link source, see
/// [`Authority`]), and the turn id its events are stamped with.
#[derive(Clone)]
pub struct BlockContext {
    pub teller: Teller,
    pub authority: Authority,
    pub turn_id: TurnId,
    /// How long a single block may run before it is aborted, emitting nothing (spec §Concurrency →
    /// lock acquisition). Threaded from `TurnSettings::block_timeout_seconds`.
    pub block_timeout: Duration,
    /// How many times a lock-wait-timed-out block (with no MCP call) is retried before giving up.
    /// Threaded from `TurnSettings::max_block_attempts`.
    pub max_block_attempts: u32,
    /// The maximum character length of a single memory content entry. Threaded from
    /// `MemorySettings::max_entry_chars`.
    pub max_entry_chars: usize,
    /// Who is present in the conversation this block runs in — the set `memory.search` filters its
    /// entry hits against, so the agent never recalls a teller-private aside into a room where the
    /// teller is absent (spec §Visibility). The agent is always present to itself.
    pub present_set: Vec<MemoryId>,
    /// Run the block but commit nothing: the block executes against the live graph (reads see real
    /// memory), its rendered result is returned, and its buffered effects — including the
    /// `LuaExecuted` record — are discarded. The operator Lua console's no-commit sandbox; always
    /// `false` for a real turn.
    pub dry_run: bool,
}

/// Everything one turn needs: the conversation's `session`, the shared seams (`model` and the
/// `engine` backends), the `inbound` participant message and its `inbound_participant` (the
/// speaker's [`Namespace::Person`] stub, whose content the turn's writes are attributed to), and
/// the step budget.
pub struct Turn<'a> {
    pub session: &'a Session,
    pub model: &'a dyn ModelClient,
    pub engine: Arc<Engine>,
    pub inbound: &'a str,
    pub inbound_participant: MemoryId,
    /// The session's frozen contextual brief, interpolated into the system prompt (captured on
    /// `SessionStarted`, so every turn in the session sees the same brief).
    pub brief: &'a str,
    /// When the session opened, frozen into the system prompt's "the session begins on …". Held
    /// stable across the session's turns (the live time rides in the per-message stamps) so the system
    /// prefix is identical turn to turn and the serving layer can reuse its prefix cache.
    pub session_started_at: Timestamp,
    /// The live buffer recorded before this inbound message — the session's prior turns, replayed as
    /// the prompt suffix after the frozen prefix ([`buffer_turns`]). Empty for the first turn of a
    /// session (or whenever the caller wants a single-message prompt).
    pub buffer: &'a [TurnView],
    /// Which prompt template frames the system prompt and stamps the agent turn's provenance:
    /// `Scaffold` for an ordinary participant turn, `Imprint` for the console imprint interview.
    pub template: PromptTemplateName,
    /// The authority the turn's writes run under — `Platform` for a participant turn, `Operator` for
    /// the imprint interview (the only authority that may write `self`).
    pub authority: Authority,
    /// Who is present in the conversation — the visibility set `memory.search` filters against (see
    /// [`BlockContext::present_set`]).
    pub present_set: &'a [MemoryId],
    pub max_steps: usize,
    /// Per-block duration budget (spec §Concurrency); each block this turn runs is aborted if it
    /// exceeds it.
    pub block_timeout: Duration,
    /// Per-block retry bound for a lock-wait timeout (spec §Concurrency).
    pub max_block_attempts: u32,
    /// The maximum character length of a single memory content entry. Threaded from
    /// `MemorySettings::max_entry_chars`.
    pub max_entry_chars: usize,
    /// How much of each model call to capture in the model-interaction record (spec §Observability).
    pub capture: CaptureLevel,
}

/// Everything the pre-compaction flush turn needs (spec §Compaction → pre-compaction flush). Like
/// [`Turn`], but there is no inbound participant message — the flush acts on the session `buffer`
/// alone, framed by the `Flush` template — and its writes are the agent's own (teller `Agent`).
pub(crate) struct Flush<'a> {
    pub session: &'a Session,
    pub model: &'a dyn ModelClient,
    pub engine: Arc<Engine>,
    pub brief: &'a str,
    /// When the session opened, frozen into the system prompt's time so the flush sends the same system
    /// prefix the session's live turns did (see [`Turn::session_started_at`]).
    pub session_started_at: Timestamp,
    pub buffer: &'a [TurnView],
    /// The session's participants — the visibility set the flush's `memory.search` filters against
    /// (see [`BlockContext::present_set`]).
    pub present_set: &'a [MemoryId],
    pub max_steps: usize,
    /// Per-block duration budget (spec §Concurrency); each block the flush runs is aborted if it
    /// exceeds it.
    pub block_timeout: Duration,
    /// Per-block retry bound for a lock-wait timeout (spec §Concurrency).
    pub max_block_attempts: u32,
    /// The maximum character length of a single memory content entry. Threaded from
    /// `MemorySettings::max_entry_chars`.
    pub max_entry_chars: usize,
    /// How much of each model call to capture in the model-interaction record (spec §Observability).
    pub capture: CaptureLevel,
}

/// The shared step loop a participant turn and a pre-compaction flush both run.
pub(super) struct Steps<'a> {
    pub(super) session: &'a Session,
    pub(super) model: &'a dyn ModelClient,
    pub(super) engine: Arc<Engine>,
    pub(super) system: &'a str,
    /// The typed section spans of `system`, recorded on the phase's [`crate::event::RequestRecord::Base`]
    /// so the console can break the prompt into its parts without re-deriving the boundaries.
    pub(super) system_sections: &'a [PromptSectionSpan],
    pub(super) context: BlockContext,
    pub(super) messages: Vec<Message>,
    pub(super) initiation: Initiation,
    pub(super) provenance: Option<ProducedBy>,
    pub(super) max_steps: usize,
    pub(super) capture: CaptureLevel,
}

#[cfg(test)]
mod tests;
