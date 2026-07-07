use super::*;

/// A person stub, materialized so the resolver can render the speaker's conversational handle.
fn person(store: &mut MemoryStore, clock: &ManualClock, handle: &str) -> MemoryId {
    let id = MemoryId::generate();
    store
        .append(
            clock.now(),
            vec![EventPayload::memory_created(
                id,
                Namespace::Person.with_name(handle),
            )],
        )
        .unwrap();
    id
}

/// A `SessionStarted` opening `session` in `conversation` with `participants` as its audience.
fn session_started(
    conversation: ConversationId,
    session: SessionId,
    participants: Vec<MemoryId>,
    started_at: Timestamp,
) -> EventPayload {
    EventPayload::SessionStarted {
        conversation,
        id: session,
        participants,
        started_at,
        seeded_from_turn: None,
        brief: String::new(),
    }
}

/// A `ParticipantJoined` recording `participant` arriving mid-`session`.
fn participant_joined(
    conversation: ConversationId,
    session: SessionId,
    participant: MemoryId,
) -> EventPayload {
    EventPayload::ParticipantJoined {
        conversation,
        session,
        participant,
        at_turn: TurnId::generate(),
    }
}

fn turn_event(
    conversation: ConversationId,
    turn_id: TurnId,
    role: TurnRole,
    text: &str,
    participant: Option<MemoryId>,
) -> EventPayload {
    EventPayload::ConversationTurn {
        conversation,
        turn_id,
        role,
        text: text.to_owned(),
        participant,
        initiation: Initiation::Responding,
        produced_by: None,
        brief: None,
    }
}

/// A block context whose present set drives the audience rule the resolver applies.
fn resolver_context(present_set: Vec<MemoryId>) -> BlockContext {
    BlockContext {
        teller: Teller::Agent,
        authority: Authority::Platform,
        turn_id: TurnId::generate(),
        block_timeout: TEST_BLOCK_TIMEOUT,
        max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
        present_set,
        dry_run: false,
    }
}

/// Boot an engine over a store the caller has appended to, materializing the graph.
fn resolver_engine(store: MemoryStore, clock: &ManualClock) -> Arc<Engine> {
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    Engine::new(Box::new(store), graph, Box::new(clock.clone()))
}

#[tokio::test]
async fn convo_turn_resolves_within_audience_and_carries_a_ref() {
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
    let conversation = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &Graph::open_in_memory().unwrap(),
        &ConversationLocator::new("discord", "planning"),
    )
    .unwrap();
    let sarah = person(&mut store, &clock, "sarah");
    let session = SessionId::generate();

    let before = TurnId::generate();
    let focus = TurnId::generate();
    let after = TurnId::generate();
    store
        .append(
            clock.now(),
            vec![
                session_started(conversation, session, vec![sarah], clock.now()),
                turn_event(
                    conversation,
                    before,
                    TurnRole::Participant,
                    "Kicking off Q3 planning.",
                    Some(sarah),
                ),
                turn_event(
                    conversation,
                    focus,
                    TurnRole::Participant,
                    "We ship Meridian on August 14th.",
                    Some(sarah),
                ),
                turn_event(
                    conversation,
                    after,
                    TurnRole::Agent,
                    "Noted — Meridian on the 14th.",
                    None,
                ),
            ],
        )
        .unwrap();

    let session_vm = Session::new(conversation, InstanceFeatures::default());
    let engine = resolver_engine(store, &clock);

    // Sarah is present, and she was the moment's audience — it resolves with its window.
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(vec![sarah]),
            &format!(r#"return convo.turn("{}")"#, focus.0),
        )
        .await
        .unwrap();
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.contains("We ship Meridian on August 14th."),
        "{result}"
    );
    assert!(result.contains("Kicking off Q3 planning."), "{result}");
    assert!(result.contains("Noted — Meridian on the 14th."), "{result}");
    assert!(result.contains("sarah"), "{result}");

    // The `ref` field is the canonical token, and it round-trips back to the focal id through the one
    // parser — the agent cites by copying it rather than hand-assembling syntax.
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(vec![sarah]),
            &format!(r#"return convo.turn("{}").ref"#, focus.0),
        )
        .await
        .unwrap();
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(turn_ref::extract_ids(&result), vec![focus]);
}

