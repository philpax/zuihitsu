//! Materialized-graph tests: events appended to a store and projected into the graph produce the
//! expected queryable state. The materializer is the one subsystem replay can't self-heal (a buggy
//! handler reproduces faithfully), so it is exercised against materialized state (spec §Storage).

use super::Graph;
use crate::{
    event::{Cardinality, EventPayload, LinkSource, Teller, Visibility, Volatility},
    ids::{EntryId, MemoryId, Namespace},
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::{RelationName, TagName},
};

mod describe;
mod merge;
mod occurrence;
mod projection;
mod relations;
mod replay;
mod search;

/// Standard mentor relation for the link tests: asymmetric, many-to-many.
pub(super) fn mentor_relation() -> EventPayload {
    EventPayload::LinkTypeRegistered {
        name: RelationName::new("mentor_of"),
        inverse: RelationName::new("mentored_by"),
        from_card: Cardinality::Many,
        to_card: Cardinality::Many,
        symmetric: false,
        reflexive: false,
        description: String::new(),
    }
}

/// Build a store, append `payloads`, materialize a fresh in-memory graph from it, and return both.
pub(super) fn materialized(payloads: Vec<EventPayload>) -> (MemoryStore, Graph) {
    let mut store = MemoryStore::new();
    store
        .append(Timestamp::from_millis(1_000), payloads)
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    (store, graph)
}

/// A log exercising a broad slice of the materializer — creates, varied content, a description,
/// volatility, a tag created and applied, a registered relation and a link, a supersession, and a soft
/// delete — so a rebuild over it stresses most projection handlers, not just create-and-append.
pub(super) fn recovery_log() -> Vec<EventPayload> {
    let dave = MemoryId::generate();
    let erin = MemoryId::generate();
    let hooli = MemoryId::generate();
    let (e1, e2, e3) = (
        EntryId::generate(),
        EntryId::generate(),
        EntryId::generate(),
    );
    let appended = |id, entry_id, text: &str, visibility| EventPayload::MemoryContentAppended {
        id,
        entry_id,
        asserted_at: Timestamp::from_millis(900),
        occurred_at: None,
        text: text.to_owned(),
        told_by: Teller::Agent,
        told_in: None,
        visibility,
    };
    vec![
        EventPayload::memory_created(dave, Namespace::Person.with_name("dave")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        EventPayload::memory_created(hooli, Namespace::Place.with_name("hooli")),
        appended(dave, e1, "Met at the climbing gym", Visibility::Public),
        appended(dave, e2, "Now works at Hooli", Visibility::Public),
        appended(
            erin,
            e3,
            "An old friend of Dave's",
            Visibility::PrivateToTeller,
        ),
        EventPayload::memory_description_regenerated(
            dave,
            "A climber who works at Hooli".to_owned(),
            None,
        ),
        EventPayload::memory_volatility_set(erin, Volatility::High),
        EventPayload::tag_created(TagName::new("colleagues"), "People worked with"),
        EventPayload::tag_applied_to_memory(dave, TagName::new("colleagues")),
        mentor_relation(),
        EventPayload::LinkCreated {
            from: dave,
            to: erin,
            relation: RelationName::new("mentor_of"),
            source: LinkSource::Agent,
            told_by: None,
        },
        // A describer pass over both people, so a rebuild stresses the described-state handler too.
        EventPayload::describe_pass_completed(vec![dave, erin]),
        // Supersede dave's first entry with his second — it drops from live reads but stays recorded.
        EventPayload::memory_superseded(dave, e1, e2),
        // Soft-delete a memory — filtered from reads, retained in the tables.
        EventPayload::memory_deleted(hooli),
    ]
}
