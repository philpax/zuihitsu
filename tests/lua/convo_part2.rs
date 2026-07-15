use super::*;
#[tokio::test]
async fn convo_turn_two_person_dm_resolves_only_when_both_attended() {
    // A group-room moment attended by Alice and Bob (not Carol). A two-person DM of Alice+Bob resolves
    // it; a two-person DM of Alice+Carol does not — Carol was not in that moment's audience.
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
    let graph = Graph::open_in_memory().unwrap();
    let room = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &graph,
        &ConversationLocator::new(TEST_PLATFORM, "leads"),
    )
    .unwrap();
    let alice = person(&mut store, &clock, "alice");
    let bob = person(&mut store, &clock, "bob");
    let carol = person(&mut store, &clock, "carol");

    let room_session = SessionId::generate();
    let moment = TurnId::generate();
    store
        .append(
            clock.now(),
            EventSource::Agent,
            vec![
                session_started(room, room_session, vec![alice, bob], clock.now()),
                turn_event(
                    room,
                    moment,
                    TurnRole::Participant,
                    "Budget sign-off is with finance until Thursday.",
                    Some(alice),
                ),
            ],
        )
        .unwrap();

    let session_vm = Session::new(room, InstanceFeatures::default());
    let engine = resolver_engine(store, &clock);

    // Both attended — resolves.
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(vec![alice, bob]),
            &format!(r#"return convo.turn("{}")"#, moment.0),
        )
        .await
        .unwrap();
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.contains("Budget sign-off is with finance until Thursday."),
        "{result}"
    );

    // Carol was not in that moment's audience — the same id warns.
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(vec![alice, carol]),
            &format!(r#"return convo.turn("{}")"#, moment.0),
        )
        .await
        .unwrap();
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected an audience-mismatch warning, got {outcome:?}");
    };
    assert!(message.contains("audience"), "{message}");
}

#[tokio::test]
async fn convo_turn_unknown_and_malformed_ids_are_distinct_errors() {
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
    let conversation = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &Graph::open_in_memory().unwrap(),
        &ConversationLocator::new(TEST_PLATFORM, "planning"),
    )
    .unwrap();

    let session_vm = Session::new(conversation, InstanceFeatures::default());
    let engine = resolver_engine(store, &clock);

    // A well-formed but never-recorded id is not-found — worded distinctly from the audience warning.
    let unknown = TurnId::generate();
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(Vec::new()),
            &format!(r#"return convo.turn("{}")"#, unknown.0),
        )
        .await
        .unwrap();
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(message.contains("no turn"), "{message}");
    assert!(!message.contains("audience"), "{message}");

    // A malformed id is teachable and distinctly worded again.
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(Vec::new()),
            r#"return convo.turn("not-a-ulid")"#,
        )
        .await
        .unwrap();
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a teachable error, got {outcome:?}");
    };
    assert!(message.contains("invalid turn id"), "{message}");
}

#[tokio::test]
async fn convo_turn_window_filters_a_mid_session_join() {
    // Maya and Tom open a session; a turn is recorded; then Sam joins mid-session and another turn
    // lands. Resolving the post-join turn while Maya and Sam are present drops the pre-join neighbor
    // from the window — Sam was not in the audience of that earlier turn.
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
    let conversation = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &Graph::open_in_memory().unwrap(),
        &ConversationLocator::new(TEST_PLATFORM, "leads"),
    )
    .unwrap();
    let maya = person(&mut store, &clock, "maya");
    let tom = person(&mut store, &clock, "tom");
    let sam = person(&mut store, &clock, "sam");

    let session = SessionId::generate();
    let pre_join = TurnId::generate();
    let post_join = TurnId::generate();
    store
        .append(
            clock.now(),
            EventSource::Agent,
            vec![
                session_started(conversation, session, vec![maya, tom], clock.now()),
                turn_event(
                    conversation,
                    pre_join,
                    TurnRole::Participant,
                    "Only the two of us know about the reorg.",
                    Some(tom),
                ),
                participant_joined(conversation, session, sam),
                turn_event(
                    conversation,
                    post_join,
                    TurnRole::Participant,
                    "Welcome Sam, glad you could join.",
                    Some(maya),
                ),
            ],
        )
        .unwrap();

    let session_vm = Session::new(conversation, InstanceFeatures::default());
    let engine = resolver_engine(store, &clock);
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(vec![maya, sam]),
            &format!(r#"return convo.turn("{}")"#, post_join.0),
        )
        .await
        .unwrap();
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(result.contains("Welcome Sam"), "{result}");
    assert!(
        !result.contains("reorg"),
        "the pre-join neighbor is filtered from the window: {result}"
    );
}

#[tokio::test]
async fn convo_turn_is_absent_when_transcripts_are_disabled() {
    let disabled = InstanceFeatures {
        transcripts: false,
        ..Default::default()
    };
    let h = Harness::with_features(disabled);
    let outcome = h
        .run(&format!(r#"return convo.turn("{}")"#, TurnId::generate().0))
        .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a nil-call error, got {outcome:?}");
    };
    assert!(
        message.contains("nil"),
        "a disabled convo.turn should surface a nil-call error, got: {message}"
    );
}
