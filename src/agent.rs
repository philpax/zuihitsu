//! The agent loop: a turn is a loop of model steps (spec §Agent loop).
//!
//! Each step the model emits either `run_lua` tool calls or a terminal (a reply or a stay-silent),
//! never both. Tool calls execute as blocks (Stage 4a), their rendered results feed back into the
//! next step, and the loop continues until the model replies, stays silent, or hits `max_steps`.
//! Exactly one `role = agent` `ConversationTurn` is recorded per cycle, however it ends — a reply,
//! an empty silent terminal, or a surfaced `max_steps` error — so "the agent saw this and chose
//! its outcome" is always auditable. The inbound message is its own `role = participant` turn.

use serde::Deserialize;

use crate::{
    clock::Clock,
    event::{EventPayload, Initiation, TerminalCause, TurnRole},
    graph::Graph,
    ids::{ConversationId, TurnId},
    lua::{BlockOutcome, LuaError, Session},
    model::{Completion, GenerateRequest, Message, ModelClient, ModelError, ToolCall, ToolSpec},
    store::{Store, StoreError},
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

/// Run one turn: record the inbound participant message, then loop model steps until a terminal.
pub async fn run_turn(
    session: &Session,
    model: &dyn ModelClient,
    store: &mut dyn Store,
    graph: &mut Graph,
    clock: &dyn Clock,
    inbound: &str,
    max_steps: usize,
) -> Result<TurnOutcome, TurnError> {
    let conversation = session.conversation();
    append_turn(
        store,
        clock,
        conversation,
        TurnId::generate(),
        TurnRole::Participant,
        inbound.to_owned(),
    )?;

    // The agent's whole response cycle shares one turn id; its blocks stamp their events with it.
    let turn_id = TurnId::generate();
    let tools = vec![run_lua_tool()];
    let mut messages = vec![Message::user(inbound)];

    for _ in 0..max_steps {
        let request = GenerateRequest {
            system: String::new(),
            messages: messages.clone(),
            tools: tools.clone(),
        };
        match model.generate(&request).await? {
            Completion::ToolCalls(calls) => {
                messages.push(Message::assistant_tool_calls(calls.clone()));
                for call in &calls {
                    let result = run_tool_call(session, store, graph, clock, turn_id, call)?;
                    messages.push(Message::tool_result(call.id.clone(), result));
                }
            }
            Completion::Reply(text) => {
                append_turn(
                    store,
                    clock,
                    conversation,
                    turn_id,
                    TurnRole::Agent,
                    text.clone(),
                )?;
                return Ok(TurnOutcome::Reply(text));
            }
            Completion::Silent => {
                append_turn(
                    store,
                    clock,
                    conversation,
                    turn_id,
                    TurnRole::Agent,
                    String::new(),
                )?;
                return Ok(TurnOutcome::Silent);
            }
        }
    }

    let surfaced = format!("max steps ({max_steps}) reached without a reply");
    append_turn(
        store,
        clock,
        conversation,
        turn_id,
        TurnRole::Agent,
        surfaced,
    )?;
    Ok(TurnOutcome::MaxStepsExceeded)
}

/// Execute one tool call and render the text the model sees next: the block's result on success,
/// or a teachable failure (errors teach). Only infrastructure failures propagate as `TurnError`.
fn run_tool_call(
    session: &Session,
    store: &mut dyn Store,
    graph: &mut Graph,
    clock: &dyn Clock,
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
        match session.execute(store, graph, clock, turn_id, &script)? {
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

/// The `run_lua` argument shape: `{ "script": "..." }`.
#[derive(Deserialize)]
struct RunLuaArgs {
    script: String,
}

fn run_lua_tool() -> ToolSpec {
    ToolSpec {
        name: "run_lua".to_owned(),
        description: "Execute a Lua block against your memory; returns the value of its final \
                      expression."
            .to_owned(),
    }
}

fn append_turn(
    store: &mut dyn Store,
    clock: &dyn Clock,
    conversation: ConversationId,
    turn_id: TurnId,
    role: TurnRole,
    text: String,
) -> Result<(), TurnError> {
    store.append(
        clock.now(),
        vec![EventPayload::ConversationTurn {
            conversation,
            turn_id,
            role,
            text,
            initiation: Initiation::Responding,
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
}

impl std::fmt::Display for TurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TurnError::Model(error) => write!(f, "turn: {error}"),
            TurnError::Lua(error) => write!(f, "turn: {error}"),
            TurnError::Store(error) => write!(f, "turn: {error}"),
        }
    }
}

impl std::error::Error for TurnError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TurnError::Model(error) => Some(error),
            TurnError::Lua(error) => Some(error),
            TurnError::Store(error) => Some(error),
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
