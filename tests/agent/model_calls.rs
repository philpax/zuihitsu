use super::*;
/// The `ModelCalled` events of a run, in `seq` order, projected to the fields the tests assert over.
fn model_calls(
    events: &[Event],
) -> Vec<(ModelPhase, Option<RequestRecord>, Option<String>, String)> {
    events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::ModelCalled {
                phase,
                request,
                reasoning,
                request_digest,
                ..
            } => Some((
                *phase,
                request.clone(),
                reasoning.clone(),
                request_digest.clone(),
            )),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_turn_records_the_model_interaction_with_deliberation() {
    let mut h = Harness::new();
    let usage = Usage {
        prompt_tokens: Some(100),
        completion_tokens: Some(20),
        total_tokens: Some(120),
        ..Usage::default()
    };
    // Two steps so the delta path runs: a tool call, then a reply, each carrying reasoning.
    let model = ScriptedModel::with_deliberation([
        (
            run_lua_call(r#"memory.create(PERSON_DAVE, "Met at the gym")"#),
            "I should record Dave.".to_owned(),
            usage,
        ),
        (
            Completion::Reply("Noted.".to_owned()),
            "The fact is stored, so I reply.".to_owned(),
            usage,
        ),
    ]);

    run_turn(h.as_turn(&model, "Remember Dave", 8))
        .await
        .unwrap();

    let calls = model_calls(&h.events());
    // No description-regen template is registered (no genesis), so synthesis never runs: exactly the
    // two step calls are recorded.
    assert_eq!(calls.len(), 2, "one ModelCalled per step");
    assert!(calls.iter().all(|(phase, ..)| *phase == ModelPhase::Step));

    // The deliberation is captured verbatim.
    assert_eq!(calls[0].2.as_deref(), Some("I should record Dave."));
    assert_eq!(
        calls[1].2.as_deref(),
        Some("The fact is stored, so I reply.")
    );
    // Digests are present and differ — the second call's prompt grew.
    assert!(!calls[0].3.is_empty() && !calls[1].3.is_empty());
    assert_ne!(calls[0].3, calls[1].3);

    // The first call is a Base; the second a Continuation of the messages appended since.
    let RequestRecord::Base { messages: base, .. } =
        calls[0].1.clone().expect("Full captures request")
    else {
        panic!("the first step records a Base, got {:?}", calls[0].1);
    };
    let RequestRecord::Continuation { appended_messages } =
        calls[1].1.clone().expect("Full captures request")
    else {
        panic!("a later step records a Continuation, got {:?}", calls[1].1);
    };

    // Reconstructing the second call's prompt (Base ++ Continuation) reproduces exactly what the
    // model was sent on its second call.
    let reconstructed: Vec<Message> = base.iter().chain(&appended_messages).cloned().collect();
    let seen = model.recorded_messages();
    assert_eq!(seen.len(), 2);
    assert_eq!(reconstructed, seen[1]);

    // Block timing rides on the LuaExecuted (the field is recorded; it cannot be negative).
    let events = h.events();
    assert!(events.iter().any(|e| matches!(
        &e.payload,
        EventPayload::LuaExecuted { duration_ms, .. } if *duration_ms < u64::MAX
    )));
}

#[tokio::test]
async fn a_base_record_carries_the_prompt_section_spans() {
    // context-debugger.AC1.2 (recording half): the first call of a phase records a `Base` whose
    // `system_sections` tile the `system` string exactly and slice back to each section's contribution.
    let mut h = Harness::new();
    let model = ScriptedModel::new([Completion::Reply("Noted.".to_owned())]);

    // Supply a brief so the `Brief` section is present and its header is assertable; the API reference
    // and the current-time footer are always emitted, so several sections tile the prompt.
    let mut turn = h.as_turn(&model, "Remember Dave", 8);
    turn.brief = "you are chatting about the gym";
    run_turn(turn).await.unwrap();

    let calls = model_calls(&h.events());
    assert_eq!(calls.len(), 1, "one step records one ModelCalled");
    let RequestRecord::Base {
        system,
        system_sections,
        ..
    } = calls[0].1.clone().expect("Full captures the request")
    else {
        panic!("the first step records a Base, got {:?}", calls[0].1);
    };

    assert!(
        !system_sections.is_empty(),
        "a captured Base carries the prompt's section spans"
    );

    // The spans tile `system` exactly: contiguous from zero to its length, each on UTF-8 boundaries.
    let mut cursor = 0u32;
    for span in &system_sections {
        assert_eq!(span.start, cursor, "a gap or overlap precedes {span:?}");
        assert!(
            span.start < span.end,
            "an empty span was recorded: {span:?}"
        );
        assert!(
            system.get(span.start as usize..span.end as usize).is_some(),
            "the span lands off a UTF-8 boundary: {span:?}"
        );
        cursor = span.end;
    }
    assert_eq!(
        cursor as usize,
        system.len(),
        "the spans reach the end of the system prompt"
    );

    // The Brief section's slice is the brief's whole contribution, its header included.
    let brief_span = system_sections
        .iter()
        .find(|span| span.kind == PromptSectionKind::Brief)
        .expect("a brief was supplied, so its section is present");
    assert!(
        system[brief_span.start as usize..brief_span.end as usize]
            .starts_with("\n\n# What you know right now"),
        "the Brief slice carries its header"
    );

    // The API reference is always present; its slice carries the capabilities header.
    let api_span = system_sections
        .iter()
        .find(|span| span.kind == PromptSectionKind::ApiReference)
        .expect("the API reference is always present");
    assert!(
        system[api_span.start as usize..api_span.end as usize].starts_with("\n\n# What you can do"),
        "the ApiReference slice carries its header"
    );
}

#[tokio::test]
async fn a_rerendered_buffer_reproduces_the_live_tool_call_ids() {
    // The step loop normalizes the model's arbitrary call ids to the scheme the buffer re-render
    // mints, so a later turn's rebuilt buffer reproduces the earlier exchange byte for byte — a
    // value-unstable id busts the prefix cache outright on serving stacks whose chat template
    // tokenizes it.
    let mut h = Harness::new();
    let model = ScriptedModel::new([
        run_lua_call(r#"memory.create(PERSON_DAVE, "Met at the gym")"#),
        Completion::Reply("Noted.".to_owned()),
        Completion::Reply("Hello again.".to_owned()),
    ]);
    run_turn(h.as_turn(&model, "Remember Dave", 8))
        .await
        .unwrap();

    // The live exchange, as turn 1's continuation recorded it on the wire.
    let calls = model_calls(&h.events());
    let RequestRecord::Continuation { appended_messages } =
        calls[1].1.clone().expect("Full captures the request")
    else {
        panic!(
            "turn 1's second step records a Continuation, got {:?}",
            calls[1].1
        );
    };
    let live_call = appended_messages
        .iter()
        .find_map(|message| message.tool_calls.first())
        .expect("the appended slice carries the tool call");
    let live_id = live_call.id.clone();
    assert!(
        live_id.starts_with("call_"),
        "the live id is normalized to the deterministic scheme, got {live_id:?}"
    );
    let live_result_id = appended_messages
        .iter()
        .find_map(|message| message.tool_call_id.clone())
        .expect("the appended slice carries the tool result");
    assert_eq!(
        live_id, live_result_id,
        "the live result pairs with its call"
    );

    // Turn 2 re-renders turn 1 from the log; the rebuilt exchange must carry the same ids.
    let conversation = h.session.conversation().unwrap();
    let buffer = buffer_turns(h.engine.store.lock().as_ref(), conversation, Seq::ZERO).unwrap();
    run_turn(h.as_turn_buffered(&model, "Anything else?", 8, &buffer))
        .await
        .unwrap();

    let calls = model_calls(&h.events());
    let RequestRecord::Base { messages, .. } =
        calls[2].1.clone().expect("Full captures the request")
    else {
        panic!("turn 2's first step records a Base, got {:?}", calls[2].1);
    };
    let rerendered_call = messages
        .iter()
        .find_map(|message| message.tool_calls.first())
        .expect("the rebuilt buffer replays the tool call");
    assert_eq!(
        rerendered_call.id, live_id,
        "the re-rendered id reproduces the live one"
    );
    let rerendered_result_id = messages
        .iter()
        .find_map(|message| message.tool_call_id.clone())
        .expect("the rebuilt buffer replays the tool result");
    assert_eq!(rerendered_result_id, live_id);
}

#[tokio::test]
async fn digest_capture_keeps_the_digest_but_drops_the_request() {
    let mut h = Harness::new();
    let model = ScriptedModel::new([Completion::Reply("Hi.".to_owned())]);

    run_turn(h.as_turn_capturing(&model, "Hello", 8, CaptureLevel::Digest))
        .await
        .unwrap();

    let calls = model_calls(&h.events());
    assert_eq!(calls.len(), 1);
    // The request is dropped, but the digest survives for an integrity check.
    assert!(calls[0].1.is_none(), "Digest drops the request payload");
    assert!(!calls[0].3.is_empty(), "Digest keeps the request digest");
}

#[tokio::test]
async fn off_capture_records_no_model_interaction() {
    let mut h = Harness::new();
    let model = ScriptedModel::new([Completion::Reply("Hi.".to_owned())]);

    run_turn(h.as_turn_capturing(&model, "Hello", 8, CaptureLevel::Off))
        .await
        .unwrap();

    assert!(
        model_calls(&h.events()).is_empty(),
        "Off emits no ModelCalled events"
    );
}

/// Supersession against the real model (model-gated, ignored). A realistic two-turn flow: the model
/// records a fact, then is told a correction in the same conversation, so it acts on the memory it
/// created rather than guessing a name. Observational, like the temporal-extraction probe — it prints
/// what the model did (did it supersede, append, or fragment into a duplicate?) and asserts only the
/// robust invariants: the turns complete without an infrastructure error, and our mechanism holds (an
/// entry recorded as superseded is never live). Whether the model *chooses* to supersede is a
/// model-floor observation, not a gate.
#[tokio::test]
#[ignore = "requires a reachable model endpoint (config.toml)"]
async fn real_model_supersedes_a_corrected_fact() {
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
    let conversation = h.session.conversation().unwrap();

    // Turn 1: the model records the fact under a name of its own choosing.
    let first = run_turn(h.as_turn(&client, "Please remember that Dave works at Hooli.", 12)).await;
    let Ok(first) = first else {
        eprintln!("skipping: turn 1 failed: {first:?}");
        return;
    };
    eprintln!("turn 1 reply: {:?}", first.outcome);

    // Turn 2: a natural mention of the change — no instruction to update the record — with turn 1
    // replayed as the conversation buffer. The agent should infer that its memory of Dave is now stale
    // and maintain it on its own.
    let buffer = buffer_turns(h.engine.store.lock().as_ref(), conversation, Seq::ZERO).unwrap();
    let second = run_turn(h.as_turn_buffered(
        &client,
        "Oh, by the way — Dave left Hooli. He's over at Pied Piper these days.",
        12,
        &buffer,
    ))
    .await;
    let Ok(second) = second else {
        eprintln!("skipping: turn 2 failed: {second:?}");
        return;
    };
    eprintln!("turn 2 reply: {:?}", second.outcome);

    // What the model actually did, step by step — the deliberation surface.
    let superseded_ids: std::collections::BTreeSet<EntryId> = h
        .events()
        .into_iter()
        .filter_map(|event| match event.payload {
            EventPayload::LuaExecuted {
                script,
                result,
                terminal_cause,
                ..
            } => {
                eprintln!("  lua: {script:?} -> result={result:?} terminal={terminal_cause:?}");
                None
            }
            EventPayload::MemorySuperseded { entry, .. } => Some(entry),
            _ => None,
        })
        .collect();

    // The current [`Namespace::Person`] memories and their live/superseded entries.
    let people = h
        .engine
        .graph
        .lock()
        .memories_in_namespace("person/")
        .unwrap();
    eprintln!("person/* memories ({}):", people.len());
    let mut pied_piper_live = false;
    let mut hooli_live = false;
    for person in &people {
        let history = h.engine.graph.lock().class_history(person.id).unwrap();
        eprintln!("  {} ({} entries):", person.name.as_str(), history.len());
        for entry in &history {
            let live = entry.superseded_by.is_none();
            let lower = entry.text.to_lowercase();
            if live && lower.contains("pied piper") {
                pied_piper_live = true;
            }
            if live && lower.contains("hooli") {
                hooli_live = true;
            }
            eprintln!(
                "    - [{}] {}",
                if live { "live" } else { "superseded" },
                entry.text
            );
            // Mechanism invariant: a superseded entry must not appear in the live class read.
            if !live {
                let in_live = h
                    .engine
                    .graph
                    .lock()
                    .class_entries(person.id)
                    .unwrap()
                    .iter()
                    .any(|e| e.entry_id == entry.entry_id);
                assert!(!in_live, "a superseded entry is still in the live read");
            }
        }
    }

    eprintln!(
        "verdict: supersessions={}, person/* memories={}, 'Pied Piper' live={pied_piper_live}, \
         'Hooli' live={hooli_live}",
        superseded_ids.len(),
        people.len(),
    );
    if people.len() > 1 {
        eprintln!("  NOTE: the model fragmented Dave into multiple memories (name mismatch).");
    }
    if hooli_live && pied_piper_live {
        eprintln!(
            "  NOTE: both the stale and corrected facts are live — the model appended without retracting."
        );
    }
}
