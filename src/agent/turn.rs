//! The agent loop: a turn is a loop of model steps (spec §Agent loop).
//!
//! Each step the model emits either `run_lua` tool calls or a terminal (a reply or a stay-silent),
//! never both. Tool calls execute as blocks (Stage 4a), their rendered results feed back into the
//! next step, and the loop continues until the model replies, stays silent, or hits `max_steps`.
//! Exactly one `role = agent` `ConversationTurn` is recorded per cycle, however it ends — a reply,
//! an empty silent terminal, or a surfaced `max_steps` error — so "the agent saw this and chose
//! its outcome" is always auditable. The inbound message is its own `role = participant` turn.

use schemars::JsonSchema;
use serde::Deserialize;

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    clock::Clock,
    event::{
        EventPayload, Initiation, ProducedBy, PromptTemplateName, Teller, TerminalCause, TurnRole,
    },
    graph::{EntryView, Graph, GraphError, MemoryView},
    ids::{ConversationId, EntryId, MemoryId, MemoryName, Seq, Timestamp, TurnId},
    memory::memory_block::Authority,
    model::{
        Completion, GenerateRequest, GenerateResponse, Message, ModelClient, ModelError, ToolCall,
        ToolChoice, ToolSpec,
    },
    store::{Store, StoreError},
    time::{self, CivilDate, Direction, Rrule, TemporalRef},
};

use super::{
    lua::{self, BlockOutcome, LuaError, Session},
    system_prompt, templates,
};

/// What a completed turn delivers to the platform client.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// The mutable backends every layer of a turn threads as a unit: the append-only event log
/// (`store`), the graph projection it feeds (`graph`), and the clock that stamps writes (`clock`).
/// They always travel together, so they ride as one value rather than three parallel arguments —
/// the shared shape behind [`Turn`], [`Flush`], [`Steps`], and [`crate::agent::lua::Session::execute`].
pub struct Engine<'a> {
    pub store: &'a mut dyn Store,
    pub graph: &'a mut Graph,
    pub clock: &'a dyn Clock,
}

impl Engine<'_> {
    /// A shorter-lived view of the same backends, for handing to an inner call without surrendering
    /// the borrow — so the caller can keep using the engine after the call returns.
    pub fn reborrow(&mut self) -> Engine<'_> {
        Engine {
            store: &mut *self.store,
            graph: &mut *self.graph,
            clock: self.clock,
        }
    }
}

/// The write context one block — or a whole step loop — runs under: who its content is attributed
/// to (`teller`), the authority it writes with (gating `self` and the link source, see
/// [`Authority`]), and the turn id its events are stamped with.
#[derive(Clone)]
pub struct BlockContext {
    pub teller: Teller,
    pub authority: Authority,
    pub turn_id: TurnId,
}

/// Everything one turn needs: the conversation's `session`, the shared seams (`model` and the
/// `engine` backends), the `inbound` participant message and its `inbound_participant` (the
/// speaker's `person/*` stub, whose content the turn's writes are attributed to), and the step
/// budget.
pub struct Turn<'a> {
    pub session: &'a Session,
    pub model: &'a dyn ModelClient,
    pub engine: Engine<'a>,
    pub inbound: &'a str,
    pub inbound_participant: MemoryId,
    /// The session's frozen contextual brief, interpolated into the system prompt (captured on
    /// `SessionStarted`, so every turn in the session sees the same brief).
    pub brief: &'a str,
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
    pub max_steps: usize,
}

