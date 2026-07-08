use super::*;
async fn a_neutral_third_entry_does_not_dilute_the_contradiction() {
    // The conflicting-accounts shape the eval missed in 3 of 5 runs: two accounts of one fact stand
    // as sibling public entries, and a neutral third entry (the event's own title) sits alongside
    // them. The synthesis prompt now closes with an explicit pairwise contradiction check over the
    // numbered statements, so the scripted model pairs statements 2 and 3 while crediting neither,
    // and a `BeliefArbitrated` with an empty `credited` lands. Both the emitted event and the shape
    // of the prompt the pass sent are asserted, since the fix is a prompt change.
    let h = Harness::new();
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

    // One neutral title entry and two conflicting location accounts (in the live scenario these
    // arrive from two tellers across turns; here the memory's public entries are scripted directly).
    let model = ScriptedModel::new([
        run_lua_call(
            r#"local e = memory.create(EVENT_ALL_HANDS)
               e:append("The all-hands meeting", { by_agent = true, visibility = "public" })
               e:append("Located in the main auditorium", { by_agent = true, visibility = "public" })
               e:append("Located in the rooftop terrace", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        // The description call, then the focused arbitration call: the neutral statement 1 stands apart;
        // statements 2 and 3 collide, and neither is yet known to be right, so `credited` is left empty.
        synthesize_call(SynthesizeReply::description(
            "The all-hands meeting, reported in either the main auditorium or the rooftop \
             terrace — the accounts disagree.",
        )),
        arbitrate_call(SynthesizeArbitration {
            competing: vec![2, 3],
            credited: vec![],
            statement: "Two standing accounts of the location: the main auditorium and the \
                        rooftop terrace, neither retracted."
                .to_owned(),
        }),
    ]);
    run_turn(h.as_turn(&model, "Where is the all-hands?", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    let all_hands = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Event.with_name("all-hands"))
        .unwrap()
        .unwrap();
    let entries = h.engine.graph.lock().entries_local(all_hands.id).unwrap();
    let arbitrations = belief_arbitrations(&h.events());
    assert_eq!(arbitrations.len(), 1);
    let EventPayload::BeliefArbitrated {
        memory,
        competing_entries,
        resolution,
        ..
    } = &arbitrations[0]
    else {
        unreachable!();
    };
    assert_eq!(memory, &all_hands.id);
    // The two conflicting statements (2 and 3) resolved to their entries; the neutral first entry is
    // not among them.
    assert_eq!(
        *competing_entries,
        vec![entries[1].entry_id, entries[2].entry_id]
    );
    // Both accounts stand: neither is credited above the other.
    assert!(resolution.credited.is_empty());

    // The pass posed the contradiction question over the numbered statements: the synthesis prompt
    // carries the numbered statements and the explicit pairwise contradiction ask.
    let prompts: Vec<String> = model
        .recorded_messages()
        .iter()
        .flatten()
        .map(|message| message.content.clone())
        .collect();
    assert!(
        prompts.iter().any(|p| {
            p.contains("1. [from ")
                && p.contains("] The all-hands meeting")
                && p.contains("] Located in the main auditorium")
                && p.contains("] Located in the rooftop terrace")
                && p.contains("2. [from ")
                && p.contains("3. [from ")
                && p.contains("incompatible values for the same fact")
        }),
        "the synthesis prompt must number the statements and pose the contradiction check: {prompts:?}"
    );
}

#[tokio::test]
async fn a_both_stand_arbitration_survives_a_null_credited() {
    // The conflicting-accounts drop path: two accounts of one fact stand as sibling public entries and
    // the synthesis flags them, but — crediting neither — the model expresses the empty `credited` by
    // emitting `null` rather than `[]`. A strict sub-object parse would throw the whole conflict away
    // over exactly the both-stand shape the scenario tests; the lenient salvage keeps it, so a
    // `BeliefArbitrated` with an empty `credited` still lands. The reply is hand-built JSON so
    // `credited` is a literal `null`, which the typed harness reply cannot express.
    let h = Harness::new();
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
        run_lua_call(
            r#"local e = memory.create(EVENT_ALL_HANDS)
               e:append("Located in the main auditorium", { by_agent = true, visibility = "public" })
               e:append("Located in the rooftop terrace", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        synthesize_call(SynthesizeReply::description(
            "The all-hands, reported in either the main auditorium or the rooftop terrace — the \
             accounts disagree.",
        )),
        // The focused arbitration reply: statements 1 and 2 collide; `credited` is a literal `null`,
        // the loose way a model says "neither", which the typed harness reply cannot express — so this
        // one is hand-built JSON.
        Completion::Reply(
            r#"{
                "competing": [1, 2],
                "credited": null,
                "statement": "Two standing accounts of the location, neither retracted."
            }"#
            .to_owned(),
        ),
    ]);
    run_turn(h.as_turn(&model, "Where is the all-hands?", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    let all_hands = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Event.with_name("all-hands"))
        .unwrap()
        .unwrap();
    let entries = h.engine.graph.lock().entries_local(all_hands.id).unwrap();
    let arbitrations = belief_arbitrations(&h.events());
    assert_eq!(arbitrations.len(), 1);
    let EventPayload::BeliefArbitrated {
        memory,
        competing_entries,
        resolution,
        ..
    } = &arbitrations[0]
    else {
        unreachable!();
    };
    assert_eq!(memory, &all_hands.id);
    assert_eq!(
        *competing_entries,
        vec![entries[0].entry_id, entries[1].entry_id]
    );
    // The null `credited` salvaged to an empty set: both accounts stand.
    assert!(resolution.credited.is_empty());
}

#[tokio::test]
async fn a_private_entry_stays_out_of_the_description_but_is_still_extracted() {
    let h = Harness::new();
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

    // Dave's memory carries one public fact and one private, future-dated aside.
    let model = ScriptedModel::new([
        run_lua_call(
            r#"local d = memory.create(PERSON_DAVE)
               d:append("Dave is a climber", { by_agent = true, visibility = "public" })
               d:append("Dave has a private therapy session next Tuesday", { by_agent = true, visibility = "private" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        // The description pass sees only the public entry.
        synthesize_call(SynthesizeReply::description("Dave is a climber.")),
        // The focused extraction pass over the private untimed entry resolves its occurrence.
        synthesize_call(
            SynthesizeReply::description("(discarded)")
                .with_occurrence(SynthesizeOccurrence::day(1, "2026-06-16")),
        ),
    ]);
    run_turn(h.as_turn(&model, "Remember Dave", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    // The description-synthesis prompt was shown the public fact but never the private aside — the leak
    // the split closes.
    let prompts: Vec<String> = model
        .recorded_messages()
        .iter()
        .flatten()
        .map(|message| message.content.clone())
        .collect();
    assert!(
        prompts
            .iter()
            .any(|p| p.contains("Dave is a climber") && !p.contains("therapy")),
        "the description pass must not see the private entry: {prompts:?}"
    );
    assert!(
        prompts.iter().any(|p| p.contains("therapy")),
        "the private entry must still reach the focused extraction pass"
    );

    // Yet the private entry still gained its occurrence (so a private reminder can still fire).
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    let entries = h.engine.graph.lock().entries_local(dave.id).unwrap();
    let therapy = entries
        .iter()
        .find(|entry| entry.text.contains("therapy"))
        .unwrap();
    assert_eq!(therapy.occurred_sort, Some(day_noon("2026-06-16")));
}
