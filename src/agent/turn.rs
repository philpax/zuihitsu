//! The agent loop: a turn is a loop of model steps (spec §Agent loop).
//!
//! Each step the model emits either `run_lua` tool calls or a terminal (a reply or a stay-silent),
//! never both. Tool calls execute as blocks (Stage 4a), their rendered results feed back into the
//! next step, and the loop continues until the model replies, stays silent, or hits `max_steps`.
//! Exactly one `role = agent` `ConversationTurn` is recorded per cycle, however it ends — a reply,
//! an empty silent terminal, or a surfaced `max_steps` error — so "the agent saw this and chose
//! its outcome" is always auditable. The inbound message is its own `role = participant` turn.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    clock::Clock,
    engine::Engine,
    event::{
        ArbitrationResolution, EventPayload, Initiation, ModelPhase, ProducedBy,
        PromptTemplateName, RequestRecord, Teller, TerminalCause, TurnRole, Visibility,
    },
    graph::{EntryView, GraphError, MemoryView},
    ids::{ConversationId, EntryId, MemoryId, MemoryName, Seq, TurnId},
    memory::memory_block::Authority,
    model::{
        Completion, GenerateRequest, GenerateResponse, Message, ModelClient, ModelError, ToolCall,
        ToolChoice, ToolSpec,
    },
    settings::CaptureLevel,
    store::{Store, StoreError},
    time::{self, CivilDate, Direction, Rrule, TemporalRef, Timestamp},
};

use super::{
    lua::{self, BlockOutcome, LuaError, Session},
    system_prompt, templates,
};

/// What a completed turn delivers to the platform client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnOutcome {
    /// A reply to post back.
    Reply(String),
    /// The stay-silent terminal — nothing to post.
    Silent,
    /// The step budget was exhausted without a terminal; recorded for the agent to reason about.
    MaxStepsExceeded,
}

/// What a completed turn reports to the platform: its conversational `outcome` and the peak
/// `prompt_tokens` observed across the turn's generation steps — the largest the buffer reached, and
/// what the next turn would build on. `None` when no step reported usage (the platform then falls
/// back to a deterministic estimate). The platform compares this against the compaction budget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnReport {
    pub outcome: TurnOutcome,
    pub prompt_tokens: Option<u32>,
}

/// One turn replayed into the live buffer — the conversational surface the next turn sees as the
/// prompt suffix. Carries only the durable turn text, never the within-turn `run_lua` exchange (the
/// agent does not re-see its own scratch reasoning, consistent with the durable record). `seq` and
/// `turn_id` let a compaction mark the carried tail (`seeded_from_turn` and the next buffer's start).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnView {
    pub seq: Seq,
    pub turn_id: TurnId,
    pub role: TurnRole,
    pub text: String,
    pub participant: Option<MemoryId>,
    /// When the turn was recorded — the time it is stamped with when replayed (spec §Time → "Now").
    pub recorded_at: Timestamp,
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
    for event in store.read_from(from_seq)? {
        if let EventPayload::ConversationTurn {
            conversation: turn_conversation,
            turn_id,
            role,
            text,
            participant,
            ..
        } = event.payload
            && turn_conversation == conversation
        {
            turns.push(TurnView {
                seq: event.seq,
                turn_id,
                role,
                text,
                participant,
                recorded_at: event.recorded_at,
            });
        }
    }
    Ok(turns)
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
}

