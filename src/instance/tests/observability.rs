//! The metrics helpers observe a conversational turn and its model calls (spec §Observability →
//! metrics). Driven with a scripted model so no GPU is needed; a thread-local recorder
//! (`set_default_local_recorder`) keeps each test isolated — no global recorder, no cross-test
//! pollution. The per-turn span's step/block counts are exercised by the `TurnReport` counting
//! test in `tests/agent.rs`; the span itself is surfaced by `init_tracing`'s `FmtSpan::CLOSE`.
use crate::{
    ConversationLocator, Instance, PersonId, TEST_PLATFORM,
    clock::ManualClock,
    metrics::{LATENCY_BUCKETS, describe},
    model::{Completion, ScriptedModel},
    time::Timestamp,
};

fn born_server() -> Instance {
    let server =
        Instance::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    server
        .control()
        .create_agent(&crate::SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "An assistant.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    server
}

#[tokio::test]
async fn a_turn_observes_its_metrics() {
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new()
        .set_buckets(LATENCY_BUCKETS)
        .unwrap()
        .build_recorder();
    let handle = recorder.handle();
    let _guard = metrics::set_default_local_recorder(&recorder);
    describe();
    let server = born_server();
    let model = ScriptedModel::new([Completion::Reply("Hi there.".to_owned())]);
    server
        .platform()
        .route_message(
            &model,
            &ConversationLocator::new(TEST_PLATFORM, "general"),
            &PersonId::new(TEST_PLATFORM, "dave"),
            "hello",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    server.control().refresh_gauges().unwrap();
    let text = handle.render();
    assert!(
        text.contains("zuihitsu_turns_total 1\n"),
        "one turn observed"
    );
    assert!(
        text.contains("zuihitsu_model_calls_total 1\n"),
        "the turn's step was observed at the chokepoint"
    );
    assert!(text.contains("zuihitsu_sessions_opened_total 1\n"));
    assert!(
        text.contains("zuihitsu_sessions_active 1\n"),
        "session stays open"
    );
    // The agent-state gauges were refreshed from the graph.
    assert!(text.contains("zuihitsu_memory_count"));
    // The pre-brief pass described the brief's memories (the participant stub and the room's
    // context) and the plain-reply turn wrote nothing, so the describer's backlog gauge reads zero.
    assert!(
        text.contains("zuihitsu_describer_stale_memories 0\n"),
        "nothing is stale after a plain-reply turn"
    );

    // A write to a memory outside any brief leaves it stale, and the refreshed gauge counts it.
    let orphan = crate::ids::MemoryId::generate();
    let now = server.engine.clock.now();
    server
        .engine
        .store
        .lock()
        .append(
            now,
            crate::event::EventSource::Agent,
            vec![crate::event::EventPayload::memory_created(
                orphan,
                crate::ids::MemoryName::new("topic/orphan"),
            )],
        )
        .unwrap();
    server
        .engine
        .graph
        .lock()
        .materialize_from(server.engine.store.lock().as_ref())
        .unwrap();
    server.control().refresh_gauges().unwrap();
    let text = handle.render();
    assert!(
        text.contains("zuihitsu_describer_stale_memories 1\n"),
        "an undescribed write surfaces in the backlog gauge"
    );
}

#[tokio::test]
async fn model_call_tokens_accumulate_from_usage() {
    // A scripted step that reports token usage feeds the cumulative token counters.
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new()
        .set_buckets(LATENCY_BUCKETS)
        .unwrap()
        .build_recorder();
    let handle = recorder.handle();
    let _guard = metrics::set_default_local_recorder(&recorder);
    describe();
    let server = born_server();
    let model = ScriptedModel::with_responses([crate::model::GenerateResponse {
        completion: Completion::Reply("Hi there.".to_owned()),
        usage: crate::model::Usage {
            prompt_tokens: Some(120),
            completion_tokens: Some(30),
            total_tokens: Some(150),
            ..crate::model::Usage::default()
        },
        reasoning: None,
        finish_reason: Some("stop".to_owned()),
    }]);
    server
        .platform()
        .route_message(
            &model,
            &ConversationLocator::new(TEST_PLATFORM, "general"),
            &PersonId::new(TEST_PLATFORM, "dave"),
            "hello",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    let text = handle.render();
    assert!(text.contains("zuihitsu_model_prompt_tokens_total 120\n"));
    assert!(text.contains("zuihitsu_model_completion_tokens_total 30\n"));
}
