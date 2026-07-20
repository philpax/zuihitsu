use super::*;
#[tokio::test]
async fn tool_call_then_reply_commits_and_replies() {
    let mut h = Harness::new();
    let model = ScriptedModel::new([
        run_lua_call(
            r#"memory.create(PERSON_DAVE, "Met at the climbing gym", { visibility = "public" })"#,
        ),
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
    assert!(
        h.engine
            .graph
            .lock()
            .memory_by_name(Namespace::Person.with_name("dave"))
            .unwrap()
            .is_some()
    );
    // Exactly one agent turn for the cycle, plus the inbound participant turn and a LuaExecuted.
    assert_eq!(count_agent_turns(&h.events()), 1);
    let events = h.events();
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

/// The verbatim special-token leak observed in the wild: a pseudo-tool-call the model transcribed as
/// plain reply text at the forced-answer step.
const MALFORMED_REPLY: &str =
    "<|tool_call>call:run_lua{script:<|\"|>memory.search(\"decided\")<|\"|>}<tool_call|>";

/// Whether any recorded `ConversationTurn` carries the special-token markup — the invariant the guard
/// protects: such markup must never reach a persisted turn, and so never a participant.
fn any_turn_contains(events: &[Event], needle: &str) -> bool {
    events.iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::ConversationTurn { text, .. } if text.contains(needle)
        )
    })
}

#[tokio::test]
async fn a_malformed_reply_is_resampled_and_the_clean_retry_lands() {
    let mut h = Harness::new();
    // The first completion leaks special-token markup; the guard resamples the same step, and the
    // clean retry is what the participant receives.
    let model = ScriptedModel::new([
        Completion::Reply(MALFORMED_REPLY.to_owned()),
        Completion::Reply("Noted — I'll remember that.".to_owned()),
    ]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "What did we decide?", 8))
        .await
        .unwrap();

    assert_eq!(
        outcome,
        TurnOutcome::Reply("Noted — I'll remember that.".to_owned())
    );
    // Exactly one agent turn recorded, and the markup appears in no ConversationTurn.
    assert_eq!(count_agent_turns(&h.events()), 1);
    assert!(!any_turn_contains(&h.events(), "<|"));
    assert!(!any_turn_contains(&h.events(), "|>"));
}

#[tokio::test]
async fn two_consecutive_malformed_replies_fall_to_the_silent_terminal() {
    let mut h = Harness::new();
    // Both the initial reply and its resample leak markup; the guard delivers silence rather than
    // markup, and the malformed text is recorded nowhere.
    let model = ScriptedModel::new([
        Completion::Reply(MALFORMED_REPLY.to_owned()),
        Completion::Reply(MALFORMED_REPLY.to_owned()),
    ]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "What did we decide?", 8))
        .await
        .unwrap();

    assert_eq!(outcome, TurnOutcome::Silent);
    // A single (empty) agent turn stands for the silent terminal, and the markup is nowhere in the log.
    assert_eq!(count_agent_turns(&h.events()), 1);
    assert!(!any_turn_contains(&h.events(), "<|"));
    assert!(!any_turn_contains(&h.events(), "|>"));
}