/// Everything one turn needs: the conversation's `session`, the shared seams (`model` and the
/// `engine` backends), the `inbound` participant message and its `inbound_participant` (the
/// speaker's `person/*` stub, whose content the turn's writes are attributed to), and the step
/// budget.
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
    /// `Scaffold` for an ordinary participant turn, `Imprint` for the control-panel imprint interview.
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
    // Everything the cycle's blocks commit lands after this point, so it bounds the turn's writes.
    let cycle_start = engine.store.lock().head()?;

    // Assemble the frozen system prompt once for the cycle: the `template` framing (Scaffold for a
    // participant turn, Imprint for the interview), the agent's identity from `self`, and the time.
    let framing = templates::latest_template(engine.store.lock().as_ref(), template)?;
    let framing_version = framing.as_ref().map(|t| t.version);
    let framing_body = framing.map(|t| t.body).unwrap_or_default();
    let (identity, vocabulary) = {
        let graph = engine.graph.lock();
        let identity = match graph.memory_by_name(MemoryName::SELF)? {
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
    let mut messages = buffer_messages(buffer);
    messages.push(Message::user(stamp(inbound, engine.clock.now())));

    let (outcome, peak_prompt_tokens) = run_steps(Steps {
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
        },
        messages,
        initiation: Initiation::Responding,
        provenance: agent_provenance,
        max_steps,
        capture,
    })
    .await?;

    // Write path: coalesce the memories the turn wrote and regenerate each one's description from
    // its entries. This runs after the reply is recorded, so a regeneration hiccup never costs the
    // conversational outcome.
    let written = collect_written_memories(engine.store.lock().as_ref(), cycle_start)?;
    regenerate_descriptions(
        model,
        &engine,
        &written,
        cycle_start,
        Recording {
            conversation,
            turn_id,
            capture,
        },
    )
    .await?;

    Ok(TurnReport {
        outcome,
        prompt_tokens: peak_prompt_tokens,
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

/// Run the budget-gated pre-compaction flush: one agent turn, framed by the `Flush` template, whose
/// job is to write durable working state to memory before the session is cut (spec §Compaction). It
/// sees the full session buffer, acts unprompted (`Initiation::Initiated`), and attributes its writes
/// to the agent. An ordinary `ConversationTurn` + `LuaExecuted`, fully logged and replay-trivial. A
/// no-op if no `Flush` template is registered (an agent born before the template shipped).
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
    let Some(template) =
        templates::latest_template(engine.store.lock().as_ref(), PromptTemplateName::Flush)?
    else {
        return Ok(());
    };

    let (identity, vocabulary) = {
        let graph = engine.graph.lock();
        let identity = match graph.memory_by_name(MemoryName::SELF)? {
            Some(self_memory) => graph.entries_local(self_memory.id)?,
            None => Vec::new(),
        };
        let vocabulary =
            system_prompt::render_vocabulary(&graph.all_tags()?, &graph.all_relations()?);
        (identity, vocabulary)
    };
    let api_reference = full_api_reference(session);
    let system = system_prompt::assemble(
        &template.body,
        &identity,
        &api_reference,
        &vocabulary,
        brief,
        session_started_at,
    );
    let provenance = Some(ProducedBy {
        model_id: model.model_id().into(),
        template_name: PromptTemplateName::Flush,
        template_version: template.version,
    });

    let turn_id = TurnId::generate();
    let cycle_start = engine.store.lock().head()?;
    // The buffer is the flush's whole context; a final user nudge gives the model a turn to respond
    // to (the transcript may end on an assistant turn) and states the flush's standing instruction.
    let mut messages = buffer_messages(buffer);
    messages.push(Message::user(
        "The session is ending — record anything from it worth keeping that you have not already.",
    ));

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
        },
        messages,
        initiation: Initiation::Initiated,
        provenance,
        max_steps,
        capture,
    })
    .await?;

    let written = collect_written_memories(engine.store.lock().as_ref(), cycle_start)?;
    regenerate_descriptions(
        model,
        &engine,
        &written,
        cycle_start,
        Recording {
            conversation: session.conversation(),
            turn_id,
            capture,
        },
    )
    .await?;
    Ok(())
}

/// Replay the live buffer as chat messages: prior turns mapped to their roles (participant→user,
/// agent→assistant, system→system), skipping empty agent turns (silent terminals). The frozen brief
/// stays in the system prefix only — the buffer never perturbs it (prefix-cache stability). The
/// messages the agent *reads* — participant and system turns — are prefixed with the time they were
/// recorded; its own turns are left unstamped so it never learns to emit timestamps (spec §Time).
fn buffer_messages(buffer: &[TurnView]) -> Vec<Message> {
    let mut messages: Vec<Message> = Vec::with_capacity(buffer.len() + 1);
    for buffered in buffer {
        match buffered.role {
            TurnRole::Participant => {
                messages.push(Message::user(stamp(&buffered.text, buffered.recorded_at)))
            }
            TurnRole::Agent if buffered.text.is_empty() => {}
            TurnRole::Agent => messages.push(Message::assistant(buffered.text.clone())),
            TurnRole::System => {
                messages.push(Message::system(stamp(&buffered.text, buffered.recorded_at)))
            }
        }
    }
    messages
}

