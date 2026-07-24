//! Canonicalize pass tests: the hand-merged designation path, and abstention on evidence-poor stubs.

use std::sync::Arc;

use crate::{
    agent::maintenance::canonicalize::catch_up,
    clock::ManualClock,
    engine::Engine,
    event::{
        Cardinality, EventPayload, EventSource, LinkPosture, LinkSource, PromptTemplateName,
        Teller, Visibility,
    },
    graph::Graph,
    ids::{EntryId, MemoryId, Namespace, Seq},
    model::{Completion, ModelClient, ScriptedModel},
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::RelationName,
};

/// Build an `Arc<Engine>` over an in-memory store and graph, seeded with `events` (committed under
/// `EventSource::Agent`) and materialized.
fn engine_with(events: Vec<EventPayload>) -> Arc<Engine> {
    let mut store = MemoryStore::new();
    store
        .append(Timestamp::from_millis(1_000), EventSource::Agent, events)
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    Engine::new(
        Box::new(store),
        graph,
        Box::new(ManualClock::new(Timestamp::from_millis(2_000))),
    )
}

/// The `same_as` relation registration and the name-identification template, the two prerequisites
/// every canonicalize sweep needs in the log.
fn prerequisites() -> Vec<EventPayload> {
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
        EventPayload::prompt_template_registered(
            PromptTemplateName::NameIdentification,
            1,
            "identify a name or abstain".to_owned(),
        ),
    ]
}

/// Every `ClassPrimaryDesignated` in the store, as `(memory, designated)` pairs.
fn designations(engine: &Engine) -> Vec<(MemoryId, bool)> {
    engine
        .store
        .lock()
        .read_from(Seq::ZERO)
        .unwrap()
        .into_iter()
        .filter_map(|event| match event.payload {
            EventPayload::ClassPrimaryDesignated { memory, designated } => {
                Some((memory, designated))
            }
            _ => None,
        })
        .collect()
}

/// The bare `person/<name>` memories that were minted, as their handles — for asserting whether a new
/// profile was created.
fn minted_person_names(engine: &Engine) -> Vec<String> {
    engine
        .store
        .lock()
        .read_from(Seq::ZERO)
        .unwrap()
        .into_iter()
        .filter_map(|event| match event.payload {
            EventPayload::MemoryCreated { name, .. } => Some(name.as_str().to_owned()),
            _ => None,
        })
        .collect()
}

/// The id of the memory created under `name`, if the run created one.
fn memory_id_named(engine: &Engine, name: &str) -> Option<MemoryId> {
    engine
        .store
        .lock()
        .read_from(Seq::ZERO)
        .unwrap()
        .into_iter()
        .find_map(|event| match event.payload {
            EventPayload::MemoryCreated { id, name: created } if created.as_str() == name => {
                Some(id)
            }
            _ => None,
        })
}

/// Every `same_as` link authored, as `(from, to)` pairs.
fn same_as_pairs(engine: &Engine) -> Vec<(MemoryId, MemoryId)> {
    engine
        .store
        .lock()
        .read_from(Seq::ZERO)
        .unwrap()
        .into_iter()
        .filter_map(|event| match event.payload {
            EventPayload::LinkCreated {
                from,
                to,
                relation: RelationName::SameAs,
                ..
            } => Some((from, to)),
            _ => None,
        })
        .collect()
}

/// How many merge proposals the run recorded.
fn merge_proposal_count(engine: &Engine) -> usize {
    engine
        .store
        .lock()
        .read_from(Seq::ZERO)
        .unwrap()
        .into_iter()
        .filter(|event| matches!(event.payload, EventPayload::MergeProposed { .. }))
        .count()
}

