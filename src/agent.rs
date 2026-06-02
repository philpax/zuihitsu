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

use std::collections::BTreeSet;

use crate::{
    clock::Clock,
    event::{
        EventPayload, Initiation, ProducedBy, PromptTemplateName, Teller, TerminalCause, TurnRole,
    },
    graph::{EntryView, Graph, GraphError, MemoryView},
    ids::{ConversationId, MemoryId, Seq, TurnId},
    lua::{self, BlockOutcome, LuaError, Session},
    model::{
        Completion, GenerateRequest, Message, ModelClient, ModelError, ToolCall, ToolChoice,
        ToolSpec,
    },
    store::{Store, StoreError},
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

/// Everything one turn needs: the conversation's `session`, the shared seams (`model`, `store`,
/// `graph`, `clock`), the `inbound` participant message and its `inbound_participant` (the speaker's
/// `person/*` stub, whose content the turn's writes are attributed to), and the step budget.
pub struct Turn<'a> {
    pub session: &'a Session,
    pub model: &'a dyn ModelClient,
    pub store: &'a mut dyn Store,
    pub graph: &'a mut Graph,
    pub clock: &'a dyn Clock,
    pub inbound: &'a str,
    pub inbound_participant: MemoryId,
    /// The session's frozen contextual brief, interpolated into the system prompt (captured on
    /// `SessionStarted`, so every turn in the session sees the same brief).
    pub brief: &'a str,
    pub max_steps: usize,
}

/// Run one turn: record the inbound participant message, then loop model steps until a terminal.
pub async fn run_turn(turn: Turn<'_>) -> Result<TurnOutcome, TurnError> {
    let Turn {
        session,
        model,
        store,
        graph,
        clock,
        inbound,
        inbound_participant,
        brief,
        max_steps,
    } = turn;
    let conversation = session.conversation();
    // Content the agent writes this turn is attributed to the speaker by default (an append opts out
    // with `by_agent` for the agent's own observations — see `mem:append`).
    let teller = Teller::Participant(inbound_participant);
    // An inbound participant message is not inference, so it carries no provenance.
    append_turn(
        store,
        clock,
        TurnRecord {
            conversation,
            turn_id: TurnId::generate(),
            role: TurnRole::Participant,
            text: inbound.to_owned(),
            participant: Some(inbound_participant),
            produced_by: None,
        },
    )?;
    // Everything the cycle's blocks commit lands after this point, so it bounds the turn's writes.
    let cycle_start = store.head()?;

    // Assemble the frozen system prompt once for the cycle: the scaffold framing, the agent's
    // identity from `self`, and the declared time.
    let scaffold = templates::latest_template(store, PromptTemplateName::Scaffold)?;
    let scaffold_version = scaffold.as_ref().map(|template| template.version);
    let scaffold_body = scaffold.map(|template| template.body).unwrap_or_default();
    let identity = match graph.memory_by_name("self")? {
        Some(self_memory) => graph.entries_local(self_memory.id)?,
        None => Vec::new(),
    };
    // The API description is build-derived: rendered from the running binary so the prompt and the
    // installed Lua API can't drift (spec §System prompt → API description).
    let api_reference = lua::render_api_reference();
    let system = system_prompt::assemble(
        &scaffold_body,
        &identity,
        &api_reference,
        brief,
        clock.now(),
    );

    // Provenance for the agent's turn: the chat model and the scaffold it ran against. If no
    // scaffold is registered (it always is post-genesis), the attribution is simply absent.
    let agent_provenance = scaffold_version.map(|version| ProducedBy {
        model_id: model.model_id().into(),
        template_name: PromptTemplateName::Scaffold,
        template_version: version,
    });

    // The agent's whole response cycle shares one turn id; its blocks stamp their events with it.
    let turn_id = TurnId::generate();
    let tools = vec![run_lua_tool()];
    let mut messages = vec![Message::user(inbound)];

    let outcome = 'cycle: {
        for _ in 0..max_steps {
            let request = GenerateRequest {
                system: system.clone(),
                messages: messages.clone(),
                tools: tools.clone(),
                // The loop lets the model choose between calling run_lua and replying.
                tool_choice: ToolChoice::Auto,
                thinking: None,
            };
            match model.generate(&request).await? {
                Completion::ToolCalls(calls) => {
                    messages.push(Message::assistant_tool_calls(calls.clone()));
                    for call in &calls {
                        let result =
                            run_tool_call(session, store, graph, clock, &teller, turn_id, call)?;
                        messages.push(Message::tool_result(call.id.clone(), result));
                    }
                }
                Completion::Reply(text) => {
                    append_turn(
                        store,
                        clock,
                        TurnRecord {
                            conversation,
                            turn_id,
                            role: TurnRole::Agent,
                            text: text.clone(),
                            participant: None,
                            produced_by: agent_provenance.clone(),
                        },
                    )?;
                    break 'cycle TurnOutcome::Reply(text);
                }
                Completion::Silent => {
                    append_turn(
                        store,
                        clock,
                        TurnRecord {
                            conversation,
                            turn_id,
                            role: TurnRole::Agent,
                            text: String::new(),
                            participant: None,
                            produced_by: agent_provenance.clone(),
                        },
                    )?;
                    break 'cycle TurnOutcome::Silent;
                }
            }
        }
        let surfaced = format!("max steps ({max_steps}) reached without a reply");
        append_turn(
            store,
            clock,
            TurnRecord {
                conversation,
                turn_id,
                role: TurnRole::Agent,
                text: surfaced,
                participant: None,
                produced_by: agent_provenance.clone(),
            },
        )?;
        TurnOutcome::MaxStepsExceeded
    };

    // Write path: coalesce the memories the turn wrote and regenerate each one's description from
    // its entries. This runs after the reply is recorded, so a regeneration hiccup never costs the
    // conversational outcome.
    let written = collect_written_memories(store, cycle_start)?;
    regenerate_descriptions(model, store, graph, clock, &written).await?;

    Ok(outcome)
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

