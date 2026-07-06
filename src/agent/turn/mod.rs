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

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    clock::Clock,
    engine::Engine,
    event::{
        EventPayload, Initiation, ModelPhase, ProducedBy, PromptTemplateName, RequestRecord,
        Teller, TerminalCause, TurnRole,
    },
    ids::{ConversationId, MemoryId, MemoryName, Namespace, Seq, SessionId, TurnId},
    memory::memory_block::Authority,
    metrics::{
        observe_lua_block, observe_lua_block_error, observe_model_call, observe_turn_deferred,
    },
    model::{
        Completion, GenerateRequest, GenerateResponse, Message, ModelClient, ModelError, ToolCall,
        ToolChoice, ToolSpec, schema_of,
    },
    settings::CaptureLevel,
    store::{Store, StoreError},
    time::{self, Timestamp},
    turn_ref,
};

use super::{
    lua::{self, BlockOutcome, Session},
    system_prompt, templates,
};

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

/// One tool-call step within an agent turn: the `run_lua` script the model asked to run and the
/// result it saw back. Reconstructed from `LuaExecuted` events so the next turn's buffer carries the
/// full tool-interaction history — the model sees what it already fetched, searched, or computed
/// and does not re-issue the same call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolStep {
    pub script: String,
    pub result: String,
}

/// One turn replayed into the live buffer — the conversational surface the next turn sees as the
/// prompt suffix. Carries the durable turn text and the `run_lua` steps the agent ran this turn
/// (script + result), so the model re-sees what it already did — what it fetched, searched, or
/// wrote — and does not re-issue it next turn. `seq` and `turn_id` let a compaction mark the
/// carried tail (`seeded_from_turn` and the next buffer's start).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnView {
    pub seq: Seq,
    pub turn_id: TurnId,
    pub role: TurnRole,
    pub text: String,
    pub participant: Option<MemoryId>,
    /// When the turn was recorded — the time it is stamped with when replayed (spec §Time → "Now").
    pub recorded_at: Timestamp,
    /// The `run_lua` steps this turn's agent response ran, in order. Empty for participant/system
    /// turns, and for an agent turn that ran no blocks (a direct reply).
    pub steps: Vec<ToolStep>,
    /// The provenance the turn was recorded with — which template drove an agent turn. What lets a
    /// buffer scan recognize a flush turn (its `template_name` is `Flush`) and derive the session's
    /// flush watermark ([`flushed_up_to`]). `None` for participant/system turns and for agent turns
    /// recorded before provenance existed.
    pub produced_by: Option<ProducedBy>,
}

/// The `conversation`'s `ConversationTurn`s recorded at or after `from_seq`, oldest first — the live
/// buffer the next turn replays as the prompt suffix (spec §Conversations → the live buffer).
/// `from_seq` is the live session's start (so the whole session is read) or a carried tail across a
/// compaction seam (so only the carryover plus the new session's turns are read).
pub fn buffer_turns(
    store: &dyn Store,
    conversation: ConversationId,
    from_seq: Seq,
) -> Result<Vec<TurnView>, StoreError> {
    let mut turns = Vec::new();
    // A turn's `run_lua` blocks commit (and record their `LuaExecuted`) before the agent's reply turn,
    // both stamped with the same `turn_id` — so accumulate each turn's tool-call steps and attach them
    // to that turn's agent `TurnView` when it arrives.
    let mut steps_by_turn: BTreeMap<TurnId, Vec<ToolStep>> = BTreeMap::new();
    for event in store.read_from(from_seq)? {
        match event.payload {
            EventPayload::LuaExecuted {
                conversation: turn_conversation,
                turn_id,
                script,
                result,
                terminal_cause,
                ..
            } if turn_conversation == conversation => {
                let result = result.unwrap_or_else(|| {
                    terminal_cause
                        .as_ref()
                        .map(|cause| ToolError::from(cause.clone()).to_string())
                        .unwrap_or_default()
                });
                steps_by_turn
                    .entry(turn_id)
                    .or_default()
                    .push(ToolStep { script, result });
            }
            EventPayload::ConversationTurn {
                conversation: turn_conversation,
                turn_id,
                role,
                text,
                participant,
                produced_by,
                ..
            } if turn_conversation == conversation => {
                let steps = if role == TurnRole::Agent {
                    steps_by_turn.remove(&turn_id).unwrap_or_default()
                } else {
                    Vec::new()
                };
                turns.push(TurnView {
                    seq: event.seq,
                    turn_id,
                    role,
                    text,
                    participant,
                    recorded_at: event.recorded_at,
                    steps,
                    produced_by,
                });
            }
            _ => {}
        }
    }
    Ok(turns)
}

/// One conversation turn resolved for the `convo.turn` transcript link resolver (spec §Transcripts):
/// its stable id, who spoke, its role, its text, when it was recorded, and a ready-made canonical
/// reference to cite it by. `speaker` is the participant's conversational display name for a
/// participant turn, `self` for the agent's own turn, and `system` for an injected system turn.
/// `reference` is the `[turn:<ulid>]` token ([`turn_ref::construct`]) — the agent copies it to cite
/// the moment, so citation syntax lives in one place, never hand-assembled from the id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTurn {
    pub turn_id: TurnId,
    pub role: TurnRole,
    pub speaker: String,
    pub text: String,
    pub recorded_at: Timestamp,
    pub reference: String,
}

/// A resolved turn together with a small window of the turns immediately around it within its own
/// session, in chronological order. `focus` indexes the requested turn within `turns`. The window is
/// clamped to the focal turn's session and each neighbor is filtered by the same audience rule as the
/// focal turn, so a mid-session join never widens the window past what the current present set shared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnWindow {
    pub turns: Vec<ResolvedTurn>,
    pub focus: usize,
}

/// The outcome of resolving a turn reference, with its two refusals deliberately distinct (spec
/// §Transcripts → the two refusal tiers).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnResolution {
    /// The reference names a turn the current present set was all party to: the moment and its window.
    Resolved(TurnWindow),
    /// The reference names a real turn, but the audience rule fails — someone present here was not in
    /// that moment's audience. Refused without the content or the source room, so the agent recalls
    /// through the visibility-filtered memory channel instead of replaying a transcript to a new
    /// audience.
    AudienceMismatch,
    /// The reference names no turn anywhere in the log — an unknown or (having been parsed) a
    /// well-formed but never-recorded id.
    NotFound,
}