/// Prefix a message the agent reads with the compact wall-clock time it was recorded (spec §Time →
/// "Now").
fn stamp(text: &str, at: Timestamp) -> String {
    format!("[{}] {}", time::format_stamp(at), text)
}

/// The cohesive context every model call needs to write its model-interaction record (spec
/// §Observability): which `conversation` and `turn_id` the call belongs to, and how much to
/// `capture`. Threaded into the step loop and the synthesis pass so each `generate` is recorded
/// uniformly. [`Recording::generate`] is the single chokepoint that times a call and best-effort
/// appends a `ModelCalled`; telemetry never breaks a turn, so an append failure is logged, not
/// propagated.
#[derive(Clone, Copy)]
struct Recording {
    conversation: ConversationId,
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
        let duration_ms = started.elapsed().as_millis() as u64;
        if self.capture != CaptureLevel::Off {
            let event = EventPayload::ModelCalled {
                conversation: self.conversation,
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

async fn run_steps(steps: Steps<'_>) -> Result<(TurnOutcome, Option<u32>), TurnError> {
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
        conversation,
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
    // The message count sent in the prior step, so each step records only the messages appended
    // since (the buffer is append-only within the loop); `None` until the first call.
    let mut prev_sent_len: Option<usize> = None;
    let outcome = 'cycle: {
        for _ in 0..max_steps {
            let request = GenerateRequest {
                system: system.to_owned(),
                messages: messages.clone(),
                tools: tools.clone(),
                // The loop lets the model choose between calling run_lua and replying.
                tool_choice: ToolChoice::Auto,
                thinking: None,
            };
            let record = recording.request_record(&request, prev_sent_len);
            prev_sent_len = Some(messages.len());
            let GenerateResponse {
                completion, usage, ..
            } = recording
                .generate(&engine, model, &request, ModelPhase::Step, record)
                .await?;
            peak_prompt_tokens = peak_prompt_tokens.max(usage.prompt_tokens);
            match completion {
                Completion::ToolCalls(calls) => {
                    messages.push(Message::assistant_tool_calls(calls.clone()));
                    for call in &calls {
                        let result = run_tool_call(session, &engine, &context, call).await?;
                        messages.push(Message::tool_result(call.id.clone(), result));
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

    Ok((outcome, peak_prompt_tokens))
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
            | EventPayload::MemoryContentAppended { id, .. } => id,
            _ => continue,
        };
        if seen.insert(id) {
            ordered.push(id);
        }
    }
    Ok(ordered)
}

/// Regenerate each written memory's description from its entries and, in the same model call,
/// extract the occurrence time of any entry written this turn that the agent left untimed (spec §Time
/// → "in the same pass"). New descriptions and resolved occurrences commit in one batch. A memory
/// with no entries is skipped; a model failure on one memory is logged and leaves it unchanged rather
/// than failing the whole turn.
async fn regenerate_descriptions(
    model: &dyn ModelClient,
    engine: &Engine,
    written: &[MemoryId],
    cycle_start: Seq,
    recording: Recording,
) -> Result<(), TurnError> {
    let Some(description_template) = templates::latest_template(
        engine.store.lock().as_ref(),
        PromptTemplateName::DescriptionRegen,
    )?
    else {
        return Ok(());
    };
    // The extraction half is optional: without its template the pass degrades to description-only.
    let extraction_template = templates::latest_template(
        engine.store.lock().as_ref(),
        PromptTemplateName::TemporalExtraction,
    )?;
    let system = compose_synthesis_system(
        &description_template.body,
        extraction_template
            .as_ref()
            .map(|template| template.body.as_str()),
    );
    // Entries appended this turn that the agent left untimed, mapped to their owning memory — the
    // only entries extraction may resolve. An explicit `occurred_at` is never overridden, and settled
    // older entries are never re-touched.
    let eligible = collect_untimed_entries(engine.store.lock().as_ref(), cycle_start)?;
    let now = engine.clock.now();

    let mut events = Vec::new();
    let mut resolved = BTreeSet::new();
    let extraction_provenance = extraction_template.as_ref().map(|template| ProducedBy {
        model_id: model.model_id().into(),
        template_name: PromptTemplateName::TemporalExtraction,
        template_version: template.version,
    });
    for &id in written {
        // Read the memory and its whole same_as class with a transient lock, released before the
        // synthesis `.await` below — no graph guard is held across a suspension point.
        let (memory, entries) = {
            let graph = engine.graph.lock();
            let Some(memory) = graph.memory_by_id(id)? else {
                continue;
            };
            // Class-wide synthesis: a merged identity has one unified description, composed from the
            // whole same_as class rather than the single written stub (spec §Visibility).
            (memory, graph.class_entries(id)?)
        };
        if entries.is_empty() {
            continue;
        }

        // The description and arbitration are synthesized over the memory's PUBLIC entries only, so a
        // private aside never reaches the always-visible summary (spec §Write path → from Public
        // entries only). For an all-public memory this is the whole class, unchanged.
        let public_entries: Vec<EntryView> = entries
            .iter()
            .filter(|entry| entry.visibility == Visibility::Public)
            .cloned()
            .collect();
        if !public_entries.is_empty() {
            match synthesize(
                model,
                engine,
                recording,
                &system,
                &memory,
                &public_entries,
                now,
            )
            .await
            {
                Ok(Some(synthesis)) => {
                    if !synthesis.description.trim().is_empty() {
                        events.push(EventPayload::MemoryDescriptionRegenerated {
                            id,
                            new_text: synthesis.description.trim().to_owned(),
                            produced_by: Some(ProducedBy {
                                model_id: model.model_id().into(),
                                template_name: PromptTemplateName::DescriptionRegen,
                                template_version: description_template.version,
                            }),
                        });
                    }
                    if let Some(event) = arbitration_event(
                        id,
                        &memory,
                        synthesis.arbitration,
                        &public_entries,
                        model.model_id(),
                        description_template.version,
                    ) {
                        events.push(event);
                    }
                    if let Some(provenance) = &extraction_provenance {
                        resolve_occurrences(
                            synthesis.occurrences,
                            &public_entries,
                            &eligible,
                            &mut resolved,
                            provenance,
                            &memory,
                            &mut events,
                        );
                    }
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(
                    memory = %memory.name.as_str(),
                    %error,
                    "turn-end synthesis failed; keeping the prior description"
                ),
            }
        }

        // Private entries the agent left untimed this turn still need temporal extraction — a private
        // reminder must still become a wake-up — but must never enter the description. A focused
        // extract-only pass resolves their occurrences; its description and arbitration are discarded.
        if let Some(provenance) = &extraction_provenance {
            let private_untimed: Vec<EntryView> = entries
                .iter()
                .filter(|entry| {
                    entry.visibility != Visibility::Public && eligible.contains_key(&entry.entry_id)
                })
                .cloned()
                .collect();
            if !private_untimed.is_empty() {
                match synthesize(
                    model,
                    engine,
                    recording,
                    &system,
                    &memory,
                    &private_untimed,
                    now,
                )
                .await
                {
                    Ok(Some(synthesis)) => resolve_occurrences(
                        synthesis.occurrences,
                        &private_untimed,
                        &eligible,
                        &mut resolved,
                        provenance,
                        &memory,
                        &mut events,
                    ),
                    Ok(None) => {}
                    Err(error) => tracing::warn!(
                        memory = %memory.name.as_str(),
                        %error,
                        "private-entry extraction failed; leaving them untimed"
                    ),
                }
            }
        }
    }

    if !events.is_empty() {
        engine.store.lock().append(now, events)?;
        // Two guards at once: graph (written) before store (read), per the lock-ordering rule.
        let mut graph = engine.graph.lock();
        graph.materialize_from(engine.store.lock().as_ref())?;
    }
    Ok(())
}

/// Map a flagged conflict to a `BeliefArbitrated`, or `None` if it is malformed — fewer than two
/// distinct competing entries, or no reconciling statement (spec §Write path → arbitration). Statement
/// numbers are 1-based into `entries`, which are the Public entries the description synthesizes over,
/// so arbitration records a choice between conflicting *public* assertions.
fn arbitration_event(
    memory_id: MemoryId,
    memory: &MemoryView,
    arbitration: Option<ExtractedArbitration>,
    entries: &[EntryView],
    model_id: &str,
    template_version: u32,
) -> Option<EventPayload> {
    let arbitration = arbitration?;
    let to_entry_ids = |numbers: Vec<usize>| {
        let mut ids: Vec<EntryId> = Vec::new();
        for number in numbers {
            if let Some(entry) = number.checked_sub(1).and_then(|i| entries.get(i))
                && !ids.contains(&entry.entry_id)
            {
                ids.push(entry.entry_id);
            }
        }
        ids
    };
    let competing_entries = to_entry_ids(arbitration.competing);
    let credited = to_entry_ids(arbitration.credited);
    if competing_entries.len() < 2 || arbitration.statement.trim().is_empty() {
        tracing::debug!(memory = %memory.name.as_str(), "dropping a malformed arbitration");
        return None;
    }
    Some(EventPayload::BeliefArbitrated {
        memory: memory_id,
        competing_entries,
        resolution: ArbitrationResolution {
            credited,
            statement: arbitration.statement.trim().to_owned(),
        },
        produced_by: Some(ProducedBy {
            model_id: model_id.into(),
            template_name: PromptTemplateName::DescriptionRegen,
            template_version,
        }),
    })
}

/// Resolve the extracted `occurrences` for the entries `list` (1-based statement numbers), pushing an
/// `EntryTemporalResolved` for each new, untimed entry, once. Shared by the public synthesis pass and
/// the focused private-entry extraction pass, so each only resolves the entries it was shown.
fn resolve_occurrences(
    occurrences: Vec<ExtractedOccurrence>,
    list: &[EntryView],
    eligible: &BTreeMap<EntryId, MemoryId>,
    resolved: &mut BTreeSet<EntryId>,
    provenance: &ProducedBy,
    memory: &MemoryView,
    events: &mut Vec<EventPayload>,
) {
    for occurrence in occurrences {
        // The statement number is 1-based into the entries listed in the prompt.
        let Some(entry) = occurrence.entry.checked_sub(1).and_then(|i| list.get(i)) else {
            continue;
        };
        // Only a new, untimed entry; skip anything else the model keyed (an entry already timed,
        // explicitly set, or a class sibling not written this turn), and resolve each once.
        let Some(&entry_memory) = eligible.get(&entry.entry_id) else {
            continue;
        };
        if !resolved.insert(entry.entry_id) {
            continue;
        }
        let Some(occurred_at) = occurrence.occurred_at.into_temporal_ref() else {
            tracing::debug!(memory = %memory.name.as_str(), "dropping an unparseable extracted occurrence");
            continue;
        };
        events.push(EventPayload::EntryTemporalResolved {
            id: entry_memory,
            entry_id: entry.entry_id,
            occurred_at,
            produced_by: Some(provenance.clone()),
        });
    }
}

/// Entries appended since `cycle_start` that carry no `occurred_at`, mapped to their owning memory —
/// the entries the extraction pass is allowed to resolve. An entry the agent timed explicitly is
/// excluded, so extraction never overrides a deliberate occurrence.
fn collect_untimed_entries(
    store: &dyn Store,
    cycle_start: Seq,
) -> Result<BTreeMap<EntryId, MemoryId>, TurnError> {
    let mut untimed = BTreeMap::new();
    for event in store.read_from(cycle_start.next())? {
        if let EventPayload::MemoryContentAppended {
            id,
            entry_id,
            occurred_at: None,
            ..
        } = event.payload
        {
            untimed.insert(entry_id, id);
        }
    }
    Ok(untimed)
}

/// The synthesis call's system prompt: the description-regeneration instructions, plus the
/// temporal-extraction instructions when that template exists, joined for the single combined call
/// (spec §Time → same pass). Each half still stamps its own events' provenance.
fn compose_synthesis_system(description_body: &str, extraction_body: Option<&str>) -> String {
    match extraction_body {
        Some(extraction) => format!("{description_body}\n\n{extraction}"),
        None => description_body.to_owned(),
    }
}

/// Ask the model, in one forced `synthesize` call, to describe a memory from its entries and extract
/// the occurrence time of any time-bearing statement. The entries are numbered (1-based) so the
/// extracted occurrences key back to them, and the current time is stated so relative phrases ("last
/// Tuesday") resolve. `None` means no usable call came back, which the caller treats as "leave the
/// memory unchanged".
async fn synthesize(
    model: &dyn ModelClient,
    engine: &Engine,
    recording: Recording,
    system: &str,
    memory: &MemoryView,
    entries: &[EntryView],
    now: Timestamp,
) -> Result<Option<SynthesizeArgs>, ModelError> {
    let mut prompt = format!(
        "Memory: {}\nCurrent time: {}\n\nStatements:\n",
        memory.name.as_str(),
        time::format_datetime(now),
    );
    for (index, entry) in entries.iter().enumerate() {
        prompt.push_str(&format!("{}. {}\n", index + 1, entry.text));
    }

    // Force a single `synthesize` tool call so the description and occurrences come back as clean
    // arguments. Reasoning is forced off: a live probe showed extraction accuracy holds without it,
    // and it makes the forced call intermittently emit an empty message.
    let request = GenerateRequest {
        system: system.to_owned(),
        messages: vec![Message::user(prompt)],
        tools: vec![synthesize_tool()],
        tool_choice: ToolChoice::Required,
        thinking: Some(false),
    };
    // The model still occasionally returns no usable call; retry a few times before giving up (this
    // pass is off the hot path, so a couple of extra attempts is cheap).
    const ATTEMPTS: usize = 3;
    for attempt in 1..=ATTEMPTS {
        // An off-buffer structured call; its usage must not move the conversational compaction
        // trigger, so it is read and discarded here. Each attempt is its own `Base` (a fresh
        // single-message buffer), recorded under the synthesis phase.
        let record = recording.request_record(&request, None);
        let GenerateResponse { completion, .. } = recording
            .generate(engine, model, &request, ModelPhase::Synthesis, record)
            .await?;
        if let Completion::ToolCalls(calls) = completion
            && let Some(args) = synthesize_argument(&calls)
        {
            if attempt > 1 {
                tracing::debug!(memory = %memory.name.as_str(), attempt, "synthesis succeeded after a retry");
            }
            return Ok(Some(args));
        }
        tracing::debug!(
            memory = %memory.name.as_str(),
            attempt,
            "synthesis returned no usable call"
        );
    }
    tracing::warn!(
        memory = %memory.name.as_str(),
        attempts = ATTEMPTS,
        "synthesis gave up after retries; keeping the memory unchanged"
    );
    Ok(None)
}

/// The system prompt's API-description block: the build-derived Lua API catalogue, plus the connected
/// MCP servers' projected tools (runtime-derived from the session's probed catalogue). Both render
/// through the same [`super::api_doc::render`] so the description is one consistent catalogue.
fn full_api_reference(session: &Session) -> String {
    let mut entries = lua::api_reference();
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
    Ok(match session.execute(engine, context, &script).await? {
        BlockOutcome::Committed { result } => result,
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            ToolError::BlockError(message).to_string()
        }
        BlockOutcome::Terminated(TerminalCause::Aborted(reason)) => {
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
            ToolError::UnknownTool(name) => write!(f, "error: no such tool {name:?}"),
            ToolError::InvalidArguments(message) => {
                write!(f, "error: invalid run_lua arguments: {message}")
            }
            ToolError::BlockError(message) => write!(f, "error: {message}"),
            ToolError::BlockAborted(reason) => write!(f, "aborted: {reason}"),
        }
    }
}

/// The `run_lua` argument shape; doubles as the tool's parameter schema, so the schema sent to the
/// model and the parser can't drift.
#[derive(Deserialize, JsonSchema)]
struct RunLuaArgs {
    /// Lua source to execute.
    script: String,
}

/// The `synthesize` argument shape (turn-end description + temporal extraction); doubles as the
/// tool's parameter schema, so the schema sent to the model and the parser can't drift.
#[derive(Deserialize, JsonSchema)]
struct SynthesizeArgs {
    /// The memory's description as plain third-person prose — no preamble, headings, or notes.
    description: String,
    /// One entry per statement that refers to a real-world time; omit statements with no temporal
    /// reference.
    #[serde(default)]
    occurrences: Vec<ExtractedOccurrence>,
    /// Present only when two or more statements directly contradict each other; absent otherwise.
    #[serde(default)]
    arbitration: Option<ExtractedArbitration>,
}

/// One extracted occurrence: the statement it applies to (1-based, as numbered in the prompt) and
/// the time it refers to.
#[derive(Deserialize, JsonSchema)]
struct ExtractedOccurrence {
    entry: usize,
    occurred_at: ExtractedTime,
}

/// A conflict the synthesis found among the numbered statements (spec §Write path → arbitration):
/// which statements collide, which the model credits, and a one-line reconciling note. Statement
/// numbers are 1-based, the same numbering [`ExtractedOccurrence`] keys off.
#[derive(Deserialize, JsonSchema)]
struct ExtractedArbitration {
    competing: Vec<usize>,
    credited: Vec<usize>,
    statement: String,
}

/// The date-string occurrence shape the model produces — it cannot compute epoch milliseconds, so it
/// emits ISO dates (and occasionally datetimes), which [`ExtractedTime::into_temporal_ref`] maps to
/// the stored [`TemporalRef`]. Mirrors `TemporalRef`'s tags but with string dates.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ExtractedTime {
    Instant(String),
    Day(String),
    Range { start: String, end: String },
    Approx { center: String, fuzz_days: u32 },
    Recurring(String),
    BeforeAfter { dir: String, anchor: String },
}

impl ExtractedTime {
    /// Map the model's date strings to the stored [`TemporalRef`], or `None` if a date won't parse.
    /// A bare calendar day under `instant` becomes a `Day`: a live probe showed the model uses the
    /// two interchangeably.
    fn into_temporal_ref(self) -> Option<TemporalRef> {
        match self {
            ExtractedTime::Instant(text) => match civil_date(&text) {
                Some(day) => Some(TemporalRef::Day(day)),
                None => Some(TemporalRef::Instant(Timestamp::from_millis(
                    time::datetime_to_millis(&text)?,
                ))),
            },
            ExtractedTime::Day(text) => civil_date(&text).map(TemporalRef::Day),
            ExtractedTime::Range { start, end } => Some(TemporalRef::Range {
                start: Timestamp::from_millis(time::date_or_datetime_to_millis(&start)?),
                end: Timestamp::from_millis(time::date_or_datetime_to_millis(&end)?),
            }),
            ExtractedTime::Approx { center, fuzz_days } => Some(TemporalRef::Approx {
                center: Timestamp::from_millis(time::date_or_datetime_to_millis(&center)?),
                fuzz_days,
            }),
            ExtractedTime::Recurring(rule) => Some(TemporalRef::Recurring(Rrule(rule.into()))),
            ExtractedTime::BeforeAfter { dir, anchor } => {
                let dir = match dir.trim().to_ascii_lowercase().as_str() {
                    "before" => Direction::Before,
                    "after" => Direction::After,
                    _ => return None,
                };
                Some(TemporalRef::BeforeAfter {
                    dir,
                    anchor: MemoryName::new(anchor),
                })
            }
        }
    }
}

/// The model's date string as a validated `Day` civil date, or `None`. A bare `YYYY-MM-DD` under
/// `instant` becomes a `Day` (the model uses the two interchangeably).
fn civil_date(text: &str) -> Option<CivilDate> {
    let date = CivilDate(text.trim().into());
    date.midnight_millis().map(|_| date)
}

fn run_lua_tool() -> ToolSpec {
    ToolSpec {
        name: "run_lua".to_owned(),
        description: "Execute a Lua block against your memory; returns the value of its final \
                      expression."
            .to_owned(),
        parameters: schema_of::<RunLuaArgs>(),
    }
}

/// The single tool the turn-end synthesis call is forced to use (`ToolChoice::Required`), so the
/// description and occurrences come back as clean arguments rather than free-form prose.
fn synthesize_tool() -> ToolSpec {
    ToolSpec {
        name: "synthesize".to_owned(),
        description: "Record the memory's description, the occurrence time of any time-bearing \
                      statement, and any conflict between contradicting statements."
            .to_owned(),
        parameters: schema_of::<SynthesizeArgs>(),
    }
}

/// The JSON-Schema for a tool's argument struct, the single source of truth shared with the parser.
fn schema_of<T: JsonSchema>() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or_default()
}

/// Parse a forced `synthesize` tool call's arguments, or `None` if the model produced no usable call
/// (no `synthesize` call, unparseable arguments, or an empty description).
fn synthesize_argument(calls: &[ToolCall]) -> Option<SynthesizeArgs> {
    let call = calls.iter().find(|call| call.name == "synthesize")?;
    let args: SynthesizeArgs = serde_json::from_str(&call.arguments).ok()?;
    (!args.description.trim().is_empty()).then_some(args)
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
        }],
    )?;
    Ok(())
}

/// A failure running a turn.
#[derive(Debug)]
pub enum TurnError {
    Model(ModelError),
    Lua(LuaError),
    Store(StoreError),
    Graph(GraphError),
}

impl std::fmt::Display for TurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TurnError::Model(error) => write!(f, "turn (model): {error}"),
            TurnError::Lua(error) => write!(f, "turn (lua): {error}"),
            TurnError::Store(error) => write!(f, "turn (store): {error}"),
            TurnError::Graph(error) => write!(f, "turn (graph): {error}"),
        }
    }
}

