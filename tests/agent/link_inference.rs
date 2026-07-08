use super::*;

/// A `Completion::Reply` carrying a serialized [`LinkInferenceArgs`] reply.
fn link_inference_call(args: LinkInferenceArgs) -> Completion {
    Completion::Reply(serde_json::to_string(&args).unwrap())
}

#[tokio::test]
async fn link_inference_registers_and_links_from_content() {
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
    h.baseline_link_inference();

    // The turn creates person/dave and topic/zephyr, appending a public entry about authorship.
    // The agent does NOT call mem:link — the inference pass should extract the relationship.
    let model = ScriptedModel::new([
        run_lua_call(
            r#"memory.create(PERSON_DAVE, "a person")
               local zephyr = memory.create("topic/zephyr")
               zephyr:append("Authored by Dave", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        // The describe catch-up synthesizes zephyr (its public authorship entry); person/dave is not
        // synthesized, its only entry being a private description mirror, which is exempt from both the
        // public description pass and temporal extraction.
        synthesize_call(SynthesizeReply::description("The zephyr project.")),
        // The link-inference call: register authored_by and link zephyr → dave.
        link_inference_call(LinkInferenceArgs {
            new_relations: vec![NewRelationSpec {
                name: "authored_by".to_owned(),
                inverse: "authored".to_owned(),
                from_card: "many".to_owned(),
                to_card: "one".to_owned(),
                symmetric: false,
                reflexive: false,
                description: String::new(),
            }],
            links: vec![InferredLink {
                entry: 1,
                relation: "authored_by".to_owned(),
                target: "person/dave".to_owned(),
                direction: "to".to_owned(),
            }],
        }),
    ]);

    run_turn(h.as_turn(&model, "Working on zephyr, authored by Dave", 8))
        .await
        .unwrap();
    h.describe(&model).await;
    h.link_inference(&model).await;

    let events = h.events();

    // A LinkTypeRegistered for authored_by was committed.
    let registered = events.iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::LinkTypeRegistered { name, .. } if name.as_str() == "authored_by"
        )
    });
    assert!(
        registered,
        "the pass should register the authored_by relation"
    );

    // An inferred LinkCreated from topic/zephyr to person/dave under authored_by.
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    let zephyr = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Topic.with_name("zephyr"))
        .unwrap()
        .unwrap();

    let linked = events.iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::LinkCreated { from, to, relation, source, .. }
            if *from == zephyr.id && *to == dave.id
              && relation.as_str() == "authored_by"
              && *source == zuihitsu::LinkSource::Inferred
        )
    });
    assert!(
        linked,
        "the pass should create an inferred authored_by link"
    );
}

#[tokio::test]
async fn link_inference_honors_a_seeded_inverse_label() {
    // The model is shown both labels of every registered pair, so it legitimately phrases a link
    // through the inverse — `created` for the seeded `created_by` — with no new registration to
    // propose. The pass must resolve it onto the canonical relation with the direction flipped,
    // not drop it as unregistered.
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
    h.baseline_link_inference();

    let model = ScriptedModel::new([
        run_lua_call(
            r#"memory.create("person/clara", "a person")
               local zephyr = memory.create("topic/zephyr")
               zephyr:append("This project was created by Clara", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        synthesize_call(SynthesizeReply::description("The zephyr project.")),
        // The inference reply names the edge through the seeded inverse, registering nothing new:
        // clara --created--> zephyr, the same fact as zephyr --created_by--> clara.
        link_inference_call(LinkInferenceArgs {
            new_relations: vec![],
            links: vec![InferredLink {
                entry: 1,
                relation: "created".to_owned(),
                target: "person/clara".to_owned(),
                direction: "from".to_owned(),
            }],
        }),
    ]);

    run_turn(h.as_turn(&model, "This project was created by Clara", 8))
        .await
        .unwrap();
    h.describe(&model).await;
    h.link_inference(&model).await;

    let clara = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("clara"))
        .unwrap()
        .unwrap();
    let zephyr = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Topic.with_name("zephyr"))
        .unwrap()
        .unwrap();

    let linked = h.events().iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::LinkCreated { from, to, relation, source, .. }
            if *from == zephyr.id && *to == clara.id
              && relation.as_str() == "created_by"
              && *source == zuihitsu::LinkSource::Inferred
        )
    });
    assert!(
        linked,
        "an inverse-labeled inferred link should land on the canonical relation, direction flipped"
    );
}