/// Resolve a turn id to that moment plus a window of `before`/`after` surrounding turns, under one
/// audience rule for every conversation: the turn resolves iff **every member of `present_set` was in
/// that turn's audience** (spec §Transcripts → the audience rule). A turn's audience is derived from
/// the log — its session's `SessionStarted.participants` plus every `ParticipantJoined` for that
/// session up to the turn's seq — so the check is "was everyone here also there then."
///
/// This is a deliberate loosening from the v1 same-room scope: a solo DM (present is just the
/// requester) resolves any turn the requester attended in any room, and a two-person DM resolves turns
/// whose audience included both. The distinction between [`TurnResolution::AudienceMismatch`] and
/// [`TurnResolution::NotFound`] confirms, on a cross-room id, that the id maps to *something* — which
/// is safe because ULIDs are unguessable: holding one means you were there, or someone who was there
/// gave you the link.
///
/// The whole log is read — turns are event-sourced, not materialized in the graph, so a store scan is
/// the read shape available. No content visibility filtering is applied within the window: it is a
/// transcript replay of a moment the whole current present set was party to (that is exactly what the
/// audience rule enforces), so it opens no new visibility surface (spec §Visibility). System turns
/// (join briefs, drained wake-ups) resolve too — they were injected into that session and read there.
pub fn resolve_turn(
    engine: &Engine,
    present_set: &[MemoryId],
    turn_id: TurnId,
    before: usize,
    after: usize,
) -> Result<TurnResolution, StoreError> {
    // Read every turn's audience off the store lock, then resolve speakers off the graph lock — the
    // two locks are taken in sequence, never held together, so this read observes the graph-before-
    // store ordering without violating it.
    let turns = {
        let store = engine.store.lock();
        audience_turns(store.as_ref())?
    };
    let Some(focus) = turns.iter().find(|turn| turn.view.turn_id == turn_id) else {
        return Ok(TurnResolution::NotFound);
    };
    // Resolve every id the audience rule will weigh — the present set and every turn's audience — to
    // its `same_as` class, so the rule compares identities rather than raw stubs. An operator-asserted
    // cross-platform merge (`person/maya@direct` same_as `person/maya@discord`) then reads as one
    // person: the merged identity resolves a turn recorded under either stub, consistent with how
    // class-wide reads (`class_entries`) already treat a merge (spec §Visibility). Built once off a
    // single graph lock, taken after the store lock above is released (graph-before-store ordering is
    // for holding both at once, not for taking them in sequence).
    let class_of = class_map(engine, present_set, &turns);
    if !audience_admits(present_set, &focus.audience, &class_of) {
        // The id maps to a real turn, but not one everyone present shared — a distinct, teachable
        // refusal that (safely, ULIDs being unguessable) confirms existence.
        return Ok(TurnResolution::AudienceMismatch);
    }
    // Clamp the window to the focal turn's own session (a session belongs to one conversation, so its
    // turns are the transcript neighbors), then keep only neighbors the present set also shared — a
    // mid-session join changes the audience within a session, so an earlier neighbor may fail the rule
    // the focal turn passed.
    let session_turns: Vec<&AudienceTurn> = turns
        .iter()
        .filter(|turn| turn.conversation == focus.conversation && turn.session == focus.session)
        .collect();
    let focus_idx = session_turns
        .iter()
        .position(|turn| turn.view.turn_id == turn_id)
        .expect("the focal turn is in its own session");
    let start = focus_idx.saturating_sub(before);
    let end = focus_idx
        .saturating_add(after)
        .saturating_add(1)
        .min(session_turns.len());
    let admitted: Vec<&AudienceTurn> = session_turns[start..end]
        .iter()
        .copied()
        .filter(|turn| audience_admits(present_set, &turn.audience, &class_of))
        .collect();
    let focus_position = admitted
        .iter()
        .position(|turn| turn.view.turn_id == turn_id)
        .expect("the focal turn passed the audience rule, so it survives the filter");
    let views: Vec<TurnView> = admitted.iter().map(|turn| turn.view.clone()).collect();
    let names = participant_names(engine, &views, &[]);
    let resolved = views
        .iter()
        .map(|turn| ResolvedTurn {
            turn_id: turn.turn_id,
            role: turn.role,
            speaker: turn_speaker(turn, &names),
            text: turn.text.clone(),
            recorded_at: turn.recorded_at,
            reference: turn_ref::construct(turn.turn_id),
        })
        .collect();
    Ok(TurnResolution::Resolved(TurnWindow {
        turns: resolved,
        focus: focus_position,
    }))
}

/// One conversation turn read for the resolver, carrying enough to apply the audience rule: the turn's
/// [`TurnView`] fields, which conversation and session it belongs to, and the audience that had
/// accumulated by its seq (its session's opening participants plus every mid-session joiner up to it).
struct AudienceTurn {
    view: TurnView,
    conversation: ConversationId,
    /// The session the turn was recorded in, or `None` for a turn outside any open session (which
    /// should not arise in normal operation, but is handled so a stray turn resolves only to itself).
    session: Option<SessionId>,
    audience: Vec<MemoryId>,
}

/// Whether every member of `present_set` is in `audience` — the audience rule, compared by `same_as`
/// class rather than by raw id. Each side is mapped through `class_of` (a member with no class in the
/// graph stands for itself), so a present member is admitted when any of its class siblings was in the
/// audience. This makes an operator-merged cross-platform identity one person for transcript
/// resolution, matching how class-wide reads treat a merge. An empty present set is vacuously admitted
/// (there is no one to have been excluded).
fn audience_admits(
    present_set: &[MemoryId],
    audience: &[MemoryId],
    class_of: &HashMap<MemoryId, MemoryId>,
) -> bool {
    let class = |id: &MemoryId| class_of.get(id).copied().unwrap_or(*id);
    let audience_classes: BTreeSet<MemoryId> = audience.iter().map(class).collect();
    present_set
        .iter()
        .all(|member| audience_classes.contains(&class(member)))
}

/// Map every id the audience rule weighs — the present set and every turn's audience — to its
/// `same_as`-class id, off a single graph lock. A member whose class cannot be read (unknown, soft-
/// deleted, or a graph error) is simply absent, so [`audience_admits`] falls back to its raw id — the
/// strict pre-merge behavior, never a looser one. The lock is taken after the resolver's store read is
/// released, so it never holds the store and graph guards together.
fn class_map(
    engine: &Engine,
    present_set: &[MemoryId],
    turns: &[AudienceTurn],
) -> HashMap<MemoryId, MemoryId> {
    let graph = engine.graph.lock();
    let mut class_of = HashMap::new();
    for id in present_set
        .iter()
        .copied()
        .chain(turns.iter().flat_map(|turn| turn.audience.iter().copied()))
    {
        if let std::collections::hash_map::Entry::Vacant(entry) = class_of.entry(id)
            && let Ok(Some(class)) = graph.class_id(id)
        {
            entry.insert(class);
        }
    }
    class_of
}