#[tokio::test]
async fn a_name_collision_mints_a_suffixed_profile_without_touching_the_existing_one() {
    // The stub's person goes by "Robin", but an unrelated, already-populated `person/robin` (a
    // different person) already owns the bare name and is not linked to the stub. The pass must mint a
    // disambiguated `person/robin-2` bound to the stub, and must never merge or assert onto the
    // stranger's profile — a suffixed mint, not a squat.
    let stub = MemoryId::generate();
    let existing = MemoryId::generate();
    let mut events = prerequisites();
    events.extend([
        EventPayload::memory_created(stub, Namespace::Person.with_name("robin@discord")),
        EventPayload::MemoryContentAppended {
            id: stub,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(1_000),
            occurred_at: None,
            text: "Goes by Robin on the server.".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
        EventPayload::participant_identified(stub, "discord", "robin#7788"),
        // An unrelated, populated person who already owns the bare name — not linked to the stub.
        EventPayload::memory_created(existing, Namespace::Person.with_name("robin")),
        EventPayload::MemoryContentAppended {
            id: existing,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(1_000),
            occurred_at: None,
            text: "Robin is a freelance graphic designer based in Perth.".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
    ]);
    let engine = engine_with(events);
    let model = ScriptedModel::new([Completion::Reply(r#"{"name": "robin"}"#.to_owned())]);

    catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    let robin2 = memory_id_named(&engine, "person/robin-2")
        .expect("a disambiguated person/robin-2 profile is minted");
    let same_as = same_as_pairs(&engine);
    assert!(
        same_as
            .iter()
            .any(|(from, to)| (*from == stub && *to == robin2) || (*from == robin2 && *to == stub)),
        "the suffixed profile is same_as-bound to the stub: {same_as:?}"
    );
    assert!(
        !same_as
            .iter()
            .any(|(from, to)| *from == existing || *to == existing),
        "the unrelated existing profile is never a same_as endpoint: {same_as:?}"
    );
    assert_eq!(
        merge_proposal_count(&engine),
        0,
        "the pass proposes no merge onto the existing profile"
    );
}

#[tokio::test]
async fn a_hand_merged_stub_designates_its_bare_member_rather_than_minting() {
    // The live-data shape: a platform stub (`person/vertas@discord`) hand-merged with a bare profile
    // (`person/vertas`) via `same_as`, but with no designation ever written. The pass must designate
    // the bare member primary and mint no new profile — not collide on the name and mint `vertas-2`.
    let stub = MemoryId::generate();
    let bare = MemoryId::generate();
    let mut events = prerequisites();
    events.extend([
        EventPayload::memory_created(stub, Namespace::Person.with_name("vertas@discord")),
        EventPayload::memory_created(bare, Namespace::Person.with_name("vertas")),
        EventPayload::link_created(
            stub,
            bare,
            RelationName::SameAs,
            LinkPosture {
                source: LinkSource::Operator,
                told_by: None,
                told_in: None,
                visibility: Visibility::Public,
            },
        ),
        EventPayload::participant_identified(stub, "discord", "vertas#0001"),
    ]);
    let engine = engine_with(events);
    let model = ScriptedModel::new([]);

    let (_, considered) = catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    assert_eq!(considered, 1, "the one identified stub is considered");
    assert_eq!(
        designations(&engine),
        vec![(bare, true)],
        "the existing bare member is designated primary"
    );
    // Only the two seeded memories exist — no suffixed duplicate was minted.
    assert_eq!(
        minted_person_names(&engine),
        vec![
            "person/vertas@discord".to_owned(),
            "person/vertas".to_owned()
        ],
        "no new profile is minted for a stub that already has a bare member"
    );
}

#[tokio::test]
async fn an_entryless_stub_is_left_unnamed() {
    // A stub with no bare member and no entries has no name evidence: the pass abstains, calling the
    // model not at all (the scripted model would panic on an unexpected call) and minting nothing.
    let stub = MemoryId::generate();
    let mut events = prerequisites();
    events.extend([
        EventPayload::memory_created(stub, Namespace::Person.with_name("ghost@discord")),
        EventPayload::participant_identified(stub, "discord", "ghost#0002"),
    ]);
    let engine = engine_with(events);
    let model = ScriptedModel::new([]);

    let (_, considered) = catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    assert_eq!(considered, 1);
    assert!(
        designations(&engine).is_empty(),
        "nothing is designated for an evidence-poor stub"
    );
    assert_eq!(
        minted_person_names(&engine),
        vec!["person/ghost@discord".to_owned()],
        "no canonical profile is minted for an entryless stub"
    );
}

#[tokio::test]
async fn a_vague_stub_abstains_when_the_model_returns_no_name() {
    // A stub with entries but no clear name: the model is called and abstains (an empty JSON object,
    // no `name` field), so no profile is minted.
    let stub = MemoryId::generate();
    let entry = crate::ids::EntryId::generate();
    let mut events = prerequisites();
    events.extend([
        EventPayload::memory_created(stub, Namespace::Person.with_name("someone@discord")),
        EventPayload::MemoryContentAppended {
            id: stub,
            entry_id: entry,
            asserted_at: Timestamp::from_millis(1_000),
            occurred_at: None,
            text: "likes long walks".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
        EventPayload::participant_identified(stub, "discord", "someone#0003"),
    ]);
    let engine = engine_with(events);
    // The model abstains: an empty object parses to `NameIdentification { name: None }`.
    let model = ScriptedModel::new([Completion::Reply("{}".to_owned())]);

    let (_, considered) = catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    assert_eq!(considered, 1);
    assert_eq!(
        minted_person_names(&engine),
        vec!["person/someone@discord".to_owned()],
        "an abstention mints no canonical profile"
    );
    assert!(designations(&engine).is_empty());
}
