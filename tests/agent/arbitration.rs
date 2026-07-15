use super::*;
#[tokio::test]
async fn a_neutral_third_entry_does_not_dilute_the_contradiction() {
    // The conflicting-accounts shape the eval missed in 3 of 5 runs: two accounts of one fact stand
    // as sibling public entries, and a neutral third entry (the event's own title) sits alongside
    // them. The synthesis prompt now closes with an explicit pairwise contradiction check over the
    // numbered statements, so the scripted model pairs statements 2 and 3 while crediting neither,
    // and a `BeliefArbitrated` with an empty `credited` lands. Both the emitted event and the shape
    // of the prompt the pass sent are asserted, since the fix is a prompt change.
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
async fn two_attributed_accounts_are_arbitrated_and_read_back_disputed() {
    // The regression the widened arbitration pool closes: two relayed-but-conflicting accounts the
    // agent marked `Attributed` (rather than `Public`) must still collide. With no `Public` entry the
    // description pass never runs — there is nothing to summarize into the always-visible prose — but
    // arbitration now scans the `Public` + `Attributed` slice, so the two attributed location accounts
    // are numbered together, flagged, and (crediting neither) read back `disputed`.
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

    // Two participants relay conflicting secondhand accounts of the location; each is recorded on the
    // event memory as an `Attributed` fact, credited to the teller who relayed it. The teller memories
    // are empty, so they are stale but drive no synthesis call — only the event memory does.
    let model = ScriptedModel::new([
        run_lua_call(
            r#"local marcus = memory.create(PERSON_MARCUS)
               local erin = memory.create(PERSON_ERIN)
               local e = memory.create(EVENT_ALL_HANDS)
               e:append("The all-hands is in the main auditorium", { told_by = PERSON_MARCUS, visibility = "attributed" })
               e:append("The all-hands is in the rooftop terrace", { told_by = PERSON_ERIN, visibility = "attributed" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        // No description call: with no public entry there is nothing to summarize. Only the focused
        // arbitration call runs, over the two attributed statements numbered 1 and 2.
        arbitrate_call(SynthesizeArbitration {
            competing: vec![1, 2],
            credited: vec![],
            statement: "Two standing accounts of the location, each relayed secondhand: the main \
                        auditorium and the rooftop terrace, neither retracted."
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
    // Both accounts were recorded as attributed, so the widened pool is exactly what arbitration saw.
    assert!(
        entries
            .iter()
            .all(|entry| entry.visibility == zuihitsu::Visibility::Attributed),
        "both relayed accounts should be recorded as attributed"
    );

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
    assert!(resolution.credited.is_empty());

    // Both attributed entries read back disputed — the marker the read path renders regardless of
    // visibility.
    let disputed = h
        .engine
        .graph
        .lock()
        .disputed_entries(all_hands.id)
        .unwrap();
    assert_eq!(
        disputed,
        [entries[0].entry_id, entries[1].entry_id]
            .into_iter()
            .collect()
    );

    // No description was synthesized for the event — with no public entry, the description pass had
    // nothing to write, so no attributed content could reach the always-visible summary.
    assert!(
        !h.events().iter().any(|event| matches!(
            &event.payload,
            EventPayload::MemoryDescriptionRegenerated { id, .. } if *id == all_hands.id
        )),
        "no description should be regenerated for an all-attributed memory"
    );

    // The arbitration pass posed the contradiction check over both attributed statements, each carrying
    // its "via <teller>" annotation.
    let arbitrated_over_both = model.recorded_messages().iter().flatten().any(|message| {
        let p = &message.content;
        p.contains("1. [from ")
            && p.contains("2. [from ")
            && p.contains("main auditorium")
            && p.contains("rooftop terrace")
            && p.contains("incompatible values for the same fact")
    });
    assert!(
        arbitrated_over_both,
        "arbitration must number both attributed statements and pose the contradiction check"
    );
}

#[tokio::test]
async fn a_mixed_public_and_attributed_pair_is_arbitrated_without_leaking_into_the_description() {
    // The cross-posture collision: one public account and one attributed (relayed-secondhand) account
    // of the same fact. The description pass sees only the public entry — the attributed account keeps
    // its "via <teller>" framing out of the always-visible summary — yet arbitration scans the wider
    // `Public` + `Attributed` slice, so the two still collide and read back disputed.
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
        run_lua_call(
            r#"local marcus = memory.create(PERSON_MARCUS)
               local e = memory.create(EVENT_ALL_HANDS)
               e:append("The all-hands is in the main auditorium", { by_agent = true, visibility = "public" })
               e:append("The all-hands is in the rooftop terrace", { told_by = PERSON_MARCUS, visibility = "attributed" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        // The description pass sees only the public entry, so its prose names the auditorium and never
        // the attributed terrace.
        synthesize_call(SynthesizeReply::description(
            "The all-hands is held in the main auditorium.",
        )),
        // The focused arbitration call sees the wider pool: statements 1 (public) and 2 (attributed)
        // collide, and neither is yet known to be right.
        arbitrate_call(SynthesizeArbitration {
            competing: vec![1, 2],
            credited: vec![],
            statement:
                "Two standing accounts of the location: the main auditorium and the rooftop \
                        terrace, neither retracted."
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
        competing_entries,
        resolution,
        ..
    } = &arbitrations[0]
    else {
        unreachable!();
    };
    // The public entry (1) and the attributed entry (2) are the two competitors.
    assert_eq!(
        *competing_entries,
        vec![entries[0].entry_id, entries[1].entry_id]
    );
    assert!(resolution.credited.is_empty());

    let disputed = h
        .engine
        .graph
        .lock()
        .disputed_entries(all_hands.id)
        .unwrap();
    assert_eq!(
        disputed,
        [entries[0].entry_id, entries[1].entry_id]
            .into_iter()
            .collect()
    );

    let prompts: Vec<String> = model
        .recorded_messages()
        .iter()
        .flatten()
        .map(|message| message.content.clone())
        .collect();
    // The description pass saw the public account but never the attributed one — the split the widened
    // pool preserves.
    assert!(
        prompts.iter().any(|p| {
            p.contains("main auditorium")
                && !p.contains("rooftop terrace")
                && !p.contains("incompatible values for the same fact")
        }),
        "the description pass must see only the public entry: {prompts:?}"
    );
    // The arbitration pass saw both, numbered together.
    assert!(
        prompts.iter().any(|p| {
            p.contains("1. [from ")
                && p.contains("2. [from ")
                && p.contains("main auditorium")
                && p.contains("rooftop terrace")
                && p.contains("incompatible values for the same fact")
        }),
        "arbitration must number the public and attributed statements together: {prompts:?}"
    );

    // The regenerated description carries the public account only — the attributed terrace never
    // reaches the always-visible summary.
    let description = h
        .events()
        .into_iter()
        .rev()
        .find_map(|event| match event.payload {
            EventPayload::MemoryDescriptionRegenerated { id, new_text, .. }
                if id == all_hands.id =>
            {
                Some(new_text)
            }
            _ => None,
        })
        .expect("the memory's public entry drives a description");
    assert!(description.contains("main auditorium"));
    assert!(
        !description.to_lowercase().contains("terrace"),
        "the attributed account must not leak into the description: {description:?}"
    );
}

#[tokio::test]
async fn a_private_entry_stays_out_of_the_description_but_is_still_extracted() {
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