/// Every `ConversationTurn` in the log, each tagged with the audience in effect at its seq. One
/// forward pass tracks the open session per conversation and the audience accumulating within it
/// (`SessionStarted.participants`, then each `ParticipantJoined`), so a turn's audience is the set as
/// of the moment it was recorded — which is what lets the window filter mid-session joins correctly.
fn audience_turns(store: &dyn Store) -> Result<Vec<AudienceTurn>, StoreError> {
    let mut open: HashMap<ConversationId, (SessionId, Vec<MemoryId>)> = HashMap::new();
    let mut turns = Vec::new();
    for event in store.read_from(Seq::ZERO)? {
        match event.payload {
            EventPayload::SessionStarted {
                conversation,
                id,
                participants,
                ..
            } => {
                open.insert(conversation, (id, participants));
            }
            EventPayload::ParticipantJoined {
                conversation,
                session,
                participant,
                ..
            } => {
                if let Some((open_id, audience)) = open.get_mut(&conversation)
                    && *open_id == session
                    && !audience.contains(&participant)
                {
                    audience.push(participant);
                }
            }
            EventPayload::SessionEnded { conversation, id }
                if open
                    .get(&conversation)
                    .is_some_and(|(open_id, _)| *open_id == id) =>
            {
                open.remove(&conversation);
            }
            EventPayload::ConversationTurn {
                conversation,
                turn_id,
                role,
                text,
                participant,
                produced_by,
                ..
            } => {
                let (session, audience) = match open.get(&conversation) {
                    Some((id, audience)) => (Some(*id), audience.clone()),
                    None => (None, Vec::new()),
                };
                turns.push(AudienceTurn {
                    view: TurnView {
                        seq: event.seq,
                        turn_id,
                        role,
                        text,
                        participant,
                        recorded_at: event.recorded_at,
                        steps: Vec::new(),
                        produced_by,
                    },
                    conversation,
                    session,
                    audience,
                });
            }
            _ => {}
        }
    }
    Ok(turns)
}

/// The conversational display name for a resolved turn: the participant's handle for a participant
/// turn (falling back to `someone` when it is not in the graph, matching [`participant_names`]),
/// `self` for the agent's own turn, and `system` for an injected system turn.
fn turn_speaker(turn: &TurnView, names: &BTreeMap<MemoryId, String>) -> String {
    match turn.role {
        TurnRole::Participant => turn
            .participant
            .and_then(|id| names.get(&id))
            .cloned()
            .unwrap_or_else(|| "someone".to_owned()),
        TurnRole::Agent => MemoryName::SELF.to_owned(),
        TurnRole::System => "system".to_owned(),
    }
}

/// Read the live buffer ([`buffer_turns`]) and bound its carried tail, so the buffer cannot grow
/// without bound across compaction seams. `session_start_seq` is this session's own `SessionStarted`
/// seq; it splits the read into the carried tail (turns before it, seeded from a prior session across
/// a compaction seam) and this session's own turns (at or after it). The tail is re-trimmed to
/// `char_budget` — the same newest-first fill the carryover staging uses ([`carryover_start`]) — so a
/// session seeded from a carryover, and every session after it, sees a tail no larger than the budget
/// rather than every turn accrued since the original carryover point. The session's own turns always
/// ride whole (the token-budget compaction already bounds them), so the buffer is structurally
/// `≤ char_budget + one session's turns`, regardless of how the budgets are tuned. For a fresh session
/// `start_seq == session_start_seq`, the tail is empty and this is exactly [`buffer_turns`].
pub fn bounded_buffer_turns(
    store: &dyn Store,
    conversation: ConversationId,
    start_seq: Seq,
    session_start_seq: Seq,
    char_budget: i64,
) -> Result<Vec<TurnView>, StoreError> {
    let mut turns = buffer_turns(store, conversation, start_seq)?;
    // The read is in seq order, so the carried tail is the prefix below this session's own start.
    let split = turns.partition_point(|turn| turn.seq < session_start_seq);
    let keep_from = carryover_start(&turns[..split], char_budget);
    turns.drain(..keep_from);
    Ok(turns)
}

/// The index into `turns` of the oldest turn that fits `char_budget`, filling backward from the newest
/// — the raw-transcript carryover trim rule (spec §Compaction → raw-transcript carryover). The newest
/// turn is always kept (even if it alone exceeds the budget), then older turns while their running
/// character total fits. Returns `turns.len()` for an empty slice (an empty tail keeps nothing).
/// Shared by the read-time tail bound ([`bounded_buffer_turns`]) and the carryover staging, so both
/// trim by the same rule.
pub fn carryover_start(turns: &[TurnView], char_budget: i64) -> usize {
    let char_budget = char_budget.max(0) as usize;
    let mut total = 0usize;
    let mut start = turns.len();
    for (idx, turn) in turns.iter().enumerate().rev() {
        let next = total.saturating_add(turn.text.chars().count());
        if start != turns.len() && next > char_budget {
            break;
        }
        total = next;
        start = idx;
    }
    start
}

/// The distinct memory IDs the `conversation`'s blocks touched (read or wrote) from `from_seq`,
/// unioned across its `LuaExecuted` events in first-touch order — the touch-derived working set
/// carried across a compaction seam (spec §Compaction → working-set carryover). The read half is as
/// valuable as the write half: the agent looked something up because it was relevant.
pub fn session_touched(
    store: &dyn Store,
    conversation: ConversationId,
    from_seq: Seq,
) -> Result<Vec<MemoryId>, StoreError> {
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for event in store.read_from(from_seq)? {
        if let EventPayload::LuaExecuted {
            conversation: block_conversation,
            touched,
            ..
        } = event.payload
            && block_conversation == conversation
        {
            for id in touched {
                if seen.insert(id) {
                    ordered.push(id);
                }
            }
        }
    }
    Ok(ordered)
}

/// The session's flush watermark, derived from the log: the seq of the buffer's last flush turn — an
/// agent turn whose `produced_by` carries the `Flush` template, a checkpoint or a prior session's
/// end-flush riding the carried tail — or `session_start` when no flush turn is in view. Everything at
/// or before the watermark has been flushed to memory; the turns past it are the unflushed delta a
/// checkpoint flush scopes itself to (spec §Compaction → checkpoint flush). Derived per read rather
/// than held as mutable session state, so replaying the log reproduces it exactly.
pub fn flushed_up_to(buffer: &[TurnView], session_start: Seq) -> Seq {
    buffer
        .iter()
        .rev()
        .find(|turn| {
            turn.produced_by
                .as_ref()
                .is_some_and(|produced| produced.template_name == PromptTemplateName::Flush)
        })
        .map(|turn| turn.seq)
        .unwrap_or(session_start)
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
    /// How much of each model call to capture in the model-interaction record (spec §Observability).
    pub capture: CaptureLevel,
}

