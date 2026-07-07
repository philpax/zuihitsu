use super::{TurnResolution, recording::reply_leaks_special_tokens, resolve_turn};
use crate::{
    clock::ManualClock,
    engine::Engine,
    event::{Cardinality, EventPayload, Initiation, LinkSource, TurnRole},
    graph::Graph,
    ids::{ConversationId, MemoryId, Namespace, SessionId, TurnId},
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::RelationName,
};

/// A single-participant discord session in which `maya@discord` records one turn — the group-room
/// moment a later reference points back to. Optionally operator-merges `maya@direct` into the same
/// `same_as` class, mirroring how the console confirms a cross-platform identity. Returns the
/// booted engine, the direct stub's id (the requester in a solo DM), and the recorded turn's id.
fn discord_moment(merge_direct: bool) -> (std::sync::Arc<Engine>, MemoryId, TurnId) {
    let conversation = ConversationId::generate();
    let session = SessionId::generate();
    let turn_id = TurnId::generate();
    let discord = MemoryId::generate();
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
        EventPayload::memory_created(discord, Namespace::Person.with_name("maya@discord")),
        EventPayload::memory_created(direct, Namespace::Person.with_name("maya@direct")),
        EventPayload::session_started(
            conversation,
            session,
            vec![discord],
            Timestamp::from_millis(1_000),
            None,
            "",
        ),
        EventPayload::conversation_turn(
            conversation,
            turn_id,
            TurnRole::Participant,
            "we're standardizing on Postgres",
            Some(discord),
            Initiation::Responding,
            None,
        ),
    ];
    if merge_direct {
        events.push(EventPayload::LinkCreated {
            from: direct,
            to: discord,
            relation: RelationName::SameAs,
            source: LinkSource::Operator,
            told_by: None,
        });
    }

    let mut store = MemoryStore::new();
    store.append(Timestamp::from_millis(1_000), events).unwrap();
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
    // maya's direct stub, operator-merged with her discord stub, is present in a solo DM. She
    // attended the discord room only under the discord stub, but the merge makes the two one
    // person, so the audience rule admits her and the moment resolves.
    let (engine, direct, turn_id) = discord_moment(true);
    let resolution = resolve_turn(&engine, &[direct], turn_id, 2, 2).unwrap();
    assert!(matches!(resolution, TurnResolution::Resolved(_)));
}

#[test]
fn an_unmerged_direct_stub_is_refused_as_a_different_person() {
    // Without the merge, the direct stub is a distinct identity that was never in the room's
    // audience, so the same lookup refuses — the raw-id behavior the class rule falls back to.
    let (engine, direct, turn_id) = discord_moment(false);
    let resolution = resolve_turn(&engine, &[direct], turn_id, 2, 2).unwrap();
    assert!(matches!(resolution, TurnResolution::AudienceMismatch));
}
