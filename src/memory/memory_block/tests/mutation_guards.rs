//! The confidential-untag and foreign-confidence supersede guards (issue #16).

use crate::{
    clock::ManualClock,
    event::{Cardinality, EventPayload, EventSource, Teller},
    graph::Graph,
    ids::{MemoryId, Namespace},
    memory::memory_block::{
        LinkOptions,
        tests::{
            Authority, MemoryBlock, MemoryError, VisibilityChoice, block,
            block_without_conversation, graph_with_merged_pair, told,
        },
    },
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::{RelationName, TagName},
};

#[test]
fn a_redundant_link_create_is_dropped_but_a_changed_one_records() {
    // Re-asserting a link identical to what is already committed records nothing — the graph would only
    // upsert the same row — while a re-link that changes the edge's posture (here, its visibility) still
    // records, so the "make this public" path keeps working. Two blocks with no conversation keep a
    // link's provenance a deterministic function of its inputs, so the second re-derives the first's.
    let dave = MemoryId::generate();
    let erin = MemoryId::generate();
    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![
                EventPayload::memory_created(dave, Namespace::Person.with_name("dave")),
                EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
                EventPayload::LinkTypeRegistered {
                    name: RelationName::new("knows"),
                    inverse: RelationName::new("known_by"),
                    from_card: Cardinality::Many,
                    to_card: Cardinality::Many,
                    symmetric: false,
                    reflexive: false,
                    description: String::new(),
                },
            ],
        )
        .unwrap();

    let materialize = |store: &MemoryStore| {
        let mut graph = Graph::open_in_memory().unwrap();
        graph.materialize_from(store).unwrap();
        graph
    };
    // A person-link needs an explicit visibility (the teachable gate), so every assertion forces one.
    let forced = |choice| {
        Some(LinkOptions {
            visibility: Some(choice),
            exclude: None,
        })
    };
    let created_links = |block: MemoryBlock| {
        block
            .into_effects()
            .events
            .into_iter()
            .filter(|event| matches!(event, EventPayload::LinkCreated { .. }))
            .collect::<Vec<_>>()
    };

    // The first assertion of a public knows-link records it; commit it to the log.
    let mut first = block_without_conversation(
        materialize(&store),
        ManualClock::new(Timestamp::from_millis(2_000)),
        Teller::Agent,
        Authority::Platform,
    );
    first
        .link(
            dave,
            erin,
            RelationName::new("knows"),
            forced(VisibilityChoice::Public),
        )
        .unwrap();
    let committed = created_links(first);
    assert_eq!(committed.len(), 1, "the first assertion records the link");
    store
        .append(Timestamp::from_millis(3_000), EventSource::Agent, committed)
        .unwrap();

    // Re-asserting the identical link — same source, teller, room, and visibility — records nothing.
    let mut again = block_without_conversation(
        materialize(&store),
        ManualClock::new(Timestamp::from_millis(4_000)),
        Teller::Agent,
        Authority::Platform,
    );
    again
        .link(
            dave,
            erin,
            RelationName::new("knows"),
            forced(VisibilityChoice::Public),
        )
        .unwrap();
    let effects = again.into_effects();
    assert!(
        !effects
            .events
            .iter()
            .any(|event| matches!(event, EventPayload::LinkCreated { .. })),
        "the redundant re-link is dropped"
    );
    assert!(
        effects.touched.contains(&dave) && effects.touched.contains(&erin),
        "but its endpoints still count as touched this turn"
    );

    // Re-asserting the same edge with a different visibility does record — the upsert must take effect.
    let mut changed = block_without_conversation(
        materialize(&store),
        ManualClock::new(Timestamp::from_millis(5_000)),
        Teller::Agent,
        Authority::Platform,
    );
    changed
        .link(
            dave,
            erin,
            RelationName::new("knows"),
            forced(VisibilityChoice::Private),
        )
        .unwrap();
    assert_eq!(
        created_links(changed).len(),
        1,
        "a re-link that changes visibility still records"
    );
}