/// Run one turn: record the inbound participant message, then loop model steps until a terminal.
pub async fn run_turn(turn: Turn<'_>) -> Result<TurnReport, TurnError> {
    let Turn {
        session,
        model,
        mut engine,
        inbound,
        inbound_participant,
        brief,
        buffer,
        template,
        authority,
        max_steps,
    } = turn;
    let conversation = session.conversation();
    // Content the agent writes this turn is attributed to the speaker by default (an append opts out
    // with `by_agent` for the agent's own observations — see `mem:append`).
    let teller = Teller::Participant(inbound_participant);
    // An inbound participant message is not inference, so it carries no provenance.
    append_turn(
        engine.store,
        engine.clock,
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
    let cycle_start = engine.store.head()?;

    // Assemble the frozen system prompt once for the cycle: the `template` framing (Scaffold for a
    // participant turn, Imprint for the interview), the agent's identity from `self`, and the time.
    let framing = templates::latest_template(engine.store, template)?;
    let framing_version = framing.as_ref().map(|t| t.version);
    let framing_body = framing.map(|t| t.body).unwrap_or_default();
    let identity = match engine.graph.memory_by_name(MemoryName::SELF)? {
        Some(self_memory) => engine.graph.entries_local(self_memory.id)?,
        None => Vec::new(),
    };
    // The API description is build-derived: rendered from the running binary so the prompt and the
    // installed Lua API can't drift (spec §System prompt → API description).
    let api_reference = lua::render_api_reference();
    let system = system_prompt::assemble(
        &framing_body,
        &identity,
        &api_reference,
        brief,
        engine.clock.now(),
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
    messages.push(Message::user(inbound));

    let (outcome, peak_prompt_tokens) = run_steps(Steps {
        session,
        model,
        engine: engine.reborrow(),
        system: &system,
        context: BlockContext {
            teller,
            authority,
            turn_id,
        },
        messages,
        initiation: Initiation::Responding,
        provenance: agent_provenance,
        max_steps,
    })
    .await?;

    // Write path: coalesce the memories the turn wrote and regenerate each one's description from
    // its entries. This runs after the reply is recorded, so a regeneration hiccup never costs the
    // conversational outcome.
    let written = collect_written_memories(engine.store, cycle_start)?;
    regenerate_descriptions(model, &mut engine, &written, cycle_start).await?;

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
    pub engine: Engine<'a>,
    pub brief: &'a str,
    pub buffer: &'a [TurnView],
    pub max_steps: usize,
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
        mut engine,
        brief,
        buffer,
        max_steps,
    } = flush;
    let Some(template) = templates::latest_template(engine.store, PromptTemplateName::Flush)?
    else {
        return Ok(());
    };

    let identity = match engine.graph.memory_by_name(MemoryName::SELF)? {
        Some(self_memory) => engine.graph.entries_local(self_memory.id)?,
        None => Vec::new(),
    };
    let api_reference = lua::render_api_reference();
    let system = system_prompt::assemble(
        &template.body,
        &identity,
        &api_reference,
        brief,
        engine.clock.now(),
    );
    let provenance = Some(ProducedBy {
        model_id: model.model_id().into(),
        template_name: PromptTemplateName::Flush,
        template_version: template.version,
    });

    let turn_id = TurnId::generate();
    let cycle_start = engine.store.head()?;
    // The buffer is the flush's whole context; a final user nudge gives the model a turn to respond
    // to (the transcript may end on an assistant turn) and states the flush's standing instruction.
    let mut messages = buffer_messages(buffer);
    messages.push(Message::user(
        "The session is ending — record anything from it worth keeping that you have not already.",
    ));

    run_steps(Steps {
        session,
        model,
        engine: engine.reborrow(),
        system: &system,
        // The flush's writes are the agent's own synthesis, not attributed to any participant. It
        // runs under platform authority — the flush of a platform conversation must not write `self`.
        context: BlockContext {
            teller: Teller::Agent,
            authority: Authority::Platform,
            turn_id,
        },
        messages,
        initiation: Initiation::Initiated,
        provenance,
        max_steps,
    })
    .await?;

    let written = collect_written_memories(engine.store, cycle_start)?;
    regenerate_descriptions(model, &mut engine, &written, cycle_start).await?;
    Ok(())
}

/// Replay the live buffer as chat messages: prior turns mapped to their roles (participant→user,
/// agent→assistant, system→system), skipping empty agent turns (silent terminals). The frozen brief
/// stays in the system prefix only — the buffer never perturbs it (prefix-cache stability).
fn buffer_messages(buffer: &[TurnView]) -> Vec<Message> {
    let mut messages: Vec<Message> = Vec::with_capacity(buffer.len() + 1);
    for buffered in buffer {
        match buffered.role {
            TurnRole::Participant => messages.push(Message::user(buffered.text.clone())),
            TurnRole::Agent if buffered.text.is_empty() => {}
            TurnRole::Agent => messages.push(Message::assistant(buffered.text.clone())),
            TurnRole::System => messages.push(Message::system(buffered.text.clone())),
        }
    }
    messages
}

/// The shared step loop a participant turn and a pre-compaction flush both run: generate, execute
/// `run_lua` blocks, feed their results back, until a terminal or `max_steps`. Records exactly one
/// agent `ConversationTurn` (however it ends) carrying `initiation` and `provenance`, and returns the
/// outcome with the peak prompt-token count observed (the largest the buffer reached mid-loop, which
/// the compaction budget bounds).
struct Steps<'a> {
    session: &'a Session,
    model: &'a dyn ModelClient,
    engine: Engine<'a>,
    system: &'a str,
    context: BlockContext,
    messages: Vec<Message>,
    initiation: Initiation,
    provenance: Option<ProducedBy>,
    max_steps: usize,
}

