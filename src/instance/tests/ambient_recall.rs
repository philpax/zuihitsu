//! Ambient recall: the pre-turn lexical pass surfaces a memory the frozen brief did not, injected as a
//! system hint and recorded as an `AmbientRecallSurfaced` event that a later turn replays byte for byte.
//! These tests drive the real `route_message` path so the pass runs where it does in production, and
//! check both the live prompt the model saw and the recorded event.

use super::*;
use crate::{
    ConversationLocator, SeedSelf,
    clock::ManualClock,
    event::{EventPayload, EventSource, Teller, Visibility},
    ids::{EntryId, MemoryId, Namespace},
    model::{Completion, Role, ScriptedModel},
    time::Timestamp,
};

/// Boot and birth a fresh in-memory agent, so the genesis templates are registered and turns can run.
fn born_server() -> Instance {
    let server =
        Instance::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(1_000)))).unwrap();
    server
        .control()
        .create_agent(&SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "A discreet companion with a long memory.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    server
}

/// Seed a public topic memory named `name` with one content entry, plus a dozen unrelated topics so
/// the FTS index carries a realistic corpus (bm25 collapses toward zero on a near-empty index, so the
/// salience threshold needs the filler to separate a distinctive match from noise). Returns the seeded
/// memory's id.
fn seed_public_topic(server: &Instance, name: &str, text: &str) -> MemoryId {
    let id = MemoryId::generate();
    let now = Timestamp::from_millis(1_000);
    let mut payloads = vec![
        EventPayload::memory_created(id, Namespace::Topic.with_name(name)),
        EventPayload::MemoryContentAppended {
            id,
            entry_id: EntryId::generate(),
            asserted_at: now,
            occurred_at: None,
            text: text.to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
    ];
    for i in 0..12 {
        let filler = MemoryId::generate();
        payloads.push(EventPayload::memory_created(
            filler,
            Namespace::Topic.with_name(format!("filler-{i}")),
        ));
        payloads.push(EventPayload::MemoryContentAppended {
            id: filler,
            entry_id: EntryId::generate(),
            asserted_at: now,
            occurred_at: None,
            text: format!("Unrelated note {i} about weather, lunch, and travel plans."),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        });
    }
    server
        .engine
        .store
        .lock()
        .append(now, EventSource::Agent, payloads)
        .unwrap();
    server
        .engine
        .graph
        .lock()
        .materialize_from(server.engine.store.lock().as_ref())
        .unwrap();
    id
}

/// The rendered hint text of every ambient system message a model call saw, across all calls.
fn hint_messages(model: &ScriptedModel) -> Vec<String> {
    model
        .recorded_messages()
        .into_iter()
        .flatten()
        .filter(|message| {
            message.role == Role::System
                && message
                    .content
                    .contains("Possibly relevant to the message above")
        })
        .map(|message| message.content)
        .collect()
}

/// Every `AmbientRecallSurfaced` event in the log, as `(text, surfaced memory ids)`.
fn ambient_events(server: &Instance) -> Vec<(String, Vec<MemoryId>)> {
    server
        .control()
        .events()
        .unwrap()
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::AmbientRecallSurfaced { text, hits, .. } => {
                Some((text.clone(), hits.iter().map(|hit| hit.memory).collect()))
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_matching_message_surfaces_a_recorded_hint() {
    let server = born_server();
    let bonsai = seed_public_topic(
        &server,
        "bonsai",
        "A schema-migration tool Erin built; it versions and applies database migrations.",
    );
    let model = ScriptedModel::new([Completion::Reply(
        "bonsai the migration tool, I think".to_owned(),
    )]);
    let room = ConversationLocator::new("discord", "general");
    server
        .platform()
        .route_message(
            &model,
            &room,
            "dave",
            "What do you think of bonsai?",
            &["dave"],
        )
        .await
        .unwrap();

    // The hint reached the model, naming the memory the brief did not carry.
    let hints = hint_messages(&model);
    assert!(
        hints.iter().any(|hint| hint.contains("topic/bonsai")),
        "the model's prompt carries an ambient hint naming topic/bonsai: {hints:?}"
    );

    // And it is recorded verbatim, with the memory id in the structured hits.
    let events = ambient_events(&server);
    assert!(
        events
            .iter()
            .any(|(text, hits)| hits.contains(&bonsai) && text.contains("topic/bonsai")),
        "an AmbientRecallSurfaced event records the bonsai hit: {events:?}"
    );
}

#[tokio::test]
async fn a_message_with_no_salient_term_surfaces_nothing() {
    let server = born_server();
    seed_public_topic(
        &server,
        "bonsai",
        "A schema-migration tool Erin built; it versions and applies database migrations.",
    );
    let model = ScriptedModel::new([Completion::Reply("anytime".to_owned())]);
    let room = ConversationLocator::new("discord", "general");
    server
        .platform()
        .route_message(&model, &room, "dave", "Thanks, talk soon!", &["dave"])
        .await
        .unwrap();

    assert!(
        hint_messages(&model).is_empty(),
        "no hint is injected for an unrelated message"
    );
    assert!(
        ambient_events(&server).is_empty(),
        "no ambient event is recorded"
    );
}

#[tokio::test]
async fn the_recorded_hint_replays_byte_identical_next_turn() {
    let server = born_server();
    seed_public_topic(
        &server,
        "bonsai",
        "A schema-migration tool Erin built; it versions and applies database migrations.",
    );
    // Two turns in the same room and session: the second replays the first turn's buffer, which now
    // includes the recorded hint. The clock does not advance, so the session is reused.
    let model = ScriptedModel::new([
        Completion::Reply("the migration tool".to_owned()),
        Completion::Reply("still the migration tool".to_owned()),
    ]);
    let room = ConversationLocator::new("discord", "general");
    server
        .platform()
        .route_message(
            &model,
            &room,
            "dave",
            "What do you think of bonsai?",
            &["dave"],
        )
        .await
        .unwrap();
    server
        .platform()
        .route_message(
            &model,
            &room,
            "dave",
            "Anything else about bonsai?",
            &["dave"],
        )
        .await
        .unwrap();

    let calls = model.recorded_messages();
    // The first turn's live hint system message.
    let first_hint = calls[0]
        .iter()
        .find(|message| {
            message.role == Role::System
                && message
                    .content
                    .contains("Possibly relevant to the message above")
        })
        .expect("the first turn injected a hint")
        .content
        .clone();
    // The second turn's prompt must replay that exact message from the buffer — byte-identical, so the
    // serving layer's prefix cache survives.
    assert!(
        calls[1]
            .iter()
            .any(|message| message.role == Role::System && message.content == first_hint),
        "the second turn replays the first turn's hint verbatim"
    );
}
