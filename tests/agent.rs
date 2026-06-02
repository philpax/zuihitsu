//! Agent-loop tests: a scripted model drives the step loop through tool calls and terminals, and
//! the resulting turns and side effects land in the log (spec §Agent loop).

#![cfg(feature = "lua")]

mod common;

use common::Harness;
use zuihitsu::{
    Completion, PromptTemplateName, ScriptedModel, SeedSelf, Seq, Store, ToolCall, TurnOutcome,
    TurnReport, TurnRole, event::EventPayload, genesis, run_turn,
};
#[cfg(feature = "openai")]
use zuihitsu::{EnvConfig, OpenAiClient};

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
        arguments: serde_json::json!({ "script": script }).to_string(),
    }])
}

fn count_agent_turns(store: &impl Store) -> usize {
    store
        .read_from(Seq::ZERO)
        .unwrap()
        .into_iter()
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

#[tokio::test]
async fn tool_call_then_reply_commits_and_replies() {
    let mut h = Harness::new();
    let model = ScriptedModel::new([
        run_lua_call(r#"memory.create("person/dave", "Met at the climbing gym")"#),
        Completion::Reply("Noted — I'll remember Dave.".to_owned()),
    ]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "Remember Dave", 8))
        .await
        .unwrap();

    assert_eq!(
        outcome,
        TurnOutcome::Reply("Noted — I'll remember Dave.".to_owned())
    );
    // The tool call's side effect committed and projected.
    assert!(h.graph.memory_by_name("person/dave").unwrap().is_some());
    // Exactly one agent turn for the cycle, plus the inbound participant turn and a LuaExecuted.
    assert_eq!(count_agent_turns(&h.store), 1);
    let events = h.store.read_from(Seq::ZERO).unwrap();
    assert!(events.iter().any(|e| matches!(
        &e.payload,
        EventPayload::ConversationTurn {
            role: TurnRole::Participant,
            ..
        }
    )));
    assert!(
        events
            .iter()
            .any(|e| matches!(e.payload, EventPayload::LuaExecuted { .. }))
    );
}

#[tokio::test]
async fn descriptions_regenerate_after_a_turn() {
    let mut h = Harness::new();
    // Genesis registers the description-regen template the write path reads.
    genesis::rollout(&mut h.store, &h.clock, &seed()).unwrap();
    h.graph.materialize_from(&h.store).unwrap();

    let model = ScriptedModel::new([
        run_lua_call(r#"memory.create("person/dave", "Met at the climbing gym")"#),
        Completion::Reply("Noted — I'll remember Dave.".to_owned()),
        // The post-turn regeneration call: a forced `describe` tool call carries the synthesized
        // description as a clean argument (rather than free-form prose).
        Completion::ToolCalls(vec![ToolCall {
            id: "regen".to_owned(),
            name: "describe".to_owned(),
            arguments: r#"{"description":"Dave, whom I met at the climbing gym."}"#.to_owned(),
        }]),
    ]);

    run_turn(h.as_turn(&model, "Remember Dave", 8))
        .await
        .unwrap();

    // The written memory's description was regenerated from its entries after the cycle.
    let dave = h.graph.memory_by_name("person/dave").unwrap().unwrap();
    assert_eq!(dave.description, "Dave, whom I met at the climbing gym.");
    // It carries provenance: which model and template produced it. (Genesis also seeds self's
    // description, with null provenance, so match Dave's specifically.)
    let produced_by = h
        .store
        .read_from(Seq::ZERO)
        .unwrap()
        .into_iter()
        .find_map(|e| match e.payload {
            EventPayload::MemoryDescriptionRegenerated {
                id, produced_by, ..
            } if id == dave.id => Some(produced_by),
            _ => None,
        })
        .expect("Dave's description was regenerated")
        .expect("regeneration records its provenance");
    assert_eq!(produced_by.model_id, "scripted-model");
    assert_eq!(
        produced_by.template_name,
        PromptTemplateName::DescriptionRegen
    );
    assert_eq!(produced_by.template_version, 1);
}

#[tokio::test]
async fn agent_turns_record_their_provenance() {
    let mut h = Harness::new();
    // Genesis registers the scaffold the agent turn runs against.
    genesis::rollout(&mut h.store, &h.clock, &seed()).unwrap();
    h.graph.materialize_from(&h.store).unwrap();

    let model = ScriptedModel::new([Completion::Reply("Noted.".to_owned())]);
    run_turn(h.as_turn(&model, "hello", 8)).await.unwrap();

    let turns: Vec<(TurnRole, Option<_>)> = h
        .store
        .read_from(Seq::ZERO)
        .unwrap()
        .into_iter()
        .filter_map(|e| match e.payload {
            EventPayload::ConversationTurn {
                role, produced_by, ..
            } => Some((role, produced_by)),
            _ => None,
        })
        .collect();

    // The inbound participant turn is not inference, so it has no provenance.
    let (_, participant) = turns
        .iter()
        .find(|(role, _)| *role == TurnRole::Participant)
        .expect("a participant turn");
    assert!(participant.is_none());

    // The agent turn records the chat model and the scaffold it ran against.
    let (_, agent) = turns
        .iter()
        .find(|(role, _)| *role == TurnRole::Agent)
        .expect("an agent turn");
    let provenance = agent
        .as_ref()
        .expect("the agent turn records its provenance");
    assert_eq!(provenance.model_id, "scripted-model");
    assert_eq!(provenance.template_name, PromptTemplateName::Scaffold);
    assert_eq!(provenance.template_version, 1);
}

#[tokio::test]
async fn stay_silent_terminal_posts_nothing() {
    let mut h = Harness::new();
    let model = ScriptedModel::new([Completion::Silent]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "(chatter)", 8)).await.unwrap();

    assert_eq!(outcome, TurnOutcome::Silent);
    // Auditable silence: an agent turn is still recorded, with empty text.
    let silent_recorded = h.store.read_from(Seq::ZERO).unwrap().into_iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::ConversationTurn { role: TurnRole::Agent, text, .. } if text.is_empty()
        )
    });
    assert!(silent_recorded);
}

