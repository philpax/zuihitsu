//! The confidential-untag and foreign-confidence supersede guards (issue #16).

use super::{Authority, MemoryError, VisibilityChoice, block, graph_with_merged_pair, told};
use crate::{
    clock::ManualClock,
    event::{EventPayload, Teller},
    graph::Graph,
    ids::{MemoryId, Namespace},
    time::Timestamp,
    vocabulary::TagName,
};

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
fn a_merged_identity_counts_as_the_same_teller() {
    let (graph, quinn, quinn_discord) = graph_with_merged_pair();
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
            told(
                Teller::Participant(quinn_discord),
                VisibilityChoice::Private,
            ),
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
