//! Ambient recall: the pre-turn lexical pass surfaces a memory the frozen brief did not, injected as a
//! system hint and recorded as an `AmbientRecallSurfaced` event that a later turn replays byte for byte.
//! These tests drive the real `route_message` path so the pass runs where it does in production, and
//! check both the live prompt the model saw and the recorded event.

use super::*;
use crate::{
    ConversationLocator, InstanceFeatures, PersonId, SeedSelf, TEST_PLATFORM,
    clock::ManualClock,
    event::{EventPayload, EventSource, Teller, Visibility},
    graph::Graph,
    ids::{EntryId, MemoryId, Namespace, TurnId},
    model::{Completion, Role, ScriptedModel},
    store::MemoryStore,
    time::Timestamp,
    turn_ref,
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
    let room = ConversationLocator::new(TEST_PLATFORM, "general");
    server
        .platform()
        .route_message(
            &model,
            &room,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "What do you think of bonsai?",
            &[PersonId::new(TEST_PLATFORM, "dave")],
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
    let room = ConversationLocator::new(TEST_PLATFORM, "general");
    server
        .platform()
        .route_message(
            &model,
            &room,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "Thanks, talk soon!",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
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
    let room = ConversationLocator::new(TEST_PLATFORM, "general");
    server
        .platform()
        .route_message(
            &model,
            &room,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "What do you think of bonsai?",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    server
        .platform()
        .route_message(
            &model,
            &room,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "Anything else about bonsai?",
            &[PersonId::new(TEST_PLATFORM, "dave")],
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

/// Every ambient system message a model call saw whose content carries `needle`, across all calls.
fn hints_containing(model: &ScriptedModel, needle: &str) -> Vec<String> {
    model
        .recorded_messages()
        .into_iter()
        .flatten()
        .filter(|message| message.role == Role::System && message.content.contains(needle))
        .map(|message| message.content)
        .collect()
}

#[tokio::test]
async fn a_turn_token_leads_the_hint_with_a_convo_turn_pointer() {
    let server = born_server();
    // No seeded topic, so the message matches nothing lexically: the token alone drives the hint.
    let turn = TurnId::generate();
    let token = turn_ref::construct(turn);
    let model = ScriptedModel::new([Completion::Reply("let me look".to_owned())]);
    let room = ConversationLocator::new(TEST_PLATFORM, "general");
    let message = format!("what did we decide in {token}?");
    server
        .platform()
        .route_message(
            &model,
            &room,
            &PersonId::new(TEST_PLATFORM, "dave"),
            &message,
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();

    // The hint reached the model, leading with the convo.turn pointer for the exact cited id.
    let pointer = format!("convo.turn(\"{}\")", turn.0);
    let hints = hints_containing(&model, &pointer);
    assert!(
        hints.iter().any(|hint| hint
            .lines()
            .next()
            .is_some_and(|line| line.contains(&pointer))),
        "the hint leads with the token's resolver: {hints:?}"
    );

    // And it is recorded, with no lexical hits riding along.
    let events = ambient_events(&server);
    assert!(
        events
            .iter()
            .any(|(text, hits)| text.contains(&pointer) && hits.is_empty()),
        "a token-only AmbientRecallSurfaced event is recorded: {events:?}"
    );
}

#[tokio::test]
async fn a_url_points_the_hint_at_web_markdown() {
    let server = born_server();
    // No seeded topic, so the message matches nothing lexically: the URL alone drives the hint. The
    // default instance has browsing on.
    let model = ScriptedModel::new([Completion::Reply("reading it now".to_owned())]);
    let room = ConversationLocator::new(TEST_PLATFORM, "general");
    let url = "https://example.com/article";
    let message = format!("take a look at {url} when you can");
    server
        .platform()
        .route_message(
            &model,
            &room,
            &PersonId::new(TEST_PLATFORM, "dave"),
            &message,
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();

    // The hint reached the model, pointing at reading the link with web.markdown.
    let pointer = format!("web.markdown(\"{url}\")");
    let hints = hints_containing(&model, &pointer);
    assert!(
        !hints.is_empty(),
        "the model's prompt carries a web.markdown pointer for the shared link: {hints:?}"
    );

    // And it is recorded verbatim, with no lexical hits riding along.
    let events = ambient_events(&server);
    assert!(
        events
            .iter()
            .any(|(text, hits)| text.contains(&pointer) && hits.is_empty()),
        "a URL-only AmbientRecallSurfaced event is recorded: {events:?}"
    );
}

#[tokio::test]
async fn a_turn_token_is_inert_when_transcripts_are_off() {
    // The convo.turn resolver is transcripts-gated, so with the feature off a token yields no pointer —
    // and, matching nothing lexically either, no hint at all (nudging at a nil call would be cruel).
    let features = InstanceFeatures {
        transcripts: false,
        ..InstanceFeatures::default()
    };
    let server = Instance::with_features(
        Box::new(MemoryStore::new()),
        Graph::open_in_memory().unwrap(),
        Box::new(ManualClock::new(Timestamp::from_millis(1_000))),
        features,
    );
    server
        .control()
        .create_agent(&SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "A discreet companion with a long memory.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();

    let turn = TurnId::generate();
    let token = turn_ref::construct(turn);
    let model = ScriptedModel::new([Completion::Reply("hmm".to_owned())]);
    let room = ConversationLocator::new(TEST_PLATFORM, "general");
    let message = format!("what did we decide in {token}?");
    server
        .platform()
        .route_message(
            &model,
            &room,
            &PersonId::new(TEST_PLATFORM, "dave"),
            &message,
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();

    assert!(
        hints_containing(&model, "convo.turn").is_empty(),
        "no convo.turn pointer is injected when transcripts are off"
    );
    assert!(
        ambient_events(&server).is_empty(),
        "no ambient event is recorded for a token-only message with the feature off"
    );
}

#[tokio::test]
async fn a_token_hint_replays_byte_identical_next_turn() {
    let server = born_server();
    let turn = TurnId::generate();
    let token = turn_ref::construct(turn);
    // Two turns in one session: the second replays the first's buffer, which now holds the token hint.
    let model = ScriptedModel::new([
        Completion::Reply("looking".to_owned()),
        Completion::Reply("still looking".to_owned()),
    ]);
    let room = ConversationLocator::new(TEST_PLATFORM, "general");
    let first = format!("what did we decide in {token}?");
    server
        .platform()
        .route_message(
            &model,
            &room,
            &PersonId::new(TEST_PLATFORM, "dave"),
            &first,
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    server
        .platform()
        .route_message(
            &model,
            &room,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "anything else?",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();

    let calls = model.recorded_messages();
    let first_hint = calls[0]
        .iter()
        .find(|message| message.role == Role::System && message.content.contains("convo.turn"))
        .expect("the first turn injected a token hint")
        .content
        .clone();
    assert!(
        calls[1]
            .iter()
            .any(|message| message.role == Role::System && message.content == first_hint),
        "the second turn replays the first turn's token hint verbatim"
    );
}
