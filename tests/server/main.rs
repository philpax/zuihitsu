//! Server tests via the in-process control client: creating an agent, inspecting it, idempotent
//! re-creation, and boot reconciling a fresh graph from a persisted log (spec §Clients, §Storage).

#[path = "../common/mod.rs"]
mod common;

use std::time::Duration;
use zuihitsu::{
    Completion, ConcurrencySettings, ConversationLocator, Embedder, FakeEmbedder, GenerateRequest,
    GenerateResponse, GenerateStream, Graph, InMemoryVectorIndex, ManualClock, MemoryId,
    MemoryName, MemoryStore, ModelClient, ModelError, Namespace, ScriptedModel, SeedSelf, Server,
    SqliteStore, Store, ToolCall, TurnOutcome, TurnRole, Usage, VectorIndex,
    event::{EventPayload, MergeProposalSource, PromptTemplateName},
    genesis::{GenesisStatus, Rollout},
    stream_response,
    time::MILLIS_PER_DAY,
};

use common::time::TEST_NOW;

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
    Box::new(ManualClock::new(TEST_NOW))
}
mod brief;
mod brief_advanced;
mod checkpoint;
mod checkpoint_advanced;
mod control;
mod joins;
mod routing;
mod streaming;

pub(crate) fn born_agent() -> (Server, ManualClock) {
    let clock = ManualClock::new(TEST_NOW);
    let server = Server::new(
        Box::new(MemoryStore::new()),
        Graph::open_in_memory().unwrap(),
        Box::new(clock.clone()),
    );
    server.control().create_agent(&seed()).unwrap();
    (server, clock)
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
