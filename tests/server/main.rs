//! Server tests via the in-process control client: creating an agent, inspecting it, idempotent
//! re-creation, and boot reconciling a fresh graph from a persisted log (spec §Clients, §Storage).

#[path = "../common/mod.rs"]
mod common;

use std::time::Duration;
use zuihitsu::{
    CheckpointTrigger, Completion, ConcurrencySettings, ContextEntry, ConversationLocator,
    Embedder, FakeEmbedder, GenerateRequest, GenerateResponse, GenerateStream, Graph,
    InMemoryVectorIndex, ManualClock, MemoryId, MemoryName, MemoryStore, ModelClient, ModelError,
    Namespace, ParticipantAttribute, PersonId, ScriptedModel, SeedSelf, Server, Store,
    TEST_PLATFORM, ToolCall, TurnOutcome, TurnRole, Usage, VectorIndex,
    event::{EventPayload, EventSource, PromptTemplateName},
    genesis::{GenesisStatus, Rollout},
    stream_response,
    time::{MILLIS_PER_DAY, MILLIS_PER_MINUTE, MILLIS_PER_SECOND},
};

use common::time::test_now;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

pub(crate) fn seed() -> SeedSelf {
    SeedSelf {
        agent_name: "Kestrel".to_owned(),
        persona: "A thoughtful, discreet companion with a long memory.".to_owned(),
        seed_entries: vec!["I keep what people tell me in confidence.".to_owned()],
    }
}

pub(crate) fn clock() -> Box<ManualClock> {
    Box::new(ManualClock::new(test_now()))
}
mod brief;
mod brief_advanced;
mod checkpoint;
mod checkpoint_advanced;
mod context;
mod control;
mod joins;
mod links;
mod participant;
mod routing;
mod streaming;
mod supersession;

pub(crate) fn born_agent() -> (Server, ManualClock) {
    let clock = ManualClock::new(test_now());
    let server = Server::new(
        Box::new(MemoryStore::new()),
        Graph::open_in_memory().unwrap(),
        Box::new(clock.clone()),
    );
    server.control().create_agent(&seed()).unwrap();
    (server, clock)
}

/// Advance the clock just past the configured idle gap, so the next message opens a fresh
/// session. Reads the live `idle_gap_seconds` rather than baking the default into a literal —
/// the default's move cannot silently make the advance stop crossing the gap.
pub(crate) fn advance_past_idle_gap(server: &Server, clock: &ManualClock) {
    let idle_gap_ms = server
        .control()
        .settings()
        .unwrap()
        .compaction
        .idle_gap_seconds
        * MILLIS_PER_SECOND;
    clock.advance_millis(idle_gap_ms + MILLIS_PER_SECOND);
}

pub(crate) fn run_lua_call(script: &str) -> Completion {
    Completion::ToolCalls(vec![ToolCall {
        id: "lua".to_owned(),
        name: "run_lua".to_owned(),
        arguments: serde_json::json!({ "script": script }).to_string(),
    }])
}

pub(crate) fn describe_call(description: &str) -> Completion {
    Completion::Reply(
        serde_json::json!({ "description": description, "occurrences": [] }).to_string(),
    )
}
pub(crate) use brief::DispatchingModel;
pub(crate) use checkpoint::{SUBSTANTIVE, tune_checkpoint};
