use zuihitsu::ids::DIRECT_PLATFORM;

use super::*;

/// A person stub, materialized so the resolver can render the speaker's conversational handle.
pub(crate) fn person(store: &mut MemoryStore, clock: &ManualClock, handle: &str) -> MemoryId {
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
pub(crate) fn session_started(
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
pub(crate) fn participant_joined(
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

pub(crate) fn turn_event(
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
pub(crate) fn resolver_context(present_set: Vec<MemoryId>) -> BlockContext {
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
pub(crate) fn resolver_engine(store: MemoryStore, clock: &ManualClock) -> Arc<Engine> {
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
        &ConversationLocator::new(DIRECT_PLATFORM, "maya"),
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
