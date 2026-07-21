//! Agent-loop tests: a scripted model drives the step loop through tool calls and terminals, and
//! the resulting turns and side effects land in the log (spec §Agent loop).

#[path = "../common/mod.rs"]
mod common;

use serde::Serialize;

use common::Harness;
use zuihitsu::{
    BlockOutcome, CaptureLevel, CivilDate, Completion, EntryId, EnvConfig, Event, EventPayload,
    InferredLink, InstanceFeatures, LinkInferenceArgs, Message, ModelPhase, Namespace,
    NewRelationSpec, OpenAiClient, PromptTemplateName, RequestRecord, ScriptedModel, SeedSelf, Seq,
    TemporalRef, TerminalCause, Timestamp, ToolCall, ToolChoice, TurnOutcome, TurnReport, TurnRole,
    Usage, buffer_turns, genesis, prompt::PromptSectionKind, run_turn, time::MILLIS_PER_DAY,
};

pub(crate) use temporal::{
    SynthesizeArbitration, SynthesizeOccurrence, SynthesizeReply, arbitrate_call,
    belief_arbitrations, day_noon, synthesize_call,
};

fn seed() -> SeedSelf {
    SeedSelf {
        agent_name: "Kestrel".to_owned(),
        persona: "A companion.".to_owned(),
        seed_entries: Vec::new(),
    }
}

fn run_lua_call(script: &str) -> Completion {
    Completion::ToolCalls(vec![ToolCall {
        id: "1".to_owned(),
        name: "run_lua".to_owned(),
        arguments: serde_json::json!({ "script": common::prepare_script(script) }).to_string(),
    }])
}

fn count_agent_turns(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|e| {
            matches!(
                &e.payload,
                EventPayload::ConversationTurn {
                    role: TurnRole::Agent,
                    ..
                }
            )
        })
        .count()
}

mod arbitration;
mod link_inference;
mod model_calls;
mod steps;
mod temporal;
mod turns;
