use crate::{
    Arc, Authority, BlockContext, BlockOutcome, Clock, ConversationLocator, ConversationRef,
    Engine, EventPayload, EventSource, Graph, Harness, InstanceFeatures, MILLIS_PER_DAY,
    ManualClock, MemoryId, MemoryName, MemoryStore, Namespace, STALE_HIGH_DAYS, Session, Store,
    TEST_BLOCK_TIMEOUT, TEST_MAX_BLOCK_ATTEMPTS, TEST_MAX_ENTRY_CHARS, TEST_PLATFORM, TagName,
    Teller, TerminalCause, TurnId, Visibility, common, resolve_or_mint_conversation,
};

#[tokio::test]
async fn append_carries_teller_context_and_default_visibility() {
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
    let mut graph = Graph::open_in_memory().unwrap();

    // A room (with its eagerly-minted context memory), the subject, and the speaker.
    let conversation = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &graph,
        &ConversationLocator::new(TEST_PLATFORM, "leads"),
    )
    .unwrap();
    let marcus = MemoryId::generate();
    let erin = MemoryId::generate();
    store
        .append(
            clock.now(),
            EventSource::Agent,
            vec![
                EventPayload::memory_created(marcus, Namespace::Person.with_name("marcus")),
                EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
            ],
        )
        .unwrap();
    graph.materialize_from(&store).unwrap();
    let context = graph
        .context_for_conversation(conversation)
        .unwrap()
        .unwrap();
    let session = Session::new(Some(conversation), InstanceFeatures::default());

    // The shared engine the block writes through, read back below via the same handle.
    let engine = Engine::new(Box::new(store), graph, Box::new(clock.clone()));
    async fn exec(session: &Session, engine: &Arc<Engine>, teller: MemoryId, script: &str) {
        session
            .execute(
                engine,
                &BlockContext {
                    teller: Teller::Participant(teller),
                    authority: Authority::Platform,
                    turn_id: TurnId::generate(),
                    block_timeout: TEST_BLOCK_TIMEOUT,
                    max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
                    max_entry_chars: TEST_MAX_ENTRY_CHARS,
                    present_set: Vec::new(),
                    dry_run: false,
                },
                &common::prepare_script(script),
            )
            .await
            .unwrap();
    }

    // Erin, in the room, relays something about Marcus: attributed to her, told in this context, and
    // defaulted private to its teller because the subject (Marcus) is not the teller.
    exec(
        &session,
        &engine,
        erin,
        r#"memory.get(PERSON_MARCUS):append("is being managed out")"#,
    )
    .await;
    // `by_agent` records the agent's own observation about a person, which has no protective default
    // (the aside mechanism keys on a participant teller) — so it must classify the entry explicitly.
    exec(
        &session,
        &engine,
        erin,
        r#"memory.get(PERSON_MARCUS):append("seems stressed", { by_agent = true, visibility = "public" })"#,
    )
    .await;
    exec(
        &session,
        &engine,
        erin,
        r#"memory.get(PERSON_MARCUS):append("got promoted", { visibility = "public" })"#,
    )
    .await;

    let entries = engine.graph.lock().entries_local(marcus).unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].told_by, Teller::Participant(erin));
    assert!(matches!(
        entries[0].told_in,
        Some(ConversationRef { turn: Some(_), .. })
    ));
    assert_eq!(entries[0].visibility, Visibility::PrivateToTeller);
    assert_eq!(entries[1].told_by, Teller::Agent);
    assert_eq!(entries[1].visibility, Visibility::Public);
    assert_eq!(entries[2].told_by, Teller::Participant(erin));
    assert_eq!(entries[2].visibility, Visibility::Public); // forced, despite the subject mismatch

    // context.current() resolves to this room's context memory.
    exec(
        &session,
        &engine,
        erin,
        r#"context.current():append("kept in confidence", { by_agent = true })"#,
    )
    .await;
    let context_entries = engine.graph.lock().entries_local(context).unwrap();
    assert_eq!(context_entries.len(), 1);
    assert_eq!(context_entries[0].text, "kept in confidence");
}

#[tokio::test]
async fn told_by_stamps_a_relayed_claims_source() {
    // told_by attributes an entry to a teller other than the current speaker: Erin, speaking, relays
    // something Marcus is the source of about Dave. The fact is stamped with Marcus as its provenance,
    // not Erin who relayed it, so it reads and is governed as Marcus's. Both a name string and a handle
    // resolve to the same teller.
    let h = Harness::new();
    h.run(
        r#"memory.create(PERSON_DAVE); memory.create(PERSON_ERIN); memory.create(PERSON_MARCUS)"#,
    )
    .await;
    let id = |name: &str| {
        h.engine
            .graph
            .lock()
            .memory_by_name(MemoryName::new(name))
            .unwrap()
            .unwrap()
            .id
    };
    let (dave, erin, marcus) = (
        id(MemoryName::from(Namespace::Person.with_name("dave")).as_str()),
        id(MemoryName::from(Namespace::Person.with_name("erin")).as_str()),
        id(MemoryName::from(Namespace::Person.with_name("marcus")).as_str()),
    );

    // Erin is the speaker; the first append names Marcus by name string, the second by handle.
    h.run_as(
        Teller::Participant(erin),
        vec![erin],
        r#"
        memory.get(PERSON_DAVE):append("is moving to the Atlas team", { visibility = "public", told_by = PERSON_MARCUS })
        memory.get(PERSON_DAVE):append("prefers async standups", { visibility = "public", told_by = memory.get(PERSON_MARCUS) })
        "#,
    )
    .await;

    let entries = h.engine.graph.lock().entries_local(dave).unwrap();
    assert_eq!(entries.len(), 2);
    // Both are stamped with Marcus (the source), not Erin (the speaker) — via name and via handle.
    assert_eq!(entries[0].told_by, Teller::Participant(marcus));
    assert_eq!(entries[1].told_by, Teller::Participant(marcus));
    assert_ne!(entries[0].told_by, Teller::Participant(erin));
}