/// Run one turn: record the inbound participant message, then loop model steps until a terminal.
pub async fn run_turn(turn: Turn<'_>) -> Result<TurnReport, TurnError> {
    let Turn {
        session,
        model,
        engine,
        inbound,
        inbound_participant,
        brief,
        session_started_at,
        buffer,
        template,
        authority,
        present_set,
        max_steps,
        block_timeout,
        max_block_attempts,
        capture,
    } = turn;
    let conversation = session.conversation();
    // Content the agent writes this turn is attributed to the speaker by default (an append opts out
    // with `by_agent` for the agent's own observations — see `mem:append`).
    let teller = Teller::Participant(inbound_participant);
    // An inbound participant message is not inference, so it carries no provenance.
    append_turn(
        engine.store.lock().as_mut(),
        engine.clock.as_ref(),
        TurnRecord {
            conversation,
            turn_id: TurnId::generate(),
            role: TurnRole::Participant,
            text: inbound.to_owned(),
            participant: Some(inbound_participant),
            initiation: Initiation::Responding,
            produced_by: None,
        },
    )?;

    // Assemble the frozen system prompt once for the cycle: the `template` framing (Scaffold for a
    // participant turn, Imprint for the interview), the agent's identity from `self`, and the time.
    let framing = templates::latest_template(engine.store.lock().as_ref(), template)?;
    let framing_version = framing.as_ref().map(|t| t.version);
    let framing_body = framing.map(|t| t.body).unwrap_or_default();
    let (identity, vocabulary) = {
        let graph = engine.graph.lock();
        let identity = match graph.self_memory()? {
            Some(self_memory) => graph.entries_local(self_memory.id)?,
            None => Vec::new(),
        };
        let vocabulary =
            system_prompt::render_vocabulary(&graph.all_tags()?, &graph.all_relations()?);
        (identity, vocabulary)
    };
    // The API description is build-derived: rendered from the running binary so the prompt and the
    // installed Lua API can't drift (spec §System prompt → API description), plus the connected MCP
    // servers' projected tools (runtime-derived from the session's catalogue). The tag vocabulary is
    // runtime data, read from the graph above and rendered alongside the API description.
    let api_reference = full_api_reference(session);
    // The time is frozen to the session's start, not the live clock: every turn in the session then
    // sends an identical system prefix (current time rides in the per-message stamps below), so the
    // serving layer can reuse its prefix cache across the session rather than re-encoding on each turn.
    let system = system_prompt::assemble(
        &framing_body,
        &identity,
        &api_reference,
        &vocabulary,
        brief,
        session_started_at,
    );

    // Provenance for the agent's turn: the chat model and the template it ran against. If the
    // template isn't registered (it always is post-genesis), the attribution is simply absent.
    let agent_provenance = framing_version.map(|version| ProducedBy {
        model_id: model.model_id().into(),
        template_name: template,
        template_version: version,
    });

    // The agent's whole response cycle shares one turn id; its blocks stamp their events with it. The
    // live buffer is replayed as the prompt suffix, then the current inbound message.
    let turn_id = TurnId::generate();
    let names = participant_names(engine.as_ref(), buffer, &[inbound_participant]);
    let mut messages = buffer_messages(buffer, &names);
    messages.push(Message::user(stamp(
        inbound,
        engine.clock.now(),
        names.get(&inbound_participant).map(String::as_str),
    )));

    let steps_result = run_steps(Steps {
        session,
        model,
        engine: engine.clone(),
        system: &system,
        context: BlockContext {
            teller,
            authority,
            turn_id,
            block_timeout,
            max_block_attempts,
            present_set: present_set.to_vec(),
            dry_run: false,
        },
        messages,
        initiation: Initiation::Responding,
        provenance: agent_provenance,
        max_steps,
        capture,
    })
    .await;
    let (outcome, peak_prompt_tokens, steps, blocks) = match steps_result {
        Ok(resolved) => resolved,
        // The model backend is unreachable (retries, if any, exhausted by the wrapper, or the
        // circuit open): defer the turn instead of erroring it. The inbound participant turn was
        // appended above, before the loop, so nothing durable is lost — and deliberately no agent
        // turn is recorded (the harness's retries are infra-transparent, spec §Event sourcing:
        // they emit nothing to the log). The report's `turn_id` therefore keys no events, and the
        // step/block counts read zero even if the loop ran partial steps before the outage —
        // those blocks' events are in the log under this turn id, but with no agent turn to
        // anchor them the buffer replay carries only the inbound. Lua/store/graph failures keep
        // the error path: `Deferred` is only for model-transport failure.
        Err(TurnError::Model(error)) if error.is_unavailable() => {
            tracing::warn!(%error, "the model backend is unreachable; deferring the turn");
            observe_turn_deferred();
            (TurnOutcome::Deferred, None, 0, 0)
        }
        Err(error) => return Err(error),
    };

    // Description regeneration and temporal extraction for the memories this turn wrote run off the hot
    // path, in the background describer (spec §Write path → regenerate off the hot path, as a
    // catch-up), so the reply is not held waiting on summarization. The entries are committed and
    // readable now; only the synthesized description lags until the next catch-up.

    Ok(TurnReport {
        outcome,
        prompt_tokens: peak_prompt_tokens,
        steps,
        blocks,
        turn_id,
    })
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
    /// How much of each model call to capture in the model-interaction record (spec §Observability).
    pub capture: CaptureLevel,
}

/// Run the budget-gated pre-compaction flush: one agent turn whose job is to write durable working
/// state to memory before the session is cut (spec §Compaction). It reuses the session's scaffold
/// system prompt and appends the `Flush` template's instruction as a trailing system message, so the
/// cached system-plus-buffer prefix is preserved rather than re-encoded. It sees the full session
/// buffer, acts unprompted (`Initiation::Initiated`), and attributes its writes to the agent. An
/// ordinary `ConversationTurn` + `LuaExecuted`, fully logged and replay-trivial. A no-op if no `Flush`
/// template is registered (an agent born before the template shipped).
pub(crate) async fn run_flush(flush: Flush<'_>) -> Result<(), TurnError> {
    let Flush {
        session,
        model,
        engine,
        brief,
        session_started_at,
        buffer,
        present_set,
        max_steps,
        block_timeout,
        max_block_attempts,
        capture,
    } = flush;
    // The flush's standing instruction comes from the `Flush` template; without it there is nothing to
    // flush. It rides as a trailing message (below), not as the system prompt.
    let Some(flush_instruction) =
        templates::latest_template(engine.store.lock().as_ref(), PromptTemplateName::Flush)?
    else {
        return Ok(());
    };
    // Frame the flush with the SAME scaffold system prompt the session's live turns used, so the
    // identical system-plus-buffer prefix is already in the serving layer's cache. Swapping in a
    // distinct flush system prompt would change token zero and force a full re-encode of the whole
    // buffer at max context — the worst-case latency on the hot path.
    let scaffold =
        templates::latest_template(engine.store.lock().as_ref(), PromptTemplateName::Scaffold)?
            .map(|template| template.body)
            .unwrap_or_default();

    let (identity, vocabulary) = {
        let graph = engine.graph.lock();
        let identity = match graph.self_memory()? {
            Some(self_memory) => graph.entries_local(self_memory.id)?,
            None => Vec::new(),
        };
        let vocabulary =
            system_prompt::render_vocabulary(&graph.all_tags()?, &graph.all_relations()?);
        (identity, vocabulary)
    };
    let api_reference = full_api_reference(session);
    let system = system_prompt::assemble(
        &scaffold,
        &identity,
        &api_reference,
        &vocabulary,
        brief,
        session_started_at,
    );
    // The turn is still a flush for provenance — the `Flush` instruction drove it — even though the
    // scaffold now frames the system prompt.
    let provenance = Some(ProducedBy {
        model_id: model.model_id().into(),
        template_name: PromptTemplateName::Flush,
        template_version: flush_instruction.version,
    });

    let turn_id = TurnId::generate();
    // The buffer is the flush's whole context; the flush instruction is appended as a trailing
    // system-role message — a stronger reframing than a user turn, while leaving the cached prefix
    // intact. (If a serving backend rejects a non-leading system message, switch this to
    // `Message::user`.)
    let mut messages = buffer_messages(buffer, &participant_names(engine.as_ref(), buffer, &[]));
    messages.push(Message::system(flush_instruction.body));

    run_steps(Steps {
        session,
        model,
        engine: engine.clone(),
        system: &system,
        // The flush's writes are the agent's own synthesis, not attributed to any participant. It
        // runs under platform authority — the flush of a platform conversation must not write `self`.
        context: BlockContext {
            teller: Teller::Agent,
            authority: Authority::Platform,
            turn_id,
            block_timeout,
            max_block_attempts,
            present_set: present_set.to_vec(),
            dry_run: false,
        },
        messages,
        initiation: Initiation::Initiated,
        provenance,
        max_steps,
        capture,
    })
    .await?;

    // As with an ordinary turn, the flush's writes are regenerated off the hot path by the background
    // describer (spec §Write path) — the flush stays cheap, and the post-compaction brief forces the
    // catch-up for the working set before it composes (spec §Starvation bound).
    Ok(())
}

