use crate::{
    agent::turn::{
        TurnResolution, participant_names, recording::reply_leaks_special_tokens, resolve_turn,
    },
    clock::ManualClock,
    engine::Engine,
    event::{
        Cardinality, EventPayload, EventSource, Initiation, LinkPosture, LinkSource, ProducedBy,
        PromptTemplateName, TurnRole, Visibility,
    },
    graph::Graph,
    ids::{ConversationId, MemoryId, Namespace, Seq, SessionId, TurnId},
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::RelationName,
};

/// A single-participant chat session in which `maya@chat` records one turn — the group-room
/// moment a later reference points back to. Optionally operator-merges `maya@direct` into the same
/// `same_as` class, mirroring how the console confirms a cross-platform identity. Returns the
/// booted engine, the direct stub's id (the requester in a solo DM), and the recorded turn's id.
fn chat_moment(merge_direct: bool) -> (std::sync::Arc<Engine>, MemoryId, TurnId) {
    let conversation = ConversationId::generate();
    let session = SessionId::generate();
    let turn_id = TurnId::generate();
    let chat = MemoryId::generate();
    let direct = MemoryId::generate();

    let mut events = vec![
        EventPayload::LinkTypeRegistered {
            name: RelationName::SameAs,
            inverse: RelationName::SameAs,
            from_card: Cardinality::Many,
            to_card: Cardinality::Many,
            symmetric: true,
            reflexive: false,
            description: String::new(),
        },
        EventPayload::memory_created(chat, Namespace::Person.with_name("maya@chat")),
        EventPayload::memory_created(direct, Namespace::Person.with_name("maya@direct")),
        EventPayload::session_started(
            conversation,
            session,
            vec![chat],
            Timestamp::from_millis(1_000),
            None,
            "",
        ),
        EventPayload::conversation_turn(
            conversation,
            turn_id,
            TurnRole::Participant,
            "we're standardizing on Postgres",
            Some(chat),
            Initiation::Responding,
            None,
        ),
    ];
    if merge_direct {
        events.push(EventPayload::link_created(
            direct,
            chat,
            RelationName::SameAs,
            LinkPosture {
                source: LinkSource::Operator,
                told_by: None,
                told_in: None,
                visibility: Visibility::Public,
            },
        ));
    }

    let mut store = MemoryStore::new();
    store
        .append(Timestamp::from_millis(1_000), EventSource::Agent, events)
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    let engine = Engine::new(
        Box::new(store),
        graph,
        Box::new(ManualClock::new(Timestamp::from_millis(2_000))),
    );
    (engine, direct, turn_id)
}

#[test]
fn special_token_markup_is_flagged_and_ordinary_text_is_not() {
    // The observed leak: a pseudo-tool-call transcribed with `<|`/`|>` special-token delimiters.
    assert!(reply_leaks_special_tokens(
        "<|tool_call>call:run_lua{script:<|\"|>memory.search(\"decided\")<|\"|>}<tool_call|>"
    ));
    // A normal reply is plain prose — clean.
    assert!(!reply_leaks_special_tokens(
        "Noted — I'll remember that you're standardizing on Postgres."
    ));
    // A reply quoting Lua with `..` concatenation and `{}` table syntax — clean.
    assert!(!reply_leaks_special_tokens(
        "Run `local t = { a = 1 }; return t.a .. \"!\"` to see it."
    ));
    // A comparison with `<`, `>`, and `||` but no adjacent `<|`/`|>` — clean.
    assert!(!reply_leaks_special_tokens(
        "guard against a < b || b > c here"
    ));
    // The delimiter proper: `<|` (and by symmetry `|>`) is flagged. The `<|` operator does not
    // occur in prose, so flagging `x <| y` is acceptable.
    assert!(reply_leaks_special_tokens("x <| y"));
    assert!(reply_leaks_special_tokens("x |> y"));
}

#[test]
fn a_merged_identity_resolves_a_turn_recorded_under_the_other_stub() {
    // maya's direct stub, operator-merged with her chat stub, is present in a solo DM. She
    // attended the chat room only under the chat stub, but the merge makes the two one
    // person, so the audience rule admits her and the moment resolves.
    let (engine, direct, turn_id) = chat_moment(true);
    let resolution = resolve_turn(&engine, &[direct], turn_id, 2, 2).unwrap();
    assert!(matches!(resolution, TurnResolution::Resolved(_)));
}

