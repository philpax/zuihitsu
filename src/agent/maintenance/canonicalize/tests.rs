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

    let considered = catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap()
        .considered;

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

/// Seed a bare `person/<name>` profile `same_as`-linked to `stub`, with no designation — a hand-merged
/// bare member the pass must arbitrate over when several contend.
fn bare_member(stub: MemoryId, member: MemoryId, name: &str) -> Vec<EventPayload> {
    vec![
        EventPayload::memory_created(member, Namespace::Person.with_name(name)),
        EventPayload::link_created(
            stub,
            member,
            RelationName::SameAs,
            LinkPosture {
                source: LinkSource::Operator,
                told_by: None,
                told_in: None,
                visibility: Visibility::Public,
            },
        ),
    ]
}

#[tokio::test]
async fn several_bare_members_designate_the_model_identified_one_not_the_earliest_ulid() {
    // Two undesignated bare members contend: an imprint artifact (`person/operator`, the earliest ULID)
    // and the person's real named profile (a later ULID). The old blind-pick-by-ULID rule would
    // designate the artifact; the pass must instead read the stub's evidence, identify the real name,
    // and designate the matching member — the later ULID here, proving the choice is arbitrated, not
    // positional.
    let stub = MemoryId::generate();
    let a = MemoryId::generate();
    let b = MemoryId::generate();
    let earliest = a.min(b);
    let latest = a.max(b);
    let mut events = prerequisites();
    events.extend([
        EventPayload::memory_created(stub, Namespace::Person.with_name("rowan@discord")),
        EventPayload::MemoryContentAppended {
            id: stub,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(1_000),
            occurred_at: None,
            text: "Goes by Rowan on the server.".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
        EventPayload::participant_identified(stub, "discord", "rowan#4242"),
    ]);
    // The imprint artifact is the earliest ULID; the real profile is the later one.
    events.extend(bare_member(stub, earliest, "operator"));
    events.extend(bare_member(stub, latest, "rowan"));
    let engine = engine_with(events);
    let model = ScriptedModel::new([Completion::Reply(r#"{"name": "rowan"}"#.to_owned())]);

    catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    assert_eq!(
        designations(&engine),
        vec![(latest, true)],
        "the model-identified bare member is designated, not the earliest-ULID artifact"
    );
    assert!(
        minted_person_names(&engine)
            .iter()
            .all(|name| name != "person/rowan-2"),
        "no suffixed duplicate is minted for a stub that already has bare members"
    );
}

#[tokio::test]
async fn several_bare_members_fall_back_to_the_earliest_when_the_model_abstains() {
    // Two undesignated bare members contend, but the evidence is too weak to name: the model abstains
    // (an empty object). The pass falls back to the earliest-ULID candidate deterministically rather
    // than leaving the stub undesignated.
    let stub = MemoryId::generate();
    let a = MemoryId::generate();
    let b = MemoryId::generate();
    let earliest = a.min(b);
    let latest = a.max(b);
    let mut events = prerequisites();
    events.extend([
        EventPayload::memory_created(stub, Namespace::Person.with_name("someone@discord")),
        EventPayload::MemoryContentAppended {
            id: stub,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(1_000),
            occurred_at: None,
            text: "likes long walks".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
        EventPayload::participant_identified(stub, "discord", "someone#0009"),
    ]);
    events.extend(bare_member(stub, earliest, "operator"));
    events.extend(bare_member(stub, latest, "rowan"));
    let engine = engine_with(events);
    // The model abstains: an empty object parses to `NameIdentification { name: None }`.
    let model = ScriptedModel::new([Completion::Reply("{}".to_owned())]);

    catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    assert_eq!(
        designations(&engine),
        vec![(earliest, true)],
        "an abstention falls back to the earliest-ULID bare member"
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

    let considered = catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap()
        .considered;

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

    let considered = catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap()
        .considered;

    assert_eq!(considered, 1);
    assert_eq!(
        minted_person_names(&engine),
        vec!["person/someone@discord".to_owned()],
        "an abstention mints no canonical profile"
    );
    assert!(designations(&engine).is_empty());
}

#[tokio::test]
async fn borrowed_entries_bind_the_empty_stub_to_the_profile_they_sit_on() {
    // The agent wrote this person's own facts directly onto a bare `person/rowan` profile,
    // teller-stamped from the stub `person/rowan@chat`, and never bound the two — so the stub is empty
    // and its identity class never closes over it. The pass must read those borrowed entries, name the
    // profile from them, and bind the stub to it (`same_as` plus a designation), minting nothing.
    let stub = MemoryId::generate();
    let bare = MemoryId::generate();
    let mut events = prerequisites();
    events.extend([
        EventPayload::memory_created(stub, Namespace::Person.with_name("rowan@chat")),
        EventPayload::memory_created(bare, Namespace::Person.with_name("rowan")),
        EventPayload::MemoryContentAppended {
            id: bare,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(1_000),
            occurred_at: None,
            text: "I'm Rowan, I run the community garden on weekends.".to_owned(),
            told_by: Teller::Participant(stub),
            told_in: None,
            visibility: Visibility::Public,
        },
        EventPayload::participant_identified(stub, "chat", "rowan#0001"),
    ]);
    let engine = engine_with(events);
    let model = ScriptedModel::new([Completion::Reply(r#"{"name": "rowan"}"#.to_owned())]);

    catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    let same_as = same_as_pairs(&engine);
    assert!(
        same_as
            .iter()
            .any(|(from, to)| (*from == stub && *to == bare) || (*from == bare && *to == stub)),
        "the stub is same_as-bound to the profile its borrowed evidence names: {same_as:?}"
    );
    assert_eq!(
        designations(&engine),
        vec![(bare, true)],
        "the existing profile is designated the class primary"
    );
    assert_eq!(
        minted_person_names(&engine),
        vec!["person/rowan@chat".to_owned(), "person/rowan".to_owned()],
        "no new profile is minted — the empty stub binds to the existing one"
    );
}

#[tokio::test]
async fn borrowed_entries_describing_a_third_person_bind_nothing() {
    // The only entries the stub told on `person/rowan` are about someone else — a sibling who shares
    // the name — not the teller's own identity. The model abstains, and the pass binds and mints
    // nothing: borrowed evidence names only the profile whose own identity it evidences.
    let stub = MemoryId::generate();
    let bare = MemoryId::generate();
    let mut events = prerequisites();
    events.extend([
        EventPayload::memory_created(stub, Namespace::Person.with_name("rowan@chat")),
        EventPayload::memory_created(bare, Namespace::Person.with_name("rowan")),
        EventPayload::MemoryContentAppended {
            id: bare,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(1_000),
            occurred_at: None,
            text: "My sister Rowan just moved to Perth for a new job.".to_owned(),
            told_by: Teller::Participant(stub),
            told_in: None,
            visibility: Visibility::Public,
        },
        EventPayload::participant_identified(stub, "chat", "rowan#0002"),
    ]);
    let engine = engine_with(events);
    // The model abstains on third-person evidence: an empty object parses to `name: None`.
    let model = ScriptedModel::new([Completion::Reply("{}".to_owned())]);

    catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    assert!(
        same_as_pairs(&engine).is_empty(),
        "no same_as is asserted when the borrowed evidence describes a third person"
    );
    assert!(
        designations(&engine).is_empty(),
        "nothing is designated on an abstention"
    );
    assert_eq!(
        minted_person_names(&engine),
        vec!["person/rowan@chat".to_owned(), "person/rowan".to_owned()],
        "no profile is minted from borrowed evidence"
    );
}

#[tokio::test]
async fn an_empty_stub_whose_stem_matches_no_profile_abstains_without_a_model_call() {
    // An unrelated bare profile exists, but none at the stub's stem (`person/rowan`): the lookup
    // misses, so the pass abstains before any model call and binds nothing. Borrowed evidence is drawn
    // only from the profile at the stub's own stem, never a differently-named one.
    let stub = MemoryId::generate();
    let other = MemoryId::generate();
    let mut events = prerequisites();
    events.extend([
        EventPayload::memory_created(stub, Namespace::Person.with_name("rowan@chat")),
        EventPayload::memory_created(other, Namespace::Person.with_name("robin")),
        EventPayload::MemoryContentAppended {
            id: other,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(1_000),
            occurred_at: None,
            text: "I'm Robin, a potter.".to_owned(),
            told_by: Teller::Participant(stub),
            told_in: None,
            visibility: Visibility::Public,
        },
        EventPayload::participant_identified(stub, "chat", "rowan#0003"),
    ]);
    let engine = engine_with(events);
    // An empty scripted model panics on any call — asserting the pass abstains before reaching one.
    let model = ScriptedModel::new([]);

    catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    assert!(
        same_as_pairs(&engine).is_empty(),
        "no bind happens when the stem matches no profile"
    );
    assert!(designations(&engine).is_empty());
}

#[tokio::test]
async fn a_stub_with_a_designated_bare_primary_is_untouched() {
    // The stub already has a bare `same_as` member designated its class primary — a completed
    // canonical identity. The pass must skip it: no model call, no new same_as, no re-designation.
    let stub = MemoryId::generate();
    let bare = MemoryId::generate();
    let mut events = prerequisites();
    events.extend([
        EventPayload::memory_created(stub, Namespace::Person.with_name("rowan@chat")),
        EventPayload::memory_created(bare, Namespace::Person.with_name("rowan")),
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
        EventPayload::ClassPrimaryDesignated {
            memory: bare,
            designated: true,
        },
        EventPayload::participant_identified(stub, "chat", "rowan#0004"),
    ]);
    let engine = engine_with(events);
    // An empty scripted model panics on any call — asserting the completed stub reaches no model.
    let model = ScriptedModel::new([]);

    catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    // The only designation and same_as in the log are the seeded ones; the pass appended neither.
    assert_eq!(designations(&engine), vec![(bare, true)]);
    assert_eq!(same_as_pairs(&engine).len(), 1);
}