/// Replay the live buffer as chat messages: prior turns mapped to their roles (participant→user,
/// agent→assistant, system→system), skipping empty agent turns (silent terminals). The frozen brief
/// stays in the system prefix only — the buffer never perturbs it (prefix-cache stability). The
/// messages the agent *reads* — participant and system turns — are prefixed with the time they were
/// recorded; its own turns are left unstamped so it never learns to emit timestamps (spec §Time).
fn buffer_messages(buffer: &[TurnView], names: &BTreeMap<MemoryId, String>) -> Vec<Message> {
    let mut messages: Vec<Message> = Vec::with_capacity(buffer.len() + 1);
    for buffered in buffer {
        match buffered.role {
            TurnRole::Participant => {
                // Label the turn with who spoke, so a group room is not flattened into an anonymous
                // `user` stream the model cannot attribute (see `participant_names`).
                let speaker = buffered
                    .participant
                    .and_then(|id| names.get(&id))
                    .map(String::as_str);
                messages.push(Message::user(stamp(
                    &buffered.text,
                    buffered.recorded_at,
                    speaker,
                )))
            }
            TurnRole::Agent => {
                // Re-play the turn's tool-call steps so the model re-sees what it already ran —
                // the scripts, the results, the fetched pages and search hits — and does not
                // re-issue them. Each step is an assistant tool-call message followed by its
                // tool-result, matching the within-turn message order the model produced.
                for (i, step) in buffered.steps.iter().enumerate() {
                    let call_id = format!("call_{}_{}", buffered.seq.0, i);
                    messages.push(Message::assistant_tool_calls(vec![ToolCall {
                        id: call_id.clone(),
                        name: "run_lua".to_owned(),
                        arguments: serde_json::json!({ "script": step.script }).to_string(),
                    }]));
                    messages.push(Message::tool_result(call_id, step.result.clone()));
                }
                if !buffered.text.is_empty() {
                    messages.push(Message::assistant(buffered.text.clone()));
                }
            }
            TurnRole::System => messages.push(Message::system(stamp(
                &buffered.text,
                buffered.recorded_at,
                None,
            ))),
        }
    }
    messages
}

/// The display name (memory handle, e.g. `person/erin`) of every participant in `buffer` and any
/// `extra` ids, resolved against the graph. Without these, every participant turn renders as an
/// anonymous `user` message, so in a multi-party room the model cannot tell who said what — it reads
/// two speakers as one interlocutor and attributes one's words to the other (the source of the
/// fixture-18 leak). The handle matches `teller_display`, so a brief's "told by person/erin" and a
/// buffer turn's "person/erin:" name the same person.
fn participant_names(
    engine: &Engine,
    buffer: &[TurnView],
    extra: &[MemoryId],
) -> BTreeMap<MemoryId, String> {
    let graph = engine.graph.lock();
    let mut names = BTreeMap::new();
    for id in buffer
        .iter()
        .filter_map(|turn| turn.participant)
        .chain(extra.iter().copied())
    {
        names.entry(id).or_insert_with(|| {
            graph
                .memory_by_id(id)
                .ok()
                .flatten()
                .map(|memory| speaker_display(memory.name.as_str()))
                .unwrap_or_else(|| "someone".to_owned())
        });
    }
    names
}

/// A participant's conversational display name: the [`Namespace::Person`] prefix and any `@platform`
/// stub suffix stripped, so a turn reads `dave:`, not `person/dave@discord:`. The platform suffix is
/// operational noise irrelevant to who is speaking.
fn speaker_display(memory_name: &str) -> String {
    let handle = memory_name
        .strip_prefix(Namespace::Person.prefix())
        .unwrap_or(memory_name);
    handle.split('@').next().unwrap_or(handle).to_owned()
}

/// Prefix a message the agent reads with the compact wall-clock time it was recorded (spec §Time →
/// "Now"), and — for a participant turn — who spoke, so the model can attribute statements in a
/// multi-party room.
fn stamp(text: &str, at: Timestamp, speaker: Option<&str>) -> String {
    match speaker {
        Some(name) => format!("[{}] {}: {}", time::format_stamp(at), name, text),
        None => format!("[{}] {}", time::format_stamp(at), text),
    }
}

/// The cohesive context every model call needs to write its model-interaction record (spec
/// §Observability): which `conversation` and `turn_id` the call belongs to, and how much to
/// `capture`. Threaded into the step loop and the synthesis pass so each `generate` is recorded
/// uniformly. [`Recording::generate`] is the single chokepoint that times a call and best-effort
/// appends a `ModelCalled`; telemetry never breaks a turn, so an append failure is logged, not
/// propagated.
#[derive(Clone, Copy)]
struct Recording {
    /// The conversation the recorded calls belong to, or `None` for off-conversation background work
    /// (the description catch-up). A `None` recording emits no `ModelCalled` telemetry — there is no
    /// conversation to attribute it to — but the work's own events still carry their `produced_by`.
    conversation: Option<ConversationId>,
    turn_id: TurnId,
    capture: CaptureLevel,
}

