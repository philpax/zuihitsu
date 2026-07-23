//! Per-attester retraction and the generalized foreign-confidence guard (spec §Visibility →
//! attestations). A conversation turn withdraws only its own account of a corroborated fact, the
//! fact standing on the remaining tellers; a maintenance pass or the console still retracts the whole
//! entry. The guard clears for any speaker who stands among an entry's attesters, not only its
//! founding teller.

use super::{TEST_MAX_ENTRY_CHARS, told};
use crate::{
    clock::ManualClock,
    engine::Engine,
    event::{Cardinality, EventPayload, EventSource, Teller, Visibility},
    graph::Graph,
    ids::{ConversationId, EntryId, MemoryId, Namespace},
    memory::memory_block::{Authority, MemoryBlock, MemoryError, Retraction, VisibilityChoice},
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::RelationName,
};

/// A committed content append with an explicit teller and posture — the entry's founding attestation.
fn appended(
    id: MemoryId,
    entry_id: EntryId,
    text: &str,
    told_by: Teller,
    visibility: Visibility,
) -> EventPayload {
    EventPayload::MemoryContentAppended {
        id,
        entry_id,
        asserted_at: Timestamp::from_millis(900),
        occurred_at: None,
        text: text.to_owned(),
        told_by,
        told_in: None,
        visibility,
    }
}

/// A committed further teller's attestation of an existing entry.
fn attested(memory: MemoryId, entry: EntryId, teller: Teller, posture: Visibility) -> EventPayload {
    EventPayload::EntryAttested {
        memory,
        entry,
        teller,
        told_in: None,
        asserted_at: Timestamp::from_millis(1_500),
        posture,
        phrasing: None,
        source_entry: None,
        produced_by: None,
    }
}

/// A materialized graph over `events`, with the `same_as` relation registered so class resolution
/// works. Names the memories and the entry the caller passes in.
fn committed(events: Vec<EventPayload>) -> Graph {
    let mut store = MemoryStore::new();
    let mut seed = vec![EventPayload::LinkTypeRegistered {
        name: RelationName::SameAs,
        inverse: RelationName::SameAs,
        from_card: Cardinality::Many,
        to_card: Cardinality::Many,
        symmetric: true,
        reflexive: false,
        description: String::new(),
    }];
    seed.extend(events);
    store
        .append(Timestamp::from_millis(1_000), EventSource::Agent, seed)
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    graph
}

/// A block over `graph` under the given teller, authority, and present set — the per-attester routing
/// reads the present set to name a withdrawal's surviving visible attesters.
fn block_present(
    graph: Graph,
    teller: Teller,
    authority: Authority,
    present_set: Vec<MemoryId>,
) -> MemoryBlock {
    let engine = Engine::new(
        Box::new(MemoryStore::new()),
        graph,
        Box::new(ManualClock::new(Timestamp::from_millis(2_000))),
    );
    MemoryBlock::new(
        engine,
        teller,
        authority,
        Some(ConversationId::generate()),
        None,
        present_set,
        TEST_MAX_ENTRY_CHARS,
    )
    .unwrap()
}