#[test]
fn an_unmerged_direct_stub_is_refused_as_a_different_person() {
    // Without the merge, the direct stub is a distinct identity that was never in the room's
    // audience, so the same lookup refuses — the raw-id behavior the class rule falls back to.
    let (engine, direct, turn_id) = chat_moment(false);
    let resolution = resolve_turn(&engine, &[direct], turn_id, 2, 2).unwrap();
    assert!(matches!(resolution, TurnResolution::AudienceMismatch));
}

#[test]
fn a_nonempty_flush_reply_is_marked_undelivered_in_the_buffer() {
    // A checkpoint flush that misbehaved and answered conversationally leaves a non-empty agent turn
    // in the log; on replay it must be marked as an internal note that reached no participant, while
    // an ordinary reply and a well-behaved empty flush stay unmarked.
    let conversation = ConversationId::generate();
    let session = SessionId::generate();
    let flush_provenance = Some(ProducedBy {
        model_id: "test-model".into(),
        template_name: PromptTemplateName::Flush,
        template_version: 5,
    });
    let flush_turn = TurnId::generate();
    let reply_turn = TurnId::generate();
    let empty_flush_turn = TurnId::generate();

    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![
                EventPayload::session_started(
                    conversation,
                    session,
                    vec![],
                    Timestamp::from_millis(1_000),
                    None,
                    "",
                ),
                EventPayload::conversation_turn(
                    conversation,
                    TurnId::generate(),
                    TurnRole::Participant,
                    "what's 2+2?",
                    None,
                    Initiation::Responding,
                    None,
                ),
                EventPayload::conversation_turn(
                    conversation,
                    flush_turn,
                    TurnRole::Agent,
                    "It's 4!",
                    None,
                    Initiation::Initiated,
                    flush_provenance.clone(),
                ),
                EventPayload::conversation_turn(
                    conversation,
                    reply_turn,
                    TurnRole::Agent,
                    "noted",
                    None,
                    Initiation::Responding,
                    None,
                ),
                EventPayload::conversation_turn(
                    conversation,
                    empty_flush_turn,
                    TurnRole::Agent,
                    "",
                    None,
                    Initiation::Initiated,
                    flush_provenance,
                ),
            ],
        )
        .unwrap();

    let buffer = crate::agent::buffer_turns(&store, conversation, Seq::ZERO).unwrap();

    // The non-empty flush reply is immediately followed by a system marker naming it undelivered.
    let flush_idx = buffer
        .iter()
        .position(|turn| turn.turn_id == flush_turn && turn.role == TurnRole::Agent)
        .expect("the flush reply is in the buffer");
    let marker = &buffer[flush_idx + 1];
    assert_eq!(marker.role, TurnRole::System);
    assert!(
        marker.text.contains("not delivered to any participant"),
        "the flush reply is marked undelivered: {:?}",
        marker.text,
    );

    // Exactly one marker exists — neither the ordinary reply nor the empty flush contributes one.
    let markers = buffer
        .iter()
        .filter(|turn| {
            turn.role == TurnRole::System && turn.text.contains("internal checkpoint note")
        })
        .count();
    assert_eq!(markers, 1, "only the non-empty flush reply is marked");

    // The ordinary agent reply is not marked.
    let reply_idx = buffer
        .iter()
        .position(|turn| turn.turn_id == reply_turn && turn.role == TurnRole::Agent)
        .expect("the ordinary reply is in the buffer");
    assert!(
        buffer
            .get(reply_idx + 1)
            .is_none_or(|turn| turn.role != TurnRole::System
                || !turn.text.contains("internal checkpoint note")),
        "an ordinary agent reply must not be marked undelivered",
    );
}