/// Regenerate the description of each written memory from its entries, committing the new
/// descriptions in one batch. A memory with no entries is skipped; a model failure on one memory is
/// logged and that memory keeps its prior description rather than failing the whole turn.
async fn regenerate_descriptions(
    model: &dyn ModelClient,
    store: &mut dyn Store,
    graph: &mut Graph,
    clock: &dyn Clock,
    written: &[MemoryId],
) -> Result<(), TurnError> {
    let Some(template) = templates::latest_template(store, PromptTemplateName::DescriptionRegen)?
    else {
        return Ok(());
    };

    let mut events = Vec::new();
    for &id in written {
        let Some(memory) = graph.memory_by_id(id)? else {
            continue;
        };
        // Class-wide synthesis: a merged identity has one unified description, composed from the
        // whole same_as class rather than the single written stub (spec §Visibility).
        let entries = graph.class_entries(id)?;
        if entries.is_empty() {
            continue;
        }
        match synthesize_description(model, &template.body, &memory, &entries).await {
            Ok(Some(description)) => events.push(EventPayload::MemoryDescriptionRegenerated {
                id,
                new_text: description,
                produced_by: Some(ProducedBy {
                    model_id: model.model_id().into(),
                    template_name: PromptTemplateName::DescriptionRegen,
                    template_version: template.version,
                }),
            }),
            Ok(None) => {}
            Err(error) => tracing::warn!(
                memory = %memory.name.as_str(),
                %error,
                "description regeneration failed; keeping the prior description"
            ),
        }
    }

    if !events.is_empty() {
        store.append(clock.now(), events)?;
        graph.materialize_from(store)?;
    }
    Ok(())
}