impl Recording {
    /// Run one model call, timing it and recording its interaction. The caller passes the
    /// delta-encoded `record` (the request side), since only it owns the per-phase buffer state.
    async fn generate(
        &self,
        engine: &Engine,
        model: &dyn ModelClient,
        request: &GenerateRequest,
        phase: ModelPhase,
        record: Option<RequestRecord>,
    ) -> Result<GenerateResponse, ModelError> {
        let started = Instant::now();
        let response = model.generate(request).await?;
        let duration = started.elapsed();
        // The metrics chokepoint (spec §Observability → metrics): every model call — a turn step, a
        // flush, or a background describe/adjudicate pass — observes its latency and token usage
        // here, so the `/control/metrics` saturation counters are complete. Independent of the
        // `ModelCalled` telemetry event (which is conversation-attributed and capture-gated).
        observe_model_call(duration, &response.usage);
        let duration_ms = duration.as_millis() as u64;
        // Off-conversation background work (`conversation` is `None`) records no interaction event:
        // there is no conversation to file it under, and its product carries its own provenance.
        if self.capture != CaptureLevel::Off
            && let Some(conversation) = self.conversation
        {
            let event = EventPayload::ModelCalled {
                conversation,
                turn_id: self.turn_id,
                phase,
                request_digest: request_digest(request),
                request: record,
                completion: response.completion.clone(),
                reasoning: response.reasoning.clone(),
                finish_reason: response.finish_reason.clone(),
                usage: response.usage,
                duration_ms,
            };
            let now = engine.clock.now();
            if let Err(error) = engine.store.lock().append(now, vec![event]) {
                tracing::warn!(%error, "could not record the model-interaction event; the turn continues");
            }
        }
        Ok(response)
    }

    /// The delta record for a call: a [`RequestRecord::Base`] for the first call of a phase
    /// (`prev_sent_len` is `None`), otherwise a [`RequestRecord::Continuation`] of the messages
    /// appended since the previous call. `None` unless capturing at [`CaptureLevel::Full`], so the
    /// growing buffer is cloned only when it will be stored.
    fn request_record(
        &self,
        request: &GenerateRequest,
        prev_sent_len: Option<usize>,
    ) -> Option<RequestRecord> {
        if self.capture != CaptureLevel::Full {
            return None;
        }
        Some(match prev_sent_len {
            None => RequestRecord::Base {
                system: request.system.clone(),
                messages: request.messages.clone(),
                tools: request.tools.clone(),
                tool_choice: request.tool_choice,
                thinking: request.thinking,
            },
            Some(sent) => RequestRecord::Continuation {
                appended_messages: request.messages[sent..].to_vec(),
            },
        })
    }
}

/// A `sha2::Sha256` digest (hex) over the full serialized request, recorded on every `ModelCalled`
/// so a prompt reconstructed from the deltas can be checked against the call actually sent.
fn request_digest(request: &GenerateRequest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(request).unwrap_or_default());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The shared step loop a participant turn and a pre-compaction flush both run: generate, execute
/// `run_lua` blocks, feed their results back, until a terminal or `max_steps`. Records exactly one
/// agent `ConversationTurn` (however it ends) carrying `initiation` and `provenance`, and returns the
/// outcome with the peak prompt-token count observed (the largest the buffer reached mid-loop, which
/// the compaction budget bounds).
struct Steps<'a> {
    session: &'a Session,
    model: &'a dyn ModelClient,
    engine: Arc<Engine>,
    system: &'a str,
    context: BlockContext,
    messages: Vec<Message>,
    initiation: Initiation,
    provenance: Option<ProducedBy>,
    max_steps: usize,
    capture: CaptureLevel,
}

async fn run_steps(
    steps: Steps<'_>,
) -> Result<(TurnOutcome, Option<u32>, usize, usize), TurnError> {
    let Steps {
        session,
        model,
        engine,
        system,
        context,
        mut messages,
        initiation,
        provenance,
        max_steps,
        capture,
    } = steps;
    let conversation = session.conversation();
    let recording = Recording {
        conversation: Some(conversation),
        turn_id: context.turn_id,
        capture,
    };
    let tools = vec![run_lua_tool()];

    let record_agent_turn =
        |store: &mut dyn Store, clock: &dyn Clock, text: String| -> Result<(), TurnError> {
            append_turn(
                store,
                clock,
                TurnRecord {
                    conversation,
                    turn_id: context.turn_id,
                    role: TurnRole::Agent,
                    text,
                    participant: None,
                    initiation,
                    produced_by: provenance.clone(),
                },
            )
        };

    let mut peak_prompt_tokens: Option<u32> = None;
    let mut steps = 0;
    let mut blocks = 0;
    // The message count sent in the prior step, so each step records only the messages appended
    // since (the buffer is append-only within the loop); `None` until the first call.
    let mut prev_sent_len: Option<usize> = None;
    let outcome = 'cycle: {
        for step_index in 0..max_steps {
            // Nearing-budget nudge: two steps from the bound, tell the model to wrap up — once, not
            // every remaining step. It rides the in-memory step frame as a trailing system message
            // (like the flush instruction), never recorded to the log; it persists into the following
            // step's frame, so it appears exactly once from here on.
            if max_steps >= 2 && step_index == max_steps - 2 {
                messages.push(Message::system(
                    "two steps remain in this turn — finish gathering and answer with what you have.",
                ));
            }
            // On the final step the tools are withdrawn (`ToolChoice::None`) so the model must answer
            // with what it has rather than spend its last step on another tool call. Its text becomes
            // the turn's reply through the normal terminal path; `MaxStepsExceeded` is then only the
            // fallback for a model that still fails to produce text.
            let is_final_step = step_index + 1 == max_steps;
            let request = GenerateRequest {
                system: system.to_owned(),
                messages: messages.clone(),
                tools: tools.clone(),
                tool_choice: if is_final_step {
                    ToolChoice::None
                } else {
                    ToolChoice::Auto
                },
                response_format: None,
                thinking: None,
            };
            let record = recording.request_record(&request, prev_sent_len);
            prev_sent_len = Some(messages.len());
            let GenerateResponse {
                completion, usage, ..
            } = recording
                .generate(&engine, model, &request, ModelPhase::Step, record)
                .await?;
            steps += 1;
            peak_prompt_tokens = peak_prompt_tokens.max(usage.prompt_tokens);
            match completion {
                Completion::ToolCalls(calls) => {
                    messages.push(Message::assistant_tool_calls(calls.clone()));
                    for call in &calls {
                        let result = run_tool_call(session, &engine, &context, call).await?;
                        blocks += 1;
                        messages.push(Message::tool_result(call.id.clone(), result));
                    }
                }
                Completion::Reply(text) if reply_leaks_special_tokens(&text) => {
                    // The model emitted chat-template special-token markup as reply text (typically
                    // at the forced-answer final step, where `ToolChoice::None` forbids a real tool
                    // call and a weaker model transcribes a pseudo-tool-call instead). Such markup
                    // must never reach a participant, so resample the same request once — a transient
                    // decoding artifact usually differs on resample — and take the retry only if it
                    // comes back a clean reply; anything else falls to the silent terminal.
                    tracing::warn!(
                        malformed = %truncate_for_log(&text),
                        "the model leaked special-token markup in its reply; resampling once"
                    );
                    let retry_record = recording.request_record(&request, prev_sent_len);
                    let retry = recording
                        .generate(&engine, model, &request, ModelPhase::Step, retry_record)
                        .await?;
                    steps += 1;
                    peak_prompt_tokens = peak_prompt_tokens.max(retry.usage.prompt_tokens);
                    match retry.completion {
                        Completion::Reply(retry_text)
                            if !reply_leaks_special_tokens(&retry_text) =>
                        {
                            record_agent_turn(
                                engine.store.lock().as_mut(),
                                engine.clock.as_ref(),
                                retry_text.clone(),
                            )?;
                            break 'cycle TurnOutcome::Reply(retry_text);
                        }
                        _ => {
                            tracing::warn!(
                                malformed = %truncate_for_log(&text),
                                "the resampled reply is still malformed or not a plain reply; \
                                 staying silent rather than delivering markup"
                            );
                            record_agent_turn(
                                engine.store.lock().as_mut(),
                                engine.clock.as_ref(),
                                String::new(),
                            )?;
                            break 'cycle TurnOutcome::Silent;
                        }
                    }
                }
                Completion::Reply(text) => {
                    record_agent_turn(
                        engine.store.lock().as_mut(),
                        engine.clock.as_ref(),
                        text.clone(),
                    )?;
                    break 'cycle TurnOutcome::Reply(text);
                }
                Completion::Silent => {
                    record_agent_turn(
                        engine.store.lock().as_mut(),
                        engine.clock.as_ref(),
                        String::new(),
                    )?;
                    break 'cycle TurnOutcome::Silent;
                }
            }
        }
        let surfaced = format!("max steps ({max_steps}) reached without a reply");
        record_agent_turn(
            engine.store.lock().as_mut(),
            engine.clock.as_ref(),
            surfaced,
        )?;
        TurnOutcome::MaxStepsExceeded
    };

    Ok((outcome, peak_prompt_tokens, steps, blocks))
}