#[test]
fn platform_authority_cannot_untag_confidential() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let room = block
        .create(Namespace::Context.with_name("leads"), None)
        .unwrap();

    // Removing #confidential from a turn is barred — it retroactively re-marks every aside told under it.
    assert!(matches!(
        block.untag(room, TagName::Confidential).unwrap_err(),
        MemoryError::ConfidentialUntagForbidden
    ));
    // Adding #confidential stays ungated (adding is conservative); untagging any other tag is allowed.
    block
        .create_tag(TagName::Confidential, "said in confidence")
        .unwrap();
    block.tag(room, TagName::Confidential).unwrap();
    block
        .untag(room, TagName::Other("archived".into()))
        .unwrap();
}

#[test]
fn operator_authority_may_untag_confidential() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Operator);
    let room = block
        .create(Namespace::Context.with_name("leads"), None)
        .unwrap();
    // The console may clear a room's confidentiality.
    block.untag(room, TagName::Confidential).unwrap();
}

#[test]
fn a_teller_may_supersede_their_own_confidence() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let speaker = MemoryId::generate();
    let mut block = block(
        graph,
        clock,
        Teller::Participant(speaker),
        Authority::Platform,
    );
    let topic = block
        .create(Namespace::Topic.with_name("aside"), None)
        .unwrap();
    // Both entries are the speaker's own confidence, so consolidating them is their own turn's business.
    let old = block
        .append(
            topic,
            "first",
            told(Teller::Participant(speaker), VisibilityChoice::Private),
        )
        .unwrap();
    let new = block
        .append(
            topic,
            "second",
            told(Teller::Participant(speaker), VisibilityChoice::Private),
        )
        .unwrap();
    block.supersede(topic, old, new).unwrap();
}

#[test]
fn platform_authority_cannot_supersede_a_foreign_confidence() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let speaker = MemoryId::generate();
    let other = MemoryId::generate();
    let mut block = block(
        graph,
        clock,
        Teller::Participant(speaker),
        Authority::Platform,
    );
    let topic = block
        .create(Namespace::Topic.with_name("aside"), None)
        .unwrap();
    // A confidence told by a *different* participant is theirs; the current speaker's turn cannot retract it.
    let old = block
        .append(
            topic,
            "confided",
            told(Teller::Participant(other), VisibilityChoice::Private),
        )
        .unwrap();
    let new = block
        .append(
            topic,
            "replacement",
            told(Teller::Participant(speaker), VisibilityChoice::Public),
        )
        .unwrap();
    assert!(matches!(
        block.supersede(topic, old, new).unwrap_err(),
        MemoryError::ForeignConfidenceSupersedeForbidden
    ));
}

#[test]
fn a_foreign_public_or_attributed_entry_may_be_superseded() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let speaker = MemoryId::generate();
    let other = MemoryId::generate();
    let mut block = block(
        graph,
        clock,
        Teller::Participant(speaker),
        Authority::Platform,
    );
    let topic = block
        .create(Namespace::Topic.with_name("facts"), None)
        .unwrap();
    // Public and attributed entries surface to anyone regardless of teller, so consolidating them is
    // routine even when a different participant recorded them.
    let public = block
        .append(
            topic,
            "public fact",
            told(Teller::Participant(other), VisibilityChoice::Public),
        )
        .unwrap();
    let attributed = block
        .append(
            topic,
            "secondhand fact",
            told(Teller::Participant(other), VisibilityChoice::Attributed),
        )
        .unwrap();
    let fresh = block
        .append(
            topic,
            "corrected",
            told(Teller::Participant(speaker), VisibilityChoice::Public),
        )
        .unwrap();
    block.supersede(topic, public, fresh).unwrap();
    block.supersede(topic, attributed, fresh).unwrap();
}

#[test]
fn an_agent_told_confidence_may_be_superseded_in_a_participant_turn() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let speaker = MemoryId::generate();
    let mut block = block(
        graph,
        clock,
        Teller::Participant(speaker),
        Authority::Platform,
    );
    let topic = block
        .create(Namespace::Topic.with_name("notes"), None)
        .unwrap();
    // An entry the agent authored is not a participant's confidence, so the gate does not fire.
    let observed = block
        .append(
            topic,
            "agent note",
            told(Teller::Agent, VisibilityChoice::Private),
        )
        .unwrap();
    let fresh = block
        .append(
            topic,
            "revised note",
            told(Teller::Agent, VisibilityChoice::Private),
        )
        .unwrap();
    block.supersede(topic, observed, fresh).unwrap();
}