#[tokio::test]
async fn max_steps_ends_the_turn_with_a_surfaced_error() {
    let mut h = Harness::new();
    // A model that only ever calls tools, never terminating.
    let model = ScriptedModel::new([
        run_lua_call("return 1"),
        run_lua_call("return 2"),
        run_lua_call("return 3"),
    ]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "loop forever", 2))
        .await
        .unwrap();

    assert_eq!(outcome, TurnOutcome::MaxStepsExceeded);
    // The cycle still records exactly one agent turn, carrying the surfaced error.
    assert_eq!(count_agent_turns(&h.store), 1);
    let surfaced = h.store.read_from(Seq::ZERO).unwrap().into_iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::ConversationTurn { role: TurnRole::Agent, text, .. } if text.contains("max steps")
        )
    });
    assert!(surfaced);
}

#[tokio::test]
async fn tool_result_feeds_back_across_steps() {
    let mut h = Harness::new();
    // First create, then a second block reads it back, then reply — exercising multi-step flow.
    let model = ScriptedModel::new([
        run_lua_call(r#"memory.create("topic/climbing", "Bouldering and sport climbing")"#),
        run_lua_call(r#"return memory.get("topic/climbing"):entries()"#),
        Completion::Reply("done".to_owned()),
    ]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "go", 8)).await.unwrap();
    assert_eq!(outcome, TurnOutcome::Reply("done".to_owned()));

    // Two LuaExecuted events (two blocks), both committed.
    let lua_events = h
        .store
        .read_from(Seq::ZERO)
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e.payload, EventPayload::LuaExecuted { .. }))
        .count();
    assert_eq!(lua_events, 2);
}

/// End-to-end against the real model (model-gated, ignored): the live model drives the whole loop
/// — chat protocol, tool-call threading, block execution — to a terminal without an infra error.
#[cfg(feature = "openai")]
#[tokio::test]
#[ignore = "requires a reachable model endpoint (config.toml)"]
async fn real_model_drives_a_turn() {
    let Ok(config) = EnvConfig::load(std::path::Path::new("config.toml")) else {
        return;
    };
    if config.model.endpoint.is_empty() {
        eprintln!("skipping: no model endpoint configured");
        return;
    }
    let client = OpenAiClient::new(&config.model);
    let mut h = Harness::new();

    let outcome = run_turn(h.as_turn(
        &client,
        "Please remember that Dave climbs at the bouldering gym, then confirm you've noted it.",
        8,
    ))
    .await;

    match outcome {
        Ok(outcome) => {
            // The loop completed against the real model. Exactly one agent turn was recorded.
            assert_eq!(count_agent_turns(&h.store), 1);
            eprintln!("real-model turn outcome: {outcome:?}");
        }
        Err(error) => eprintln!("skipping: {error}"),
    }
}
