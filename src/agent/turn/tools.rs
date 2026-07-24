//! Tool call execution: `run_lua` dispatch, the `ToolError` teachable-failure type, and the tool spec.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    agent::{
        lua::{self, BlockOutcome, Session},
        turn::{BlockContext, TurnError},
    },
    engine::Engine,
    event::TerminalCause,
    metrics::{observe_lua_block, observe_lua_block_error},
    model::{ToolCall, ToolSpec, schema_of},
};

/// The system prompt's API-description block: the build-derived Lua API catalogue, plus the connected
/// MCP servers' projected tools (runtime-derived from the session's probed catalogue). Both render
/// through the same [`crate::agent::api_doc::render`] so the description is one consistent catalogue.
pub(crate) fn full_api_reference(session: &Session) -> String {
    let mut entries = lua::api_reference(&session.features());
    entries.extend(session.mcp_api_entries());
    crate::agent::api_doc::render(&entries)
}

/// The outcome of a single tool call: either continue the step loop with a rendered result, or skip
/// the rest of the turn silently (a `turn.skip()` inside the block).
pub(super) enum ToolCallResult {
    /// The block's rendered result, fed back to the model.
    Continue(String),
    /// A `turn.skip()` was called; the turn should end silently. The block's writes are already
    /// committed.
    SkipTurn,
}

/// Execute one tool call and render the text the model sees next: the block's result on success,
/// or a teachable failure (errors teach). Only infrastructure failures propagate as `TurnError`.
/// A `turn.skip()` inside the block returns `SkipTurn`, signalling the caller to end the turn
/// silently.
pub(super) async fn run_tool_call(
    session: &Session,
    engine: &Arc<Engine>,
    context: &BlockContext,
    call: &ToolCall,
) -> Result<ToolCallResult, TurnError> {
    if call.name != "run_lua" {
        return Ok(ToolCallResult::Continue(
            ToolError::UnknownTool(call.name.clone()).to_string(),
        ));
    }
    let script = match serde_json::from_str::<RunLuaArgs>(&call.arguments) {
        Ok(args) => args.script,
        Err(error) => {
            return Ok(ToolCallResult::Continue(
                ToolError::InvalidArguments(error.to_string()).to_string(),
            ));
        }
    };
    observe_lua_block();
    Ok(match session.execute(engine, context, &script).await? {
        BlockOutcome::Committed { result } => ToolCallResult::Continue(result),
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            observe_lua_block_error();
            ToolCallResult::Continue(ToolError::BlockError(message).to_string())
        }
        BlockOutcome::Terminated(TerminalCause::Aborted(reason)) => {
            observe_lua_block_error();
            ToolCallResult::Continue(ToolError::BlockAborted(reason).to_string())
        }
        BlockOutcome::Terminated(TerminalCause::Skipped(_)) | BlockOutcome::Skipped(_) => {
            ToolCallResult::SkipTurn
        }
    })
}

/// A teachable failure surfaced back to the model as a tool result. Its `Display` is the single
/// place the wording of these messages lives, so the agent always sees consistent feedback.
pub(crate) enum ToolError {
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
            // A skip is intercepted by `ToolCallResult::SkipTurn` before it can reach `ToolError`,
            // so this arm is unreachable in practice — but the impl must be exhaustive to compile.
            TerminalCause::Skipped(reason) => ToolError::BlockAborted(reason.unwrap_or_default()),
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

pub(super) fn run_lua_tool() -> ToolSpec {
    ToolSpec {
        name: "run_lua".to_owned(),
        description: "Execute a Luau block (Lua with string interpolation: `like {this}`) against \
                      your memory; returns the value of its final expression."
            .to_owned(),
        parameters: schema_of::<RunLuaArgs>(),
    }
}