#[test]
fn operator_authority_may_supersede_a_foreign_confidence() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let other = MemoryId::generate();
    let mut block = block(graph, clock, Teller::Agent, Authority::Operator);
    let topic = block
        .create(Namespace::Topic.with_name("aside"), None)
        .unwrap();
    // The console may consolidate any entry, a foreign confidence included.
    let old = block
        .append(
            topic,
            "confided",
            told(Teller::Participant(other), VisibilityChoice::Private),
        )
        .unwrap();
    let new = block
        .append(
            topic,
            "replacement",
            told(Teller::Agent, VisibilityChoice::Public),
        )
        .unwrap();
    block.supersede(topic, old, new).unwrap();
}

#[test]
fn platform_authority_cannot_retract_a_foreign_confidence() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let speaker = MemoryId::generate();
    let other = MemoryId::generate();
    let mut block = block(
        graph,
        clock,
        Teller::Participant(speaker),
        Authority::Platform,
    );
    let topic = block
        .create(Namespace::Topic.with_name("aside"), None)
        .unwrap();
    // Retract routes the same foreign-confidence gate supersede does: the current speaker's turn cannot
    // withdraw a confidence a different participant entrusted.
    let confided = block
        .append(
            topic,
            "confided",
            told(Teller::Participant(other), VisibilityChoice::Private),
        )
        .unwrap();
    assert!(matches!(
        block
            .retract(topic, confided, "out of date", None)
            .unwrap_err(),
        MemoryError::ForeignConfidenceSupersedeForbidden
    ));
}

#[test]
fn a_teller_may_retract_their_own_confidence() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let speaker = MemoryId::generate();
    let mut block = block(
        graph,
        clock,
        Teller::Participant(speaker),
        Authority::Platform,
    );
    let topic = block
        .create(Namespace::Topic.with_name("aside"), None)
        .unwrap();
    // The confidence is the speaker's own, so withdrawing it is their own turn's business.
    let mine = block
        .append(
            topic,
            "confided",
            told(Teller::Participant(speaker), VisibilityChoice::Private),
        )
        .unwrap();
    block.retract(topic, mine, "no longer true", None).unwrap();
}

#[test]
fn a_merged_identity_counts_as_the_same_teller() {
    let (graph, quinn, quinn_chat) = graph_with_merged_pair();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    // The speaker is one stub of a merged identity; a confidence told by the other stub is their own.
    let mut block = block(
        graph,
        clock,
        Teller::Participant(quinn),
        Authority::Platform,
    );
    let topic = block
        .create(Namespace::Topic.with_name("aside"), None)
        .unwrap();
    let old = block
        .append(
            topic,
            "confided",
            told(Teller::Participant(quinn_chat), VisibilityChoice::Private),
        )
        .unwrap();
    let new = block
        .append(
            topic,
            "replacement",
            told(Teller::Participant(quinn), VisibilityChoice::Private),
        )
        .unwrap();
    block.supersede(topic, old, new).unwrap();
}

#[test]
fn revise_of_a_foreign_confidence_is_rejected_and_rolls_back() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let speaker = MemoryId::generate();
    let other = MemoryId::generate();
    let mut block = block(
        graph,
        clock,
        Teller::Participant(speaker),
        Authority::Platform,
    );
    let topic = block
        .create(Namespace::Topic.with_name("aside"), None)
        .unwrap();
    let old = block
        .append(
            topic,
            "confided",
            told(Teller::Participant(other), VisibilityChoice::Private),
        )
        .unwrap();
    // revise flows through supersede, so the foreign-confidence gate fires — and its transaction must
    // leave no half-applied replacement append behind.
    let error = block
        .revise(
            topic,
            old,
            "replacement",
            told(Teller::Participant(speaker), VisibilityChoice::Public),
        )
        .unwrap_err();
    assert!(
        matches!(error, MemoryError::ForeignConfidenceSupersedeForbidden),
        "expected the foreign-confidence gate, got {error:?}"
    );
    let effects = block.into_effects();
    let appended: Vec<&EventPayload> = effects
        .events
        .iter()
        .filter(
            |event| matches!(event, EventPayload::MemoryContentAppended { id, .. } if *id == topic),
        )
        .collect();
    assert_eq!(
        appended.len(),
        1,
        "only the original confided entry should remain; the revise's append must roll back"
    );
}