impl std::error::Error for TurnError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TurnError::Model(error) => Some(error),
            TurnError::Lua(error) => Some(error),
            TurnError::Store(error) => Some(error),
            TurnError::Graph(error) => Some(error),
        }
    }
}

impl From<ModelError> for TurnError {
    fn from(error: ModelError) -> Self {
        TurnError::Model(error)
    }
}

impl From<LuaError> for TurnError {
    fn from(error: LuaError) -> Self {
        TurnError::Lua(error)
    }
}

impl From<StoreError> for TurnError {
    fn from(error: StoreError) -> Self {
        TurnError::Store(error)
    }
}

impl From<GraphError> for TurnError {
    fn from(error: GraphError) -> Self {
        TurnError::Graph(error)
    }
}

#[cfg(test)]
mod tests {
    use super::ExtractedTime;
    use crate::{
        ids::MemoryName,
        time::{self, CivilDate, Direction, TemporalRef, Timestamp},
    };

    fn ms(date: &str) -> i64 {
        time::civil_date_to_millis(date).unwrap()
    }

    #[test]
    fn instant_date_only_coerces_to_day() {
        // The model uses `instant` for bare days; a date-only value becomes a `Day`, not an `Instant`.
        assert_eq!(
            ExtractedTime::Instant("2026-06-03".to_owned()).into_temporal_ref(),
            Some(TemporalRef::Day(CivilDate("2026-06-03".into())))
        );
    }