/// Whether a reply's text leaks model chat-template special-token markup — the `<|` or `|>`
/// delimiters that wrap a backend's special tokens (`<|tool_call|>`, `<|im_start|>`, and the like).
/// A well-formed reply is plain prose the participant reads; those delimiters only appear when the
/// model has transcribed template scaffolding into its answer, so their presence means the reply is
/// malformed and must not be delivered. The heuristic is deliberately exactly these two two-byte
/// delimiters: it does not parse tool-call shapes, and ordinary code or prose — Lua `..`, `{}`, or a
/// comparison like `a < b || b > c` — never contains `<|` or `|>`, so it does not false-positive.
fn reply_leaks_special_tokens(text: &str) -> bool {
    text.contains("<|") || text.contains("|>")
}

/// Clip `text` to a bounded, char-boundary-safe prefix for a log field, so a warn over a malformed
/// reply never dumps the whole (possibly large) completion into the diagnostic stream.
fn truncate_for_log(text: &str) -> String {
    const MAX_CHARS: usize = 200;
    let mut clipped: String = text.chars().take(MAX_CHARS).collect();
    if text.chars().nth(MAX_CHARS).is_some() {
        clipped.push('…');
    }
    clipped
}

/// The distinct memories that gained content (a create or an append) since `cycle_start`, in first-
/// write order. Coalescing here means a memory written several times in the turn regenerates once.
fn collect_written_memories(
    store: &dyn Store,
    cycle_start: Seq,
) -> Result<Vec<MemoryId>, TurnError> {
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for event in store.read_from(cycle_start.next())? {
        let id = match event.payload {
            EventPayload::MemoryCreated { id, .. }
            | EventPayload::MemoryContentAppended { id, .. }
            // A rename changes no content, but the description is synthesized under the memory's name,
            // so it must be re-synthesized under the new handle — otherwise it keeps the old name and
            // a renamed person's brief broadcasts a name they no longer go by (spec §Identity →
            // Renaming, deadname-safety).
            | EventPayload::MemoryRenamed { id, .. } => id,
            _ => continue,
        };
        if seen.insert(id) {
            ordered.push(id);
        }
    }
    Ok(ordered)
}

/// The system prompt's API-description block: the build-derived Lua API catalogue, plus the connected
/// MCP servers' projected tools (runtime-derived from the session's probed catalogue). Both render
/// through the same [`super::api_doc::render`] so the description is one consistent catalogue.
fn full_api_reference(session: &Session) -> String {
    let mut entries = lua::api_reference(&session.features());
    entries.extend(session.mcp_api_entries());
    super::api_doc::render(&entries)
}

/// Execute one tool call and render the text the model sees next: the block's result on success,
/// or a teachable failure (errors teach). Only infrastructure failures propagate as `TurnError`.
async fn run_tool_call(
    session: &Session,
    engine: &Arc<Engine>,
    context: &BlockContext,
    call: &ToolCall,
) -> Result<String, TurnError> {
    if call.name != "run_lua" {
        return Ok(ToolError::UnknownTool(call.name.clone()).to_string());
    }
    let script = match serde_json::from_str::<RunLuaArgs>(&call.arguments) {
        Ok(args) => args.script,
        Err(error) => return Ok(ToolError::InvalidArguments(error.to_string()).to_string()),
    };
    observe_lua_block();
    Ok(match session.execute(engine, context, &script).await? {
        BlockOutcome::Committed { result } => result,
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            observe_lua_block_error();
            ToolError::BlockError(message).to_string()
        }
        BlockOutcome::Terminated(TerminalCause::Aborted(reason)) => {
            observe_lua_block_error();
            ToolError::BlockAborted(reason).to_string()
        }
    })
}

/// A teachable failure surfaced back to the model as a tool result. Its `Display` is the single
/// place the wording of these messages lives, so the agent always sees consistent feedback.
enum ToolError {
    UnknownTool(String),
    InvalidArguments(String),
    BlockError(String),
    BlockAborted(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::UnknownTool(name) => write!(
                f,
                "error: no such tool {name:?}; the only available tool is run_lua"
            ),
            ToolError::InvalidArguments(message) => {
                write!(f, "error: invalid run_lua arguments: {message}")
            }
            ToolError::BlockError(message) => write!(f, "error: {message}"),
            ToolError::BlockAborted(reason) => {
                let reason = if reason.trim().is_empty() {
                    "(no reason given)"
                } else {
                    reason
                };
                write!(f, "aborted: {reason}")
            }
        }
    }
}

