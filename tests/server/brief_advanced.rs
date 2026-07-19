use super::*;
#[tokio::test]
async fn a_fresh_genesis_describes_nothing_on_the_first_tick() {
    // Genesis baselines the seeded `self` as already described, so the first describe pass over a
    // fresh agent regenerates nothing and calls the model zero times.
    let (server, _clock) = born_agent();
    let model = DispatchingModel::new([]);
    let considered = server.describe_catch_up(&model).await.unwrap();
    assert_eq!(considered, 0, "nothing is stale after a fresh genesis");
    assert!(
        model.synthesized().is_empty(),
        "the describer made no synthesis calls: {:?}",
        model.synthesized()
    );
}

#[tokio::test]
async fn a_describe_backlog_survives_a_restart() {
    // A memory written but not yet described before shutdown stays stale in the log-derived
    // described-state, so after a rebuild the background describer picks it up — the backlog is not
    // silently dropped at boot.
    let clock = ManualClock::new(test_now());
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");

    // First process: a turn writes a topic that the pre-brief pass does not describe (it is not in the
    // brief's read set), so it is left stale when the process ends. Its whole log is snapshotted before
    // the instance drops.
    let log = {
        let mut server = Server::new(
            Box::new(MemoryStore::new()),
            Graph::open_in_memory().unwrap(),
            Box::new(clock.clone()),
        );
        server.boot().unwrap();
        server.control().create_agent(&seed()).unwrap();
        let model = DispatchingModel::new([
            run_lua_call(
                r#"local m = memory.create("topic/backlog")
                   m:append("A durable fact left undescribed", { by_agent = true, visibility = "public" })"#,
            ),
            Completion::Reply("ok".to_owned()),
        ]);
        server
            .platform()
            .route_message(
                &model,
                &leads,
                &PersonId::new(TEST_PLATFORM, "dave"),
                "note this",
                &[PersonId::new(TEST_PLATFORM, "dave")],
            )
            .await
            .unwrap();
        assert!(
            !model
                .synthesized()
                .iter()
                .any(|name| name == "topic/backlog"),
            "the pre-brief pass left the topic undescribed: {:?}",
            model.synthesized()
        );
        server.control().events().unwrap()
    }; // the server drops: a restart — only the log survives, carried in memory

    // Second process: a fresh graph rebuilt from the same log (carried in memory, no temp file). The
    // backlog persists, so the describer catches it up.
    let mut server = Server::new(
        Box::new(MemoryStore::from_events(log)),
        Graph::open_in_memory().unwrap(),
        Box::new(clock.clone()),
    );
    server.boot().unwrap();
    let model = DispatchingModel::new([]);
    let considered = server.describe_catch_up(&model).await.unwrap();
    assert!(
        considered >= 1,
        "the pre-shutdown backlog is described after a restart"
    );
    assert!(
        model
            .synthesized()
            .iter()
            .any(|name| name == "topic/backlog"),
        "the restarted describer picks up the undescribed topic: {:?}",
        model.synthesized()
    );
}