/// The founding teller of a corroborated confidence no longer unilaterally kills it: withdrawing its
/// own account leaves the fact standing on the second teller's attestation. The withdrawal note names
/// only the surviving attesters visible to the present audience — never a hidden one.
#[test]
fn a_corroborated_fact_survives_its_founding_tellers_retraction() {
    let topic = MemoryId::generate();
    let (erin, frank, grace) = (
        MemoryId::generate(),
        MemoryId::generate(),
        MemoryId::generate(),
    );
    let entry = EntryId::generate();
    let graph = committed(vec![
        EventPayload::memory_created(topic, Namespace::Topic.with_name("launch")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        EventPayload::memory_created(frank, Namespace::Person.with_name("frank")),
        EventPayload::memory_created(grace, Namespace::Person.with_name("grace")),
        appended(
            topic,
            entry,
            "the launch slipped a week",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
        attested(
            topic,
            entry,
            Teller::Participant(frank),
            Visibility::PrivateToTeller,
        ),
        attested(
            topic,
            entry,
            Teller::Participant(grace),
            Visibility::PrivateToTeller,
        ),
    ]);

    // Erin (the founding teller) retracts, with frank present but grace absent.
    let mut block = block_present(
        graph,
        Teller::Participant(erin),
        Authority::Platform,
        vec![erin, frank],
    );
    let outcome = block
        .retract(topic, entry, "erin reconsidered", None)
        .unwrap();
    let Retraction::Withdrawn { note } = outcome else {
        panic!("expected a per-attester withdrawal, got {outcome:?}");
    };
    assert!(
        note.contains("person/frank"),
        "the note names the surviving visible attester: {note}"
    );
    assert!(
        !note.contains("person/grace"),
        "the note must not name a hidden attester: {note}"
    );
    assert!(
        !note.contains("person/erin"),
        "the note must not name the withdrawing speaker: {note}"
    );

    // Exactly erin's account is withdrawn; the entry is not tombstoned.
    let events = block.into_effects().events;
    let withdrawals: Vec<&EventPayload> = events
        .iter()
        .filter(|event| matches!(event, EventPayload::AttestationRetracted { .. }))
        .collect();
    assert_eq!(withdrawals.len(), 1, "only erin's account is withdrawn");
    assert!(
        matches!(
            withdrawals[0],
            EventPayload::AttestationRetracted { teller, .. } if *teller == Teller::Participant(erin)
        ),
        "the withdrawal names erin",
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, EventPayload::EntryRetracted { .. })),
        "the entry is not tombstoned while frank and grace still attest it"
    );
}

/// A sole teller's retraction withdraws the whole entry — with no one else standing behind the fact,
/// there is nothing left to keep it live, so it tombstones exactly as before attestations existed.
#[test]
fn a_sole_tellers_retraction_withdraws_the_whole_entry() {
    let topic = MemoryId::generate();
    let erin = MemoryId::generate();
    let entry = EntryId::generate();
    let graph = committed(vec![
        EventPayload::memory_created(topic, Namespace::Topic.with_name("launch")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        appended(
            topic,
            entry,
            "the launch slipped a week",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
    ]);

    let mut block = block_present(
        graph,
        Teller::Participant(erin),
        Authority::Platform,
        vec![erin],
    );
    let outcome = block.retract(topic, entry, "no longer true", None).unwrap();
    assert!(matches!(outcome, Retraction::Entry));
    let events = block.into_effects().events;
    assert!(
        events.iter().any(
            |event| matches!(event, EventPayload::EntryRetracted { entry: e, .. } if *e == entry)
        ),
        "the whole entry is retracted"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, EventPayload::AttestationRetracted { .. })),
        "no per-attester withdrawal is recorded for a sole teller"
    );
}

/// A maintenance pass (agent authority) always retracts the whole entry, even one corroborated by
/// several tellers — its reach must not be silently narrowed to a single account.
#[test]
fn agent_authority_retracts_the_whole_corroborated_entry() {
    let topic = MemoryId::generate();
    let (erin, frank) = (MemoryId::generate(), MemoryId::generate());
    let entry = EntryId::generate();
    let graph = committed(vec![
        EventPayload::memory_created(topic, Namespace::Topic.with_name("launch")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        EventPayload::memory_created(frank, Namespace::Person.with_name("frank")),
        appended(
            topic,
            entry,
            "the launch slipped a week",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
        attested(
            topic,
            entry,
            Teller::Participant(frank),
            Visibility::PrivateToTeller,
        ),
    ]);

    let mut block = block_present(graph, Teller::Agent, Authority::Agent, Vec::new());
    let outcome = block.retract(topic, entry, "redundant", None).unwrap();
    assert!(matches!(outcome, Retraction::Entry));
    let events = block.into_effects().events;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, EventPayload::EntryRetracted { .. })),
        "agent authority retracts the whole entry"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, EventPayload::AttestationRetracted { .. })),
        "agent authority is not downgraded to a per-attester withdrawal"
    );
}