impl From<TerminalCause> for ToolError {
    fn from(cause: TerminalCause) -> Self {
        match cause {
            TerminalCause::Error(message) => ToolError::BlockError(message),
            TerminalCause::Aborted(reason) => ToolError::BlockAborted(reason),
        }
    }
}

/// The `run_lua` argument shape; doubles as the tool's parameter schema, so the schema sent to the
/// model and the parser can't drift.
#[derive(Deserialize, JsonSchema)]
struct RunLuaArgs {
    /// Luau source to execute.
    script: String,
}

fn run_lua_tool() -> ToolSpec {
    ToolSpec {
        name: "run_lua".to_owned(),
        description: "Execute a Luau block (Lua with string interpolation: `like {this}`) against \
                      your memory; returns the value of its final expression."
            .to_owned(),
        parameters: schema_of::<RunLuaArgs>(),
    }
}

/// One `ConversationTurn` to record: the inbound participant message, the agent's response, or a
/// system message. Holds just the turn's fields; the seams it is written through — the store it is
/// appended to and the clock that stamps it — are passed to [`append_turn`] alongside it.
struct TurnRecord {
    conversation: ConversationId,
    turn_id: TurnId,
    role: TurnRole,
    text: String,
    /// The speaker of an inbound message; `None` for the agent's own and system turns.
    participant: Option<MemoryId>,
    /// Whether the turn responds to a message or is the agent acting unprompted (the pre-compaction
    /// flush is `Initiated`; ordinary participant and agent turns are `Responding`).
    initiation: Initiation,
    produced_by: Option<ProducedBy>,
}

fn append_turn(
    store: &mut dyn Store,
    clock: &dyn Clock,
    record: TurnRecord,
) -> Result<(), TurnError> {
    store.append(
        clock.now(),
        vec![EventPayload::ConversationTurn {
            conversation: record.conversation,
            turn_id: record.turn_id,
            role: record.role,
            text: record.text,
            participant: record.participant,
            initiation: record.initiation,
            produced_by: record.produced_by,
            // Only a mid-session join carries a structured brief; the turns this records — inbound,
            // agent reply, flush — do not.
            brief: None,
        }],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{TurnResolution, reply_leaks_special_tokens, resolve_turn};
    use crate::{
        clock::ManualClock,
        engine::Engine,
        event::{Cardinality, EventPayload, Initiation, LinkSource, TurnRole},
        graph::Graph,
        ids::{ConversationId, MemoryId, Namespace, SessionId, TurnId},
        store::{MemoryStore, Store},
        time::Timestamp,
        vocabulary::RelationName,
    };

    /// A single-participant discord session in which `maya@discord` records one turn — the group-room
    /// moment a later reference points back to. Optionally operator-merges `maya@direct` into the same
    /// `same_as` class, mirroring how the console confirms a cross-platform identity. Returns the
    /// booted engine, the direct stub's id (the requester in a solo DM), and the recorded turn's id.
    fn discord_moment(merge_direct: bool) -> (std::sync::Arc<Engine>, MemoryId, TurnId) {
        let conversation = ConversationId::generate();
        let session = SessionId::generate();
        let turn_id = TurnId::generate();
        let discord = MemoryId::generate();
        let direct = MemoryId::generate();

        let mut events = vec![
            EventPayload::LinkTypeRegistered {
                name: RelationName::SameAs,
                inverse: RelationName::SameAs,
                from_card: Cardinality::Many,
                to_card: Cardinality::Many,
                symmetric: true,
                reflexive: false,
                description: String::new(),
            },
            EventPayload::memory_created(discord, Namespace::Person.with_name("maya@discord")),
            EventPayload::memory_created(direct, Namespace::Person.with_name("maya@direct")),
            EventPayload::session_started(
                conversation,
                session,
                vec![discord],
                Timestamp::from_millis(1_000),
                None,
                "",
            ),
            EventPayload::conversation_turn(
                conversation,
                turn_id,
                TurnRole::Participant,
                "we're standardizing on Postgres",
                Some(discord),
                Initiation::Responding,
                None,
            ),
        ];
        if merge_direct {
            events.push(EventPayload::LinkCreated {
                from: direct,
                to: discord,
                relation: RelationName::SameAs,
                source: LinkSource::Operator,
                told_by: None,
            });
        }

        let mut store = MemoryStore::new();
        store.append(Timestamp::from_millis(1_000), events).unwrap();
        let mut graph = Graph::open_in_memory().unwrap();
        graph.materialize_from(&store).unwrap();
        let engine = Engine::new(
            Box::new(store),
            graph,
            Box::new(ManualClock::new(Timestamp::from_millis(2_000))),
        );
        (engine, direct, turn_id)
    }

    #[test]
    fn special_token_markup_is_flagged_and_ordinary_text_is_not() {
        // The observed leak: a pseudo-tool-call transcribed with `<|`/`|>` special-token delimiters.
        assert!(reply_leaks_special_tokens(
            "<|tool_call>call:run_lua{script:<|\"|>memory.search(\"decided\")<|\"|>}<tool_call|>"
        ));
        // A normal reply is plain prose — clean.
        assert!(!reply_leaks_special_tokens(
            "Noted — I'll remember that you're standardizing on Postgres."
        ));
        // A reply quoting Lua with `..` concatenation and `{}` table syntax — clean.
        assert!(!reply_leaks_special_tokens(
            "Run `local t = { a = 1 }; return t.a .. \"!\"` to see it."
        ));
        // A comparison with `<`, `>`, and `||` but no adjacent `<|`/`|>` — clean.
        assert!(!reply_leaks_special_tokens(
            "guard against a < b || b > c here"
        ));
        // The delimiter proper: `<|` (and by symmetry `|>`) is flagged. The `<|` operator does not
        // occur in prose, so flagging `x <| y` is acceptable.
        assert!(reply_leaks_special_tokens("x <| y"));
        assert!(reply_leaks_special_tokens("x |> y"));
    }

    #[test]
    fn a_merged_identity_resolves_a_turn_recorded_under_the_other_stub() {
        // maya's direct stub, operator-merged with her discord stub, is present in a solo DM. She
        // attended the discord room only under the discord stub, but the merge makes the two one
        // person, so the audience rule admits her and the moment resolves.
        let (engine, direct, turn_id) = discord_moment(true);
        let resolution = resolve_turn(&engine, &[direct], turn_id, 2, 2).unwrap();
        assert!(matches!(resolution, TurnResolution::Resolved(_)));
    }

    #[test]
    fn an_unmerged_direct_stub_is_refused_as_a_different_person() {
        // Without the merge, the direct stub is a distinct identity that was never in the room's
        // audience, so the same lookup refuses — the raw-id behavior the class rule falls back to.
        let (engine, direct, turn_id) = discord_moment(false);
        let resolution = resolve_turn(&engine, &[direct], turn_id, 2, 2).unwrap();
        assert!(matches!(resolution, TurnResolution::AudienceMismatch));
    }
}