    #[test]
    fn instant_with_a_time_stays_an_instant() {
        let at = time::datetime_to_millis("2026-06-02T09:30:00Z").unwrap();
        assert_eq!(
            ExtractedTime::Instant("2026-06-02T09:30:00Z".to_owned()).into_temporal_ref(),
            Some(TemporalRef::Instant(Timestamp::from_millis(at)))
        );
    }

    #[test]
    fn day_maps_through() {
        assert_eq!(
            ExtractedTime::Day("2026-06-03".to_owned()).into_temporal_ref(),
            Some(TemporalRef::Day(CivilDate("2026-06-03".into())))
        );
    }

    #[test]
    fn range_and_approx_convert_dates_to_millis() {
        assert_eq!(
            ExtractedTime::Range {
                start: "2019-01-01".to_owned(),
                end: "2019-12-31".to_owned(),
            }
            .into_temporal_ref(),
            Some(TemporalRef::Range {
                start: Timestamp::from_millis(ms("2019-01-01")),
                end: Timestamp::from_millis(ms("2019-12-31")),
            })
        );
        assert_eq!(
            ExtractedTime::Approx {
                center: "2024-06-07".to_owned(),
                fuzz_days: 60,
            }
            .into_temporal_ref(),
            Some(TemporalRef::Approx {
                center: Timestamp::from_millis(ms("2024-06-07")),
                fuzz_days: 60,
            })
        );
    }

    #[test]
    fn before_after_parses_direction_case_insensitively() {
        assert_eq!(
            ExtractedTime::BeforeAfter {
                dir: "After".to_owned(),
                anchor: "event/wedding".to_owned(),
            }
            .into_temporal_ref(),
            Some(TemporalRef::BeforeAfter {
                dir: Direction::After,
                anchor: MemoryName::new("event/wedding"),
            })
        );
        // An unrecognized direction drops the occurrence rather than guessing.
        assert_eq!(
            ExtractedTime::BeforeAfter {
                dir: "sideways".to_owned(),
                anchor: "x".to_owned(),
            }
            .into_temporal_ref(),
            None
        );
    }

    #[test]
    fn malformed_dates_drop() {
        // 2026 is not a leap year, so Feb 29 is impossible; a non-date instant has no datetime either.
        assert_eq!(
            ExtractedTime::Day("2026-02-29".to_owned()).into_temporal_ref(),
            None
        );
        assert_eq!(
            ExtractedTime::Instant("whenever".to_owned()).into_temporal_ref(),
            None
        );
        assert_eq!(
            ExtractedTime::Range {
                start: "nope".to_owned(),
                end: "2020-01-01".to_owned(),
            }
            .into_temporal_ref(),
            None
        );
    }
}