/// A speaker who never stood behind a foreign confidence still cannot retract it — even when a third
/// teller has corroborated it. The guard checks every live attester, not only the founding one, and
/// none is the speaker's class.
#[test]
fn a_non_attester_still_cannot_retract_a_foreign_confidence() {
    let topic = MemoryId::generate();
    let (erin, chris, bystander) = (
        MemoryId::generate(),
        MemoryId::generate(),
        MemoryId::generate(),
    );
    let entry = EntryId::generate();
    let graph = committed(vec![
        EventPayload::memory_created(topic, Namespace::Topic.with_name("launch")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        EventPayload::memory_created(chris, Namespace::Person.with_name("chris")),
        EventPayload::memory_created(bystander, Namespace::Person.with_name("dana")),
        appended(
            topic,
            entry,
            "a confided aside",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
        attested(
            topic,
            entry,
            Teller::Participant(chris),
            Visibility::PrivateToTeller,
        ),
    ]);

    let mut block = block_present(
        graph,
        Teller::Participant(bystander),
        Authority::Platform,
        vec![bystander, erin, chris],
    );
    assert!(matches!(
        block.retract(topic, entry, "meddling", None).unwrap_err(),
        MemoryError::ForeignConfidenceSupersedeForbidden
    ));
}

/// The generalization actually loosening: a speaker who privately corroborated another teller's
/// confidence now stands among its attesters, so the guard clears and they may act on it — here a
/// per-attester withdrawal of their own account, the confidence standing on its founding teller.
#[test]
fn a_speaker_who_attested_a_confidence_may_withdraw_their_account() {
    let topic = MemoryId::generate();
    let (erin, frank) = (MemoryId::generate(), MemoryId::generate());
    let entry = EntryId::generate();
    let graph = committed(vec![
        EventPayload::memory_created(topic, Namespace::Topic.with_name("launch")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        EventPayload::memory_created(frank, Namespace::Person.with_name("frank")),
        appended(
            topic,
            entry,
            "a confided aside",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
        attested(
            topic,
            entry,
            Teller::Participant(frank),
            Visibility::PrivateToTeller,
        ),
    ]);

    // Frank corroborated erin's confidence; frank retracts. The guard clears (frank attests it), and
    // the routing withdraws only frank's account.
    let mut block = block_present(
        graph,
        Teller::Participant(frank),
        Authority::Platform,
        vec![erin, frank],
    );
    let outcome = block
        .retract(topic, entry, "frank was mistaken", None)
        .unwrap();
    assert!(
        matches!(outcome, Retraction::Withdrawn { .. }),
        "frank withdraws his own account rather than being refused"
    );
    let events = block.into_effects().events;
    assert!(
        matches!(
            events.iter().find(|event| matches!(event, EventPayload::AttestationRetracted { .. })),
            Some(EventPayload::AttestationRetracted { teller, .. }) if *teller == Teller::Participant(frank)
        ),
        "frank's account is the one withdrawn"
    );
}

/// The same loosening on the supersede path: a speaker who attested a foreign confidence may now
/// supersede it (supersede stays whole-entry — it is the guard that loosened, not the routing).
#[test]
fn a_speaker_who_attested_a_confidence_may_supersede_it() {
    let topic = MemoryId::generate();
    let (erin, frank) = (MemoryId::generate(), MemoryId::generate());
    let entry = EntryId::generate();
    let graph = committed(vec![
        EventPayload::memory_created(topic, Namespace::Topic.with_name("launch")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        EventPayload::memory_created(frank, Namespace::Person.with_name("frank")),
        appended(
            topic,
            entry,
            "a confided aside",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
        attested(
            topic,
            entry,
            Teller::Participant(frank),
            Visibility::PrivateToTeller,
        ),
    ]);

    let mut block = block_present(
        graph,
        Teller::Participant(frank),
        Authority::Platform,
        vec![erin, frank],
    );
    let new = block
        .append(
            topic,
            "a corrected aside",
            told(Teller::Participant(frank), VisibilityChoice::Private),
        )
        .unwrap();
    block.supersede(topic, entry, new).unwrap();
}

/// A public entry carrying a hidden private attestation stays retractable by an uninvolved speaker:
/// the guard keys on the *founding* posture, which is public, so it never gates — gating on the hidden
/// attestation would leak its existence and make a public fact un-correctable.
#[test]
fn a_public_entry_with_a_hidden_attestation_is_retractable_by_an_uninvolved_speaker() {
    let topic = MemoryId::generate();
    let (erin, chris, bystander) = (
        MemoryId::generate(),
        MemoryId::generate(),
        MemoryId::generate(),
    );
    let entry = EntryId::generate();
    let graph = committed(vec![
        EventPayload::memory_created(topic, Namespace::Topic.with_name("launch")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        EventPayload::memory_created(chris, Namespace::Person.with_name("chris")),
        EventPayload::memory_created(bystander, Namespace::Person.with_name("dana")),
        appended(
            topic,
            entry,
            "the launch shipped on the third",
            Teller::Participant(erin),
            Visibility::Public,
        ),
        attested(
            topic,
            entry,
            Teller::Participant(chris),
            Visibility::PrivateToTeller,
        ),
    ]);

    // An uninvolved speaker, with chris absent so his private attestation is hidden.
    let mut block = block_present(
        graph,
        Teller::Participant(bystander),
        Authority::Platform,
        vec![bystander, erin],
    );
    let outcome = block
        .retract(topic, entry, "the date was wrong", None)
        .unwrap();
    assert!(
        matches!(outcome, Retraction::Entry),
        "a public fact is retractable outright by anyone"
    );
    assert!(
        block
            .into_effects()
            .events
            .iter()
            .any(|event| matches!(event, EventPayload::EntryRetracted { .. })),
        "the whole public entry is retracted"
    );
}

/// A live read names every attester the audience may see and hides a confided one: a public fact
/// corroborated by two participants and privately confided by a third reads `from person/erin,
/// person/dave` while frank is absent, his confidence leaving no residue in the read.
#[test]
fn a_read_names_visible_attesters_and_hides_a_confided_one() {
    let topic = MemoryId::generate();
    let (erin, dave, frank) = (
        MemoryId::generate(),
        MemoryId::generate(),
        MemoryId::generate(),
    );
    let entry = EntryId::generate();
    let graph = committed(vec![
        EventPayload::memory_created(topic, Namespace::Topic.with_name("launch")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        EventPayload::memory_created(dave, Namespace::Person.with_name("dave")),
        EventPayload::memory_created(frank, Namespace::Person.with_name("frank")),
        appended(
            topic,
            entry,
            "the launch shipped on the third",
            Teller::Participant(erin),
            Visibility::Public,
        ),
        attested(
            topic,
            entry,
            Teller::Participant(dave),
            Visibility::Attributed,
        ),
        attested(
            topic,
            entry,
            Teller::Participant(frank),
            Visibility::PrivateToTeller,
        ),
    ]);

    // Read with dave present (frank absent): erin's public founding and dave's attributed
    // corroboration surface; frank's confidence is hidden.
    let mut block = block_present(graph, Teller::Agent, Authority::Agent, vec![dave]);
    let refs = block.entries(topic).unwrap();
    let read = refs.iter().find(|r| r.entry_id == entry).unwrap();
    assert_eq!(
        read.attesters,
        vec!["person/erin".to_owned(), "person/dave".to_owned()],
        "the read names the visible attesters, founding-first"
    );
    assert!(
        !read.attesters.iter().any(|name| name.contains("frank")),
        "the confided attester leaves no residue: {:?}",
        read.attesters
    );
}