/// Ask the model to synthesize a description from a memory's entries, forcing a `describe` tool call
/// so the answer is a clean argument. `None` means no usable call came back, which a regeneration
/// ignores (keeping the prior description).
async fn synthesize_description(
    model: &dyn ModelClient,
    template_body: &str,
    memory: &MemoryView,
    entries: &[EntryView],
) -> Result<Option<String>, ModelError> {
    let mut prompt = format!("Memory: {}\n\nEntries:\n", memory.name.as_str());
    for entry in entries {
        prompt.push_str("- ");
        prompt.push_str(&entry.text);
        prompt.push('\n');
    }

    // Force a single `describe` tool call so the description comes back as a clean argument, not
    // free-form prose the model wraps in preamble. Reasoning is forced off: it adds nothing to a
    // structured extraction and makes the forced call intermittently emit an empty message.
    let request = GenerateRequest {
        system: template_body.to_owned(),
        messages: vec![Message::user(prompt)],
        tools: vec![describe_tool()],
        tool_choice: ToolChoice::Required,
        thinking: Some(false),
    };
    // The model still occasionally returns no usable call; retry a few times before giving up
    // (regeneration is off the hot path, so a couple of extra attempts is cheap).
    const ATTEMPTS: usize = 3;
    for attempt in 1..=ATTEMPTS {
        if let Completion::ToolCalls(calls) = model.generate(&request).await?
            && let Some(description) = describe_argument(&calls)
        {
            if attempt > 1 {
                tracing::debug!(memory = %memory.name.as_str(), attempt, "description regenerated after a retry");
            }
            return Ok(Some(description));
        }
        tracing::debug!(
            memory = %memory.name.as_str(),
            attempt,
            "description regeneration returned no usable describe call"
        );
    }
    tracing::warn!(
        memory = %memory.name.as_str(),
        attempts = ATTEMPTS,
        "description regeneration gave up after retries; keeping the prior description"
    );
    Ok(None)
}

/// Execute one tool call and render the text the model sees next: the block's result on success,
/// or a teachable failure (errors teach). Only infrastructure failures propagate as `TurnError`.
fn run_tool_call(
    session: &Session,
    store: &mut dyn Store,
    graph: &mut Graph,
    clock: &dyn Clock,
    teller: &Teller,
    turn_id: TurnId,
    call: &ToolCall,
) -> Result<String, TurnError> {
    if call.name != "run_lua" {
        return Ok(ToolError::UnknownTool(call.name.clone()).to_string());
    }
    let script = match serde_json::from_str::<RunLuaArgs>(&call.arguments) {
        Ok(args) => args.script,
        Err(error) => return Ok(ToolError::InvalidArguments(error.to_string()).to_string()),
    };
    Ok(
        match session.execute(store, graph, clock, teller.clone(), turn_id, &script)? {
            BlockOutcome::Committed { result } => result,
            BlockOutcome::Terminated(TerminalCause::Error(message)) => {
                ToolError::BlockError(message).to_string()
            }
            BlockOutcome::Terminated(TerminalCause::Aborted(reason)) => {
                ToolError::BlockAborted(reason).to_string()
            }
        },
    )
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

/// The `describe` argument shape (description regeneration); doubles as the tool's parameter schema.
#[derive(Deserialize, JsonSchema)]
struct DescribeArgs {
    /// The memory's description as plain third-person prose — no preamble, headings, or notes.
    description: String,
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

/// The single tool the description-regeneration call is forced to use (`ToolChoice::Required`), so
/// the synthesized description comes back as a clean argument rather than free-form prose with
/// preamble — the failure mode the draft template produced against a real model.
fn describe_tool() -> ToolSpec {
    ToolSpec {
        name: "describe".to_owned(),
        description: "Record the synthesized description for the memory.".to_owned(),
        parameters: schema_of::<DescribeArgs>(),
    }
}

/// The JSON-Schema for a tool's argument struct, the single source of truth shared with the parser.
fn schema_of<T: JsonSchema>() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or_default()
}

/// Extract the `description` argument from a forced `describe` tool call, or `None` if the model
/// produced no usable call.
fn describe_argument(calls: &[ToolCall]) -> Option<String> {
    let call = calls.iter().find(|call| call.name == "describe")?;
    let args: DescribeArgs = serde_json::from_str(&call.arguments).ok()?;
    let description = args.description.trim();
    (!description.is_empty()).then(|| description.to_owned())
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
            initiation: Initiation::Responding,
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