#[tokio::test]
async fn the_buffer_stays_bounded_across_repeated_compactions() {
    // The compaction-seam bug (issue #22): when the turns are small relative to the carryover char
    // budget, the pre-fix carryover tail never trimmed and `from_seq` never advanced, so the live
    // buffer re-spanned every session since the original carryover point — growing without bound. Here
    // the token budget forces a compaction on every message and the char budget is loose (its default),
    // exactly the condition that stuck `from_seq`; the buffer must stay bounded regardless.
    let (server, _clock) = born_agent();
    let mut settings = server.control().settings().unwrap();
    settings.compaction.token_budget = 100;
    // Disable the pre-compaction flush, so each message is a single scripted model step — the buffer
    // growth is isolated from flush turns.
    settings.compaction.flush_min_turns = 1_000_000;
    // A loose char budget (the default) that small turns never fill — the pre-fix stuck condition.
    settings.compaction.carryover_char_budget = 4_000;
    server.control().set_settings(settings).unwrap();

    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");
    let seams = 8;
    // Every step reports usage over the budget, so every message forces a re-segment.
    let model = ScriptedModel::with_usage(
        (0..seams).map(|i| (Completion::Reply(format!("reply {i}")), 200u32)),
    );

    for i in 0..seams {
        server
            .platform()
            .route_message(
                &model,
                &leads,
                &PersonId::new(TEST_PLATFORM, "dave"),
                &format!("message {i}"),
                &[PersonId::new(TEST_PLATFORM, "dave")],
            )
            .await
            .unwrap();
    }

    // Every message re-segmented: one session per message.
    assert_eq!(
        server.control().sessions(&leads).unwrap().len(),
        seams as usize
    );

    let seen = model.recorded_messages();
    assert_eq!(seen.len(), seams as usize);

    // The buffer is bounded: a seeded session sees only the prior session's carried tail plus its own
    // inbound. It must not grow with the number of seams — this bound (four) holds regardless of how
    // many seams precede the session, rather than growing by two messages each seam.
    for (turn_index, messages) in seen.iter().enumerate() {
        assert!(
            messages.len() <= 4,
            "turn {turn_index} buffer grew to {} messages: {:?}",
            messages.len(),
            messages
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>(),
        );
    }

    // From the second seam on, the count is steady, not climbing — the bound is a plateau, not a slower
    // leak.
    let steady = seen[2].len();
    for messages in &seen[2..] {
        assert_eq!(messages.len(), steady, "the bounded buffer size is stable");
    }

    // The trim drops the oldest and keeps the newest: the first message is long gone from the last
    // prompt, and the latest inbound is present.
    let last: Vec<&str> = seen
        .last()
        .unwrap()
        .iter()
        .map(|m| m.content.as_str())
        .collect();
    assert!(
        !last.iter().any(|content| content.contains("message 0")),
        "the original first turn should have been trimmed away, but is still present: {last:?}",
    );
    assert!(
        last.iter()
            .any(|content| content.contains(&format!("message {}", seams - 1))),
        "the newest inbound must always be present: {last:?}",
    );

    // The total char size is bounded too — a generous fixed ceiling (the char budget plus a single
    // session's stamped turns), independent of the seam count; the pre-fix buffer blows past it as the
    // seams accrue.
    let last_chars: usize = seen
        .last()
        .unwrap()
        .iter()
        .map(|m| m.content.chars().count())
        .sum();
    assert!(
        last_chars <= 4_000 + 1_000,
        "the last prompt's char size {last_chars} exceeds the bound",
    );
}

#[tokio::test]
async fn an_arrival_matching_an_unbound_stub_binds_to_it() {
    // An agent-authored stub: `person/nadia@chat` exists (written from conversation) but is bound
    // to no platform key — the agent named the qualified handle before nadia ever spoke.
    let (server, _clock) = born_agent();
    let hearsay = MemoryId::generate();
    server
        .control()
        .seed_events(vec![EventPayload::memory_created(
            hearsay,
            Namespace::Person.with_name("nadia@chat"),
        )])
        .unwrap();

    // Nadia then arrives on chat. The qualified name matches the unbound stub, so the arrival binds
    // the platform identity to it — no fresh memory, no merge proposed.
    let model = ScriptedModel::new([Completion::Reply("Hello.".to_owned())]);
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "nadia"),
            "hi there",
            &[PersonId::new(TEST_PLATFORM, "nadia")],
        )
        .await
        .unwrap();

    // Exactly one `person/nadia@chat` memory exists, and it is the pre-existing stub.
    let bound = server
        .control()
        .memory("person/nadia@chat")
        .unwrap()
        .expect("the stub still exists");
    assert_eq!(bound.id, hearsay, "the arrival bound to the existing stub");

    // No merge was proposed — the stub and the arrival are one memory, and nothing awaits the operator.
    assert!(
        !server
            .control()
            .events()
            .unwrap()
            .into_iter()
            .any(|event| matches!(event.payload, EventPayload::MergeProposed { .. })),
        "no merge proposal should be created"
    );
    assert!(server.control().merge_proposals().unwrap().is_empty());
}