#[tokio::test]
async fn convo_turn_warns_when_a_newcomer_was_not_in_the_audience() {
    // Same room, two sessions: session 1 is Maya and Tom on something sensitive; session 2 is Maya and
    // a newcomer Sam. Resolving the session-1 moment while Sam is present must warn, not replay.
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
    let conversation = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &Graph::open_in_memory().unwrap(),
        &ConversationLocator::new("discord", "leads"),
    )
    .unwrap();
    let maya = person(&mut store, &clock, "maya");
    let tom = person(&mut store, &clock, "tom");
    let sam = person(&mut store, &clock, "sam");

    let session_one = SessionId::generate();
    let sensitive = TurnId::generate();
    store
        .append(
            clock.now(),
            vec![
                session_started(conversation, session_one, vec![maya, tom], clock.now()),
                turn_event(
                    conversation,
                    sensitive,
                    TurnRole::Participant,
                    "The layoffs land Friday — keep it off the record for now.",
                    Some(tom),
                ),
                EventPayload::session_ended(conversation, session_one),
            ],
        )
        .unwrap();
    let session_two = SessionId::generate();
    store
        .append(
            clock.now(),
            vec![session_started(
                conversation,
                session_two,
                vec![maya, sam],
                clock.now(),
            )],
        )
        .unwrap();

    let session_vm = Session::new(conversation, InstanceFeatures::default());
    let engine = resolver_engine(store, &clock);
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(vec![maya, sam]),
            &format!(r#"return convo.turn("{}")"#, sensitive.0),
        )
        .await
        .unwrap();
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected an audience-mismatch warning, got {outcome:?}");
    };
    assert!(message.contains("audience"), "{message}");
    assert!(message.contains("memory"), "{message}");
    // The refusal never carries the withheld content or the id-is-unknown wording.
    assert!(
        !message.contains("layoffs"),
        "must not leak content: {message}"
    );
    assert!(
        !message.contains("no turn"),
        "audience-mismatch is worded distinctly from not-found: {message}"
    );
}

#[tokio::test]
async fn convo_turn_resolves_cross_room_for_a_solo_dm() {
    // A group room the requester attended, and a solo DM with just the requester. The loosening lets
    // the DM resolve the group-room moment the requester was party to.
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(common::time::EARLY);
    let graph = Graph::open_in_memory().unwrap();
    let room = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &graph,
        &ConversationLocator::new("discord", "leads"),
    )
    .unwrap();
    let dm = resolve_or_mint_conversation(
        &mut store,
        &clock,
        &graph,
        &ConversationLocator::new("direct", "maya"),
    )
    .unwrap();
    let maya = person(&mut store, &clock, "maya");
    let tom = person(&mut store, &clock, "tom");

    let room_session = SessionId::generate();
    let moment = TurnId::generate();
    store
        .append(
            clock.now(),
            vec![
                session_started(room, room_session, vec![maya, tom], clock.now()),
                turn_event(
                    room,
                    moment,
                    TurnRole::Participant,
                    "We ship Meridian on August 14th.",
                    Some(maya),
                ),
            ],
        )
        .unwrap();
    // The DM's own session — only Maya present.
    let dm_session = SessionId::generate();
    store
        .append(
            clock.now(),
            vec![session_started(dm, dm_session, vec![maya], clock.now())],
        )
        .unwrap();

    let session_vm = Session::new(dm, InstanceFeatures::default());
    let engine = resolver_engine(store, &clock);
    let outcome = session_vm
        .execute(
            &engine,
            &resolver_context(vec![maya]),
            &format!(r#"return convo.turn("{}")"#, moment.0),
        )
        .await
        .unwrap();
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(
        result.contains("We ship Meridian on August 14th."),
        "{result}"
    );
}

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
        &ConversationLocator::new("discord", "leads"),
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
        &ConversationLocator::new("discord", "planning"),
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
        &ConversationLocator::new("discord", "leads"),
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