async fn run_steps(steps: Steps<'_>) -> Result<(TurnOutcome, Option<u32>), TurnError> {
    let Steps {
        session,
        model,
        mut engine,
        system,
        context,
        mut messages,
        initiation,
        provenance,
        max_steps,
    } = steps;
    let conversation = session.conversation();
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
            let GenerateResponse { completion, usage } = model.generate(&request).await?;
            peak_prompt_tokens = peak_prompt_tokens.max(usage.prompt_tokens);
            match completion {
                Completion::ToolCalls(calls) => {
                    messages.push(Message::assistant_tool_calls(calls.clone()));
                    for call in &calls {
                        let result = run_tool_call(session, &mut engine, &context, call)?;
                        messages.push(Message::tool_result(call.id.clone(), result));
                    }
                }
                Completion::Reply(text) => {
                    record_agent_turn(engine.store, engine.clock, text.clone())?;
                    break 'cycle TurnOutcome::Reply(text);
                }
                Completion::Silent => {
                    record_agent_turn(engine.store, engine.clock, String::new())?;
                    break 'cycle TurnOutcome::Silent;
                }
            }
        }
        let surfaced = format!("max steps ({max_steps}) reached without a reply");
        record_agent_turn(engine.store, engine.clock, surfaced)?;
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
    engine: &mut Engine<'_>,
    written: &[MemoryId],
    cycle_start: Seq,
) -> Result<(), TurnError> {
    let Some(description_template) =
        templates::latest_template(engine.store, PromptTemplateName::DescriptionRegen)?
    else {
        return Ok(());
    };
    // The extraction half is optional: without its template the pass degrades to description-only.
    let extraction_template =
        templates::latest_template(engine.store, PromptTemplateName::TemporalExtraction)?;
    let system = compose_synthesis_system(
        &description_template.body,
        extraction_template
            .as_ref()
            .map(|template| template.body.as_str()),
    );
    // Entries appended this turn that the agent left untimed, mapped to their owning memory — the
    // only entries extraction may resolve. An explicit `occurred_at` is never overridden, and settled
    // older entries are never re-touched.
    let eligible = collect_untimed_entries(engine.store, cycle_start)?;
    let now = engine.clock.now();

    let mut events = Vec::new();
    let mut resolved = BTreeSet::new();
    for &id in written {
        let Some(memory) = engine.graph.memory_by_id(id)? else {
            continue;
        };
        // Class-wide synthesis: a merged identity has one unified description, composed from the
        // whole same_as class rather than the single written stub (spec §Visibility).
        let entries = engine.graph.class_entries(id)?;
        if entries.is_empty() {
            continue;
        }
        let synthesis = match synthesize(model, &system, &memory, &entries, now).await {
            Ok(Some(synthesis)) => synthesis,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(
                    memory = %memory.name.as_str(),
                    %error,
                    "turn-end synthesis failed; keeping the prior description"
                );
                continue;
            }
        };
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
        let Some(extraction_template) = &extraction_template else {
            continue;
        };
        for occurrence in synthesis.occurrences {
            // The statement number is 1-based into the entries listed in the prompt.
            let Some(entry) = occurrence.entry.checked_sub(1).and_then(|i| entries.get(i)) else {
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
                tracing::debug!(
                    memory = %memory.name.as_str(),
                    "dropping an unparseable extracted occurrence"
                );
                continue;
            };
            events.push(EventPayload::EntryTemporalResolved {
                id: entry_memory,
                entry_id: entry.entry_id,
                occurred_at,
                produced_by: Some(ProducedBy {
                    model_id: model.model_id().into(),
                    template_name: PromptTemplateName::TemporalExtraction,
                    template_version: extraction_template.version,
                }),
            });
        }
    }

    if !events.is_empty() {
        engine.store.append(now, events)?;
        engine.graph.materialize_from(engine.store)?;
    }
    Ok(())
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
        // trigger, so it is read and discarded here.
        let GenerateResponse { completion, .. } = model.generate(&request).await?;
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

/// Execute one tool call and render the text the model sees next: the block's result on success,
/// or a teachable failure (errors teach). Only infrastructure failures propagate as `TurnError`.
fn run_tool_call(
    session: &Session,
    engine: &mut Engine,
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
    Ok(match session.execute(engine, context, &script)? {
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
}

/// One extracted occurrence: the statement it applies to (1-based, as numbered in the prompt) and
/// the time it refers to.
#[derive(Deserialize, JsonSchema)]
struct ExtractedOccurrence {
    entry: usize,
    occurred_at: ExtractedTime,
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
        description: "Record the memory's description and the occurrence time of any time-bearing \
                      statement."
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
        ids::{MemoryName, Timestamp},
        time::{self, CivilDate, Direction, TemporalRef},
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
