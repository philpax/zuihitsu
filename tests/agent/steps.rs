use super::*;
#[tokio::test]
async fn agent_turns_record_their_provenance() {
    let mut h = Harness::new();
    // Genesis registers the scaffold the agent turn runs against.
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();

    let model = ScriptedModel::new([Completion::Reply("Noted.".to_owned())]);
    run_turn(h.as_turn(&model, "hello", 8)).await.unwrap();

    let turns: Vec<(TurnRole, Option<_>)> = h
        .events()
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
    assert_eq!(provenance.template_version, 22);
}

#[tokio::test]
async fn stay_silent_terminal_posts_nothing() {
    let mut h = Harness::new();
    let model = ScriptedModel::new([Completion::Silent]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "(chatter)", 8)).await.unwrap();

    assert_eq!(outcome, TurnOutcome::Silent);
    // Auditable silence: an agent turn is still recorded, with empty text.
    let silent_recorded = h.events().into_iter().any(|e| {
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
    assert_eq!(count_agent_turns(&h.events()), 1);
    let surfaced = h.events().into_iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::ConversationTurn { role: TurnRole::Agent, text, .. } if text.contains("max steps")
        )
    });
    assert!(surfaced);
}

/// The nearing-budget nudge (a system message telling the model to wrap up) lands exactly once, on
/// the step two before the bound, and persists into the frames after it — so the model gets the
/// legibility warning without it being re-appended every remaining step.
#[tokio::test]
async fn the_nearing_budget_nudge_lands_once_at_max_minus_two() {
    let mut h = Harness::new();
    // max_steps = 3: the nudge is due before step index 1 (max_steps - 2).
    let model = ScriptedModel::new([
        run_lua_call("return 1"),
        run_lua_call("return 2"),
        Completion::Reply("done".to_owned()),
    ]);

    run_turn(h.as_turn(&model, "go", 3)).await.unwrap();

    let nudge = "two steps remain in this turn";
    let count = |messages: &[Message]| {
        messages
            .iter()
            .filter(|m| m.content.contains(nudge))
            .count()
    };
    let seen = model.recorded_messages();
    assert_eq!(seen.len(), 3, "three generate calls");
    assert_eq!(count(&seen[0]), 0, "no nudge before the max-2 step");
    assert_eq!(count(&seen[1]), 1, "the nudge lands on the max-2 step");
    assert_eq!(count(&seen[2]), 1, "it persists once, not re-appended");
}

/// On the final step the loop withdraws the tools (`ToolChoice::None`) so the model must answer with
/// what it has, and that text terminates the turn as an ordinary `Reply` — not a `MaxStepsExceeded`.
#[tokio::test]
async fn the_final_step_forces_a_textual_answer() {
    let mut h = Harness::new();
    let model = ScriptedModel::new([
        run_lua_call("return 1"),
        run_lua_call("return 2"),
        Completion::Reply("here is what I found".to_owned()),
    ]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "go", 3)).await.unwrap();

    assert_eq!(
        outcome,
        TurnOutcome::Reply("here is what I found".to_owned())
    );
    // The earlier steps let the model choose; only the final step withdraws the tools.
    assert_eq!(
        model.recorded_tool_choices(),
        vec![ToolChoice::Auto, ToolChoice::Auto, ToolChoice::None],
    );
}

/// The forced final answer is a nudge, not a guarantee: a model that still produces no text on the
/// final step (a tool call, defying the withdrawn tools) falls back to the surfaced `MaxStepsExceeded`
/// terminal — the fallback the loop keeps, not the norm.
#[tokio::test]
async fn a_model_that_produces_no_text_on_the_final_step_still_max_steps() {
    let mut h = Harness::new();
    let model = ScriptedModel::new([run_lua_call("return 1"), run_lua_call("return 2")]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "go", 2)).await.unwrap();

    assert_eq!(outcome, TurnOutcome::MaxStepsExceeded);
    assert_eq!(count_agent_turns(&h.events()), 1);
    // The final step still had its tools withdrawn, even though the model did not reply.
    assert_eq!(
        model.recorded_tool_choices().last(),
        Some(&ToolChoice::None)
    );
}