#[test]
fn a_participant_label_is_the_canonical_class_primary_handle() {
    // A platform stub bound `same_as` a designated canonical primary stamps the primary's complete
    // handle, so the speaker label is a directly usable `memory.get` operand rather than an opaque
    // snowflake; a classless participant stamps its own full handle, still a valid operand.
    let primary = MemoryId::generate();
    let stub = MemoryId::generate();
    let classless = MemoryId::generate();

    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![
                EventPayload::LinkTypeRegistered {
                    name: RelationName::SameAs,
                    inverse: RelationName::SameAs,
                    from_card: Cardinality::Many,
                    to_card: Cardinality::Many,
                    symmetric: true,
                    reflexive: false,
                    description: String::new(),
                },
                EventPayload::memory_created(primary, Namespace::Person.with_name("rowan")),
                EventPayload::memory_created(
                    stub,
                    Namespace::Person.with_name("201689218030895104@discord"),
                ),
                EventPayload::memory_created(classless, Namespace::Person.with_name("wren@direct")),
                EventPayload::link_created(
                    stub,
                    primary,
                    RelationName::SameAs,
                    LinkPosture {
                        source: LinkSource::Operator,
                        told_by: None,
                        told_in: None,
                        visibility: Visibility::Public,
                    },
                ),
                EventPayload::class_primary_designated(primary, true),
            ],
        )
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    let engine = Engine::new(
        Box::new(store),
        graph,
        Box::new(ManualClock::new(Timestamp::from_millis(2_000))),
    );

    let names = participant_names(&engine, &[], &[stub, classless]);
    // The stub canonicalizes to its class primary and renders the primary's complete handle.
    assert_eq!(names.get(&stub).map(String::as_str), Some("person/rowan"));
    // A classless participant renders its own full handle, no prefix or platform stripping.
    assert_eq!(
        names.get(&classless).map(String::as_str),
        Some("person/wren@direct"),
    );
}

/// A `LuaExecuted` event for `conversation` that touched `touched`, the shape [`recent_touched`]
/// scans for the cold-open working set.
fn lua_touch(conversation: ConversationId, touched: Vec<MemoryId>) -> EventPayload {
    EventPayload::LuaExecuted {
        conversation,
        turn_id: TurnId::generate(),
        script: "-- touch".to_owned(),
        result: None,
        touched,
        terminal_cause: None,
        duration_ms: 0,
    }
}

#[test]
fn recent_touched_ranks_recent_first_deduped_and_capped() {
    // Three blocks across two conversations touch overlapping memories at rising times. The cold-open
    // set is most-recent-first (the freshest thread leads so it survives the brief's budget), each id
    // once, capped at the limit.
    let room_a = ConversationId::generate();
    let room_b = ConversationId::generate();
    let (a, b, c, d) = (
        MemoryId::generate(),
        MemoryId::generate(),
        MemoryId::generate(),
        MemoryId::generate(),
    );
    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![lua_touch(room_a, vec![a, b])],
        )
        .unwrap();
    store
        .append(
            Timestamp::from_millis(2_000),
            EventSource::Agent,
            vec![lua_touch(room_b, vec![c])],
        )
        .unwrap();
    // The newest block re-touches `b` (already seen) and adds `d`.
    store
        .append(
            Timestamp::from_millis(3_000),
            EventSource::Agent,
            vec![lua_touch(room_a, vec![d, b])],
        )
        .unwrap();

    let all = crate::agent::recent_touched(&store, Timestamp::from_millis(0), 10).unwrap();
    assert_eq!(all, vec![d, b, c, a]);

    let capped = crate::agent::recent_touched(&store, Timestamp::from_millis(0), 2).unwrap();
    assert_eq!(capped, vec![d, b]);
}

#[test]
fn recent_touched_excludes_blocks_before_the_cutoff() {
    // A block older than `since` never contributes a candidate — the recency window is the guard
    // against a long-dead thread re-surfacing on a cold open.
    let room = ConversationId::generate();
    let (old, fresh) = (MemoryId::generate(), MemoryId::generate());
    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![lua_touch(room, vec![old])],
        )
        .unwrap();
    store
        .append(
            Timestamp::from_millis(5_000),
            EventSource::Agent,
            vec![lua_touch(room, vec![fresh])],
        )
        .unwrap();

    let within = crate::agent::recent_touched(&store, Timestamp::from_millis(4_000), 10).unwrap();
    assert_eq!(within, vec![fresh]);
}

#[test]
fn recent_touched_is_empty_for_zero_limit_or_empty_log() {
    let room = ConversationId::generate();
    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![lua_touch(room, vec![MemoryId::generate()])],
        )
        .unwrap();

    // A zero limit disables the cold-open derivation.
    assert!(
        crate::agent::recent_touched(&store, Timestamp::from_millis(0), 0)
            .unwrap()
            .is_empty()
    );
    // An empty log yields nothing.
    let empty = MemoryStore::new();
    assert!(
        crate::agent::recent_touched(&empty, Timestamp::from_millis(0), 10)
            .unwrap()
            .is_empty()
    );
}
