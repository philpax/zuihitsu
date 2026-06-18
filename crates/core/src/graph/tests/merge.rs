use super::materialized;
use crate::{
    event::{Cardinality, EventPayload, LinkSource, Teller, Visibility},
    graph::EntryView,
    ids::{EntryId, MemoryId, Namespace},
    time::Timestamp,
    vocabulary::RelationName,
};

#[test]
fn same_as_merges_stubs_into_one_class() {
    let a = MemoryId::generate();
    let b = MemoryId::generate();
    let c = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::LinkTypeRegistered {
            name: RelationName::SameAs,
            inverse: RelationName::SameAs,
            from_card: Cardinality::Many,
            to_card: Cardinality::Many,
            symmetric: true,
            reflexive: false,
        },
        EventPayload::MemoryCreated {
            id: a,
            name: Namespace::Person.handle("phil@direct"),
        },
        EventPayload::MemoryCreated {
            id: b,
            name: Namespace::Person.handle("phil@discord"),
        },
        EventPayload::MemoryCreated {
            id: c,
            name: Namespace::Person.handle("dave@direct"),
        },
        EventPayload::LinkCreated {
            from: a,
            to: b,
            relation: RelationName::SameAs,
            source: LinkSource::Operator,
            told_by: None,
        },
    ]);

    // The two Phil stubs share one class whose id is the earliest member by ULID (the primary);
    // Dave is his own class.
    let class = graph.class_id(a).unwrap().unwrap();
    assert_eq!(graph.class_id(b).unwrap().unwrap(), class);
    assert_eq!(class, a.min(b));
    assert_eq!(graph.class_id(c).unwrap().unwrap(), c);
    assert_ne!(graph.class_id(c).unwrap().unwrap(), class);

    // Class membership is the whole class, deduplicated and ordered; a lone stub is just itself.
    let mut phil = vec![a, b];
    phil.sort();
    assert_eq!(graph.class_members(a).unwrap(), phil);
    assert_eq!(graph.class_members(b).unwrap(), phil);
    assert_eq!(graph.class_members(c).unwrap(), vec![c]);
}

#[test]
fn class_entries_compose_across_a_merged_class_in_commit_order() {
    let a = MemoryId::generate();
    let b = MemoryId::generate();
    let c = MemoryId::generate();
    let appended = |id, text: &str| EventPayload::MemoryContentAppended {
        id,
        entry_id: EntryId::generate(),
        asserted_at: Timestamp::from_millis(900),
        occurred_at: None,
        text: text.to_owned(),
        told_by: Teller::Agent,
        told_in: None,
        visibility: Visibility::Public,
    };
    let (_store, graph) = materialized(vec![
        EventPayload::LinkTypeRegistered {
            name: RelationName::SameAs,
            inverse: RelationName::SameAs,
            from_card: Cardinality::Many,
            to_card: Cardinality::Many,
            symmetric: true,
            reflexive: false,
        },
        EventPayload::MemoryCreated {
            id: a,
            name: Namespace::Person.handle("phil@direct"),
        },
        EventPayload::MemoryCreated {
            id: b,
            name: Namespace::Person.handle("phil@discord"),
        },
        EventPayload::MemoryCreated {
            id: c,
            name: Namespace::Person.handle("dave@direct"),
        },
        // Appended interleaved across the two Phil stubs to prove the union is ordered by global
        // commit order (seq), not grouped by stub.
        appended(a, "phil one"),
        appended(b, "phil two"),
        appended(a, "phil three"),
        appended(c, "dave only"),
        EventPayload::LinkCreated {
            from: a,
            to: b,
            relation: RelationName::SameAs,
            source: LinkSource::Operator,
            told_by: None,
        },
    ]);

    let texts = |entries: Vec<EntryView>| entries.into_iter().map(|e| e.text).collect::<Vec<_>>();

    // The class read unions both stubs in commit order, from either member.
    assert_eq!(
        texts(graph.class_entries(a).unwrap()),
        ["phil one", "phil two", "phil three"]
    );
    assert_eq!(
        texts(graph.class_entries(b).unwrap()),
        ["phil one", "phil two", "phil three"]
    );
    // The local read sees only its own stub.
    assert_eq!(
        texts(graph.entries_local(a).unwrap()),
        ["phil one", "phil three"]
    );
    assert_eq!(texts(graph.entries_local(b).unwrap()), ["phil two"]);
    // A singleton class: the class read equals the local read.
    assert_eq!(texts(graph.class_entries(c).unwrap()), ["dave only"]);
    assert_eq!(texts(graph.entries_local(c).unwrap()), ["dave only"]);
}