#[tokio::test]
async fn tool_result_feeds_back_across_steps() {
    let mut h = Harness::new();
    // First create, then a second block reads it back, then reply — exercising multi-step flow.
    let model = ScriptedModel::new([
        run_lua_call(r#"memory.create(TOPIC_CLIMBING, "Bouldering and sport climbing")"#),
        run_lua_call(r#"return memory.get(TOPIC_CLIMBING):entries()"#),
        Completion::Reply("done".to_owned()),
    ]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "go", 8)).await.unwrap();
    assert_eq!(outcome, TurnOutcome::Reply("done".to_owned()));

    // Two LuaExecuted events (two blocks), both committed.
    let lua_events = h
        .events()
        .into_iter()
        .filter(|e| matches!(e.payload, EventPayload::LuaExecuted { .. }))
        .count();
    assert_eq!(lua_events, 2);
}

#[tokio::test]
async fn tool_calls_persist_in_the_buffer_across_turns() {
    // A turn's run_lua blocks (script + result) should survive into the next turn's buffer so the
    // model sees what it already did — not just the reply text, but the tool interaction itself.
    let mut h = Harness::new();
    let model = ScriptedModel::new([
        run_lua_call("return 'hello from lua'"),
        Completion::Reply("done".to_owned()),
        Completion::Reply("ok".to_owned()),
    ]);
    let conversation = h.session.conversation().unwrap();

    // Turn 1: a run_lua block then a reply.
    run_turn(h.as_turn(&model, "go", 8)).await.unwrap();

    // Rebuild the buffer from the recorded turns.
    let buffer = buffer_turns(h.engine.store.lock().as_ref(), conversation, Seq::ZERO).unwrap();

    // The agent's turn view should carry the tool step.
    let agent_turn = buffer
        .iter()
        .find(|t| t.role == TurnRole::Agent)
        .expect("an agent turn");
    assert_eq!(
        agent_turn.steps.len(),
        1,
        "the agent turn carries its run_lua step"
    );
    assert!(agent_turn.steps[0].result.contains("hello from lua"));

    // Turn 2: the model's first generate call should include the tool-call/result pair in its messages.
    run_turn(h.as_turn_buffered(&model, "next", 8, &buffer))
        .await
        .unwrap();

    let seen = model.recorded_messages();
    // Turn 1 had 2 generate calls; turn 2's first call is the last recorded.
    let turn2_messages = seen.last().unwrap();
    let has_tool_call = turn2_messages.iter().any(|m| !m.tool_calls.is_empty());
    let has_tool_result = turn2_messages.iter().any(|m| m.tool_call_id.is_some());
    assert!(
        has_tool_call,
        "turn 2 should see turn 1's tool call in the buffer"
    );
    assert!(
        has_tool_result,
        "turn 2 should see turn 1's tool result in the buffer"
    );
}

#[tokio::test]
async fn turn_report_counts_steps_and_blocks() {
    // The per-turn observability span records `steps` (model calls) and `blocks` (run_lua
    // executions); the counts live on `TurnReport`, so this verifies the substance the span carries
    // without depending on `tracing`'s global subscriber state (spec §Observability → per-turn spans).

    // A single reply: one model step, no blocks.
    let mut h = Harness::new();
    let model = ScriptedModel::new([Completion::Reply("hi".to_owned())]);
    let report = run_turn(h.as_turn(&model, "hello", 8)).await.unwrap();
    assert_eq!(report.outcome, TurnOutcome::Reply("hi".to_owned()));
    assert_eq!(report.steps, 1);
    assert_eq!(report.blocks, 0);

    // A multi-step turn: two run_lua calls then a reply → three steps, two blocks.
    let mut h = Harness::new();
    let model = ScriptedModel::new([
        run_lua_call("return 1"),
        run_lua_call("return 2"),
        Completion::Reply("done".to_owned()),
    ]);
    let report = run_turn(h.as_turn(&model, "go", 8)).await.unwrap();
    assert_eq!(report.outcome, TurnOutcome::Reply("done".to_owned()));
    assert_eq!(report.steps, 3);
    assert_eq!(report.blocks, 2);
}

/// End-to-end against the real model (model-gated, ignored): the live model drives the whole loop
/// — chat protocol, tool-call threading, block execution — to a terminal without an infra error.
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
            assert_eq!(count_agent_turns(&h.events()), 1);
            eprintln!("real-model turn outcome: {outcome:?}");
        }
        Err(error) => eprintln!("skipping: {error}"),
    }
}

/// Temporal extraction against the real model (model-gated, ignored, tracked/non-gating): a turn
/// whose content carries natural-language times should leave at least one durable entry with a
/// resolved `occurred_at`. Logs the timed/total rate — load-bearing news about the model floor, the
/// same epistemic status as the compaction continuity metric (spec §Validation).
#[tokio::test]
#[ignore = "requires a reachable model endpoint (config.toml)"]
async fn real_model_extracts_temporal_references() {
    let Ok(config) = EnvConfig::load(std::path::Path::new("config.toml")) else {
        return;
    };
    if config.model.endpoint.is_empty() {
        eprintln!("skipping: no model endpoint configured");
        return;
    }
    let client = OpenAiClient::new(&config.model);
    let mut h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();

    let outcome = run_turn(h.as_turn(
        &client,
        "Please note: I met Dave at the climbing gym last Tuesday, and the database migration \
         ships next Friday.",
        8,
    ))
    .await;
    if let Err(error) = outcome {
        eprintln!("skipping: {error}");
        return;
    }

    // Scan the namespaces a turn like this could write into for entries that gained an occurrence.
    let (mut total, mut timed) = (0usize, 0usize);
    for prefix in ["person/", "topic/", "project/", "event/"] {
        for memory in h.engine.graph.lock().memories_in_namespace(prefix).unwrap() {
            for entry in h.engine.graph.lock().entries_local(memory.id).unwrap() {
                total += 1;
                if entry.occurred_sort.is_some() {
                    timed += 1;
                    eprintln!("timed: {} :: {}", memory.name.as_str(), entry.text);
                }
            }
        }
    }
    eprintln!("temporal extraction: {timed}/{total} durable entries carry an occurred_at");
    assert!(
        timed >= 1,
        "expected the model to resolve at least one temporal reference"
    );
}