#[tokio::test]
async fn descriptions_regenerate_after_a_turn() {
    let mut h = Harness::new();
    // Genesis registers the description-regen template the write path reads.
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
    h.baseline_descriptions();

    let model = ScriptedModel::new([
        // A public fact about Dave (the description is synthesized from Public entries only).
        run_lua_call(
            r#"local d = memory.create(PERSON_DAVE)
               d:append("Met at the climbing gym", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted — I'll remember Dave.".to_owned()),
        // The post-turn synthesis call: a `response_format`-constrained reply carries the description
        // as clean JSON (the entry has no temporal phrase, so no occurrences).
        synthesize_call(SynthesizeReply::description(
            "Dave, whom I met at the climbing gym.",
        )),
    ]);

    run_turn(h.as_turn(&model, "Remember Dave", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    // The written memory's description was regenerated from its entries after the cycle.
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    assert_eq!(dave.description, "Dave, whom I met at the climbing gym.");
    // It carries provenance: which model and template produced it. (Genesis also seeds self's
    // description, with null provenance, so match Dave's specifically.)
    let produced_by = h
        .events()
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
async fn a_rename_re_describes_the_memory_under_the_new_name() {
    // A rename changes no content, but the description is synthesized under the memory's name, so it
    // must be re-synthesized — otherwise the description (which reaches participants in briefs) keeps
    // the old name (spec §Identity → Renaming, deadname-safety).
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
    h.baseline_descriptions();

    let model = ScriptedModel::new([
        // Turn 1: a public fact, then its description synthesized under the old name.
        run_lua_call(
            r#"local d = memory.create(PERSON_DAVE)
               d:append("Handles the deploys.", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        synthesize_call(SynthesizeReply::description("Dave handles the deploys.")),
        // Turn 2: the rename — no content change.
        run_lua_call(r#"memory.get(PERSON_DAVE):rename(PERSON_SARAH)"#),
        Completion::Reply("Will do.".to_owned()),
        // The rename re-triggers synthesis, now under the new name — no "Dave".
        synthesize_call(SynthesizeReply::description("Sarah handles the deploys.")),
    ]);

    run_turn(h.as_turn(&model, "Dave handles the deploys", 8))
        .await
        .unwrap();
    h.describe(&model).await;
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    assert_eq!(dave.description, "Dave handles the deploys.");

    run_turn(h.as_turn(&model, "I go by Sarah now", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    // The rename alone re-described the memory under the new handle; the old name is gone from the
    // description, so it no longer rides into a brief.
    let sarah = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("sarah"))
        .unwrap()
        .unwrap();
    assert_eq!(sarah.description, "Sarah handles the deploys.");
}

#[tokio::test]
async fn turn_skip_ends_silent() {
    let mut h = Harness::new();
    // The agent creates a memory, then calls turn.skip() — no reply should follow.
    let model = ScriptedModel::new([
        run_lua_call(r#"memory.create("topic/incidental", "A note")"#),
        run_lua_call(r#"turn.skip()"#),
    ]);

    let TurnReport { outcome, .. } = run_turn(h.as_turn(&model, "hello", 8)).await.unwrap();

    // The turn ended silent — no reply, no max-steps.
    assert_eq!(outcome, TurnOutcome::Silent);
}

#[tokio::test]
async fn turn_skip_commits_writes() {
    let mut h = Harness::new();
    // The agent writes a memory before calling turn.skip() — the write should persist.
    let model = ScriptedModel::new([
        run_lua_call(r#"memory.create("topic/skip-test", "Committed before skip")"#),
        run_lua_call(r#"turn.skip("not worth a reply")"#),
    ]);

    run_turn(h.as_turn(&model, "hello", 8)).await.unwrap();

    // The memory was created and committed despite the skip.
    assert!(
        h.engine
            .graph
            .lock()
            .memory_by_name(Namespace::Topic.with_name("skip-test"))
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn turn_skip_records_lua_event() {
    let mut h = Harness::new();
    let model = ScriptedModel::new([
        run_lua_call(r#"memory.create("topic/skip-event", "Written")"#),
        run_lua_call(r#"turn.skip("not addressed to me")"#),
    ]);

    run_turn(h.as_turn(&model, "hello", 8)).await.unwrap();

    let events = h.events();
    // The second LuaExecuted should carry a Skipped terminal cause.
    let lua_events: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::LuaExecuted { terminal_cause, .. } => Some(terminal_cause),
            _ => None,
        })
        .collect();
    assert!(
        lua_events
            .iter()
            .any(|cause| matches!(cause, Some(TerminalCause::Skipped(Some(reason))) if reason == "not addressed to me")),
        "expected a LuaExecuted with Skipped terminal cause, got: {lua_events:?}"
    );
}

#[tokio::test]
async fn turn_skip_with_reason() {
    let mut h = Harness::new();
    let model = ScriptedModel::new([run_lua_call(r#"turn.skip("deliberately silent")"#)]);

    run_turn(h.as_turn(&model, "hello", 8)).await.unwrap();

    let events = h.events();
    // The LuaExecuted should carry the reason in the Skipped cause.
    let has_skip_with_reason = events.iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::LuaExecuted {
                terminal_cause: Some(TerminalCause::Skipped(Some(reason))),
                ..
            } if reason == "deliberately silent"
        )
    });
    assert!(
        has_skip_with_reason,
        "expected a LuaExecuted with Skipped(\"deliberately silent\")"
    );
}