#[tokio::test]
async fn told_by_an_unknown_name_is_a_teachable_error() {
    // told_by names a teller that must exist: an unknown name is a teachable error naming the option,
    // not a silent attribution to a nonexistent memory.
    let h = Harness::new();
    h.run(r#"memory.create(PERSON_DAVE)"#).await;
    let outcome = h
        .run(
            r#"memory.get(PERSON_DAVE):append("relayed", { visibility = "public", told_by = PERSON_NOBODY })"#,
        )
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(
        message.contains("told_by") && message.contains("person/nobody"),
        "the error should name told_by and the unknown teller: {message}"
    );
}

#[tokio::test]
async fn a_write_in_a_confidential_room_defaults_private() {
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
    let mut graph = Graph::open_in_memory().unwrap();

    let conversation = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &graph,
        &ConversationLocator::new(TEST_PLATFORM, "leads"),
    )
    .unwrap();
    graph.materialize_from(&store).unwrap();
    let context = graph
        .context_for_conversation(conversation)
        .unwrap()
        .unwrap();
    // Mark the room #confidential.
    store
        .append(
            clock.now(),
            EventSource::Agent,
            vec![
                EventPayload::tag_created(
                    TagName::new("confidential"),
                    "a confidential room".to_owned(),
                ),
                EventPayload::tag_applied_to_memory(context, TagName::new("confidential")),
            ],
        )
        .unwrap();
    graph.materialize_from(&store).unwrap();

    // The agent records a topic in the confidential room. A topic write would normally default
    // public, and the agent teller is always present — but the confidential room forces it private,
    // so it cannot silently surface to whoever is around.
    let session = Session::new(Some(conversation), InstanceFeatures::default());
    let engine = Engine::new(Box::new(store), graph, Box::new(clock.clone()));
    session
        .execute(
            &engine,
            &BlockContext {
                teller: Teller::Agent,
                authority: Authority::Platform,
                turn_id: TurnId::generate(),
                block_timeout: TEST_BLOCK_TIMEOUT,
                max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
                max_entry_chars: TEST_MAX_ENTRY_CHARS,
                present_set: Vec::new(),
                dry_run: false,
            },
            &common::prepare_script(
                r#"memory.create(TOPIC_SENSITIVE, "something said in confidence")"#,
            ),
        )
        .await
        .unwrap();

    let topic = engine
        .graph
        .lock()
        .memory_by_name(Namespace::Topic.with_name("sensitive"))
        .unwrap()
        .unwrap();
    let entries = engine.graph.lock().entries_local(topic.id).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].visibility, Visibility::PrivateToTeller);
}

#[tokio::test]
async fn a_high_volatility_fact_reads_stale_after_aging() {
    let h = Harness::new();
    h.run(
        r#"
        local d = memory.create(PERSON_DAVE)
        -- Classify volatility inline on the append (the ergonomic path).
        d:append("leads the Atlas project", { visibility = "public", volatility = "high" })
        local p = memory.create("project/atlas")
        p:append("the Atlas project ships in Q3", { visibility = "public" })
        "#,
    )
    .await;
    // Age past the staleness horizon (STALE_HIGH_DAYS + a buffer) so the High-volatility entry reads
    // as stale.
    h.clock
        .advance_millis((STALE_HIGH_DAYS as i64 + 10) * MILLIS_PER_DAY);

    let read = r#"
        local e = memory.get("MEM"):entries()[1]
        return tostring(e.stale) .. "|" .. tostring(e)
    "#;
    let BlockOutcome::Committed { result } = h
        .run(&read.replace(
            "MEM",
            MemoryName::from(Namespace::Person.with_name("dave")).as_str(),
        ))
        .await
    else {
        panic!("expected commit");
    };
    assert!(
        result.starts_with("true|") && result.contains("stale — no newer entry"),
        "the aged high-volatility fact should read `stale — no newer entry`: {result}"
    );
    let BlockOutcome::Committed { result } = h.run(&read.replace("MEM", "project/atlas")).await
    else {
        panic!("expected commit");
    };
    assert!(
        result.starts_with("false|"),
        "a default-volatility fact never goes stale: {result}"
    );
}