#[tokio::test]
async fn link_inference_is_idempotent() {
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

    h.baseline_link_inference();
    let model = ScriptedModel::new([
        run_lua_call(
            r#"memory.create(PERSON_DAVE, "a person")
               local zephyr = memory.create("topic/zephyr")
               zephyr:append("Authored by Dave", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        synthesize_call(SynthesizeReply::description("A person.")),
        synthesize_call(SynthesizeReply::description("The zephyr project.")),
        link_inference_call(LinkInferenceArgs {
            new_relations: vec![NewRelationSpec {
                name: "authored_by".to_owned(),
                inverse: "authored".to_owned(),
                from_card: "many".to_owned(),
                to_card: "one".to_owned(),
                symmetric: false,
                reflexive: false,
                description: String::new(),
            }],
            links: vec![InferredLink {
                entry: 1,
                relation: "authored_by".to_owned(),
                target: "person/dave".to_owned(),
                direction: "to".to_owned(),
            }],
        }),
    ]);

    run_turn(h.as_turn(&model, "Working on zephyr, authored by Dave", 8))
        .await
        .unwrap();
    h.describe(&model).await;
    h.link_inference(&model).await;

    // Count inferred LinkCreated events after the first pass.
    let first_count = h
        .events()
        .into_iter()
        .filter(|e| {
            matches!(
                &e.payload,
                EventPayload::LinkCreated {
                    source: zuihitsu::LinkSource::Inferred,
                    ..
                }
            )
        })
        .count();

    // Re-run from the same cursor — no new events (the cursor advance prevents re-scanning).
    h.link_inference(&model).await;
    let second_count = h
        .events()
        .into_iter()
        .filter(|e| {
            matches!(
                &e.payload,
                EventPayload::LinkCreated {
                    source: zuihitsu::LinkSource::Inferred,
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        first_count, second_count,
        "re-running should produce no new inferred links"
    );
}

#[tokio::test]
async fn link_inference_degrades_gracefully_with_no_usable_reply() {
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
    h.baseline_link_inference();

    let model = ScriptedModel::new([
        run_lua_call(
            r#"memory.create(PERSON_DAVE, "a person")
               local zephyr = memory.create("topic/zephyr")
               zephyr:append("Authored by Dave", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        synthesize_call(SynthesizeReply::description("A person.")),
        synthesize_call(SynthesizeReply::description("The zephyr project.")),
    ]);

    run_turn(h.as_turn(&model, "Working on zephyr, authored by Dave", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    // Run the pass with a model that has no more scripted completions — each generate call
    // returns Exhausted, which the pass logs and skips (graceful degradation: a failed inference
    // leaves the memory unchanged — no link, no harm).
    h.link_inference(&ScriptedModel::new([])).await;

    let inferred = h
        .events()
        .into_iter()
        .filter(|e| {
            matches!(
                &e.payload,
                EventPayload::LinkCreated {
                    source: zuihitsu::LinkSource::Inferred,
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        inferred, 0,
        "no inferred links should be created with no usable reply"
    );
}

/// With `linking` disabled, calling `:link` from Lua yields a nil-call error (the method is not
/// installed), while the link-inference pass — which runs as a model call, not through the agent's
/// Lua — still creates the link. This is the AC.9 contract: the three gates (Lua registration, API
/// reference, scaffold) all drop linking, so the inference pass is the sole path to a link.
#[tokio::test]
async fn disabled_linking_rejects_mem_link_but_inference_still_links() {
    let disabled = InstanceFeatures {
        linking: false,
        ..Default::default()
    };
    let h = Harness::with_features(disabled);
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &disabled,
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.baseline_descriptions();
    h.baseline_link_inference();

    // Create the two memories through a turn (the agent-authored path that sets visibility
    // correctly), appending the authorship entry to topic/zephyr — but NOT calling mem:link.
    let create_model = ScriptedModel::new([
        run_lua_call(
            r#"memory.create(PERSON_DAVE, "a person")
               local zephyr = memory.create("topic/zephyr")
               zephyr:append("Authored by Dave", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
    ]);
    run_turn(h.as_turn(&create_model, "Working on zephyr, authored by Dave", 8))
        .await
        .unwrap();

    // Now attempt a `:link` call directly — it must fail because `:link` is not installed when
    // linking is disabled. The block terminates with Luau's absent-method call error, which names
    // the missing method ("attempt to call missing method 'link' of table").
    let link_outcome = h
        .run(r#"memory.get("topic/zephyr"):link("authored_by", memory.get(PERSON_DAVE))"#)
        .await;
    match link_outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => {
            assert!(
                message.contains("'link'") && message.contains("missing method"),
                "a disabled :link should surface an absent-method call error, got: {message}"
            );
        }
        other => panic!("expected the :link call to terminate, got {other:?}"),
    }

    // The link was not created by the agent (the block terminated before it). Now drive the
    // link-inference pass, which runs as a model call — it should still create the authored_by link.
    let inference_model = ScriptedModel::new([
        synthesize_call(SynthesizeReply::description("The zephyr project.")),
        link_inference_call(LinkInferenceArgs {
            new_relations: vec![NewRelationSpec {
                name: "authored_by".to_owned(),
                inverse: "authored".to_owned(),
                from_card: "many".to_owned(),
                to_card: "one".to_owned(),
                symmetric: false,
                reflexive: false,
                description: String::new(),
            }],
            links: vec![InferredLink {
                entry: 1,
                relation: "authored_by".to_owned(),
                target: "person/dave".to_owned(),
                direction: "to".to_owned(),
            }],
        }),
    ]);
    h.describe(&inference_model).await;
    h.link_inference(&inference_model).await;

    let events = h.events();
    let linked = events.iter().any(|e| {
        matches!(
            &e.payload,
            EventPayload::LinkCreated { relation, source, .. }
            if relation.as_str() == "authored_by"
              && *source == zuihitsu::LinkSource::Inferred
        )
    });
    assert!(
        linked,
        "the inference pass should still create the authored_by link when linking is disabled"
    );
}
