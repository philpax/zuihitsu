use super::{materialized, mentor_relation};
use crate::{
    event::{Cardinality, EventPayload, LinkSource},
    ids::{MemoryId, MemoryName},
    vocabulary::RelationName,
};

#[test]
fn relation_registry_records_cardinality() {
    let (_store, graph) = materialized(vec![mentor_relation()]);
    let relation = graph.relation("mentor_of").unwrap().unwrap();
    assert_eq!(relation.inverse, RelationName::new("mentored_by"));
    assert_eq!(relation.from_card, Cardinality::Many);
    assert!(!relation.symmetric);
    assert!(graph.relation("nonexistent").unwrap().is_none());
}

#[test]
fn relation_resolves_by_either_label() {
    // A relation is one thing with two labels (spec §Data model), so it must resolve by its inverse
    // label too — what lets `mem:link` be asserted under either name and `links.get` find it either
    // way. Looking up "mentored_by" returns the same canonical relation as "mentor_of".
    let (_store, graph) = materialized(vec![mentor_relation()]);
    let by_inverse = graph.relation("mentored_by").unwrap().unwrap();
    assert_eq!(by_inverse.name, RelationName::new("mentor_of"));
    assert_eq!(by_inverse.inverse, RelationName::new("mentored_by"));
}

#[test]
fn link_canonicalizes_inverse_label_to_one_edge() {
    let dave = MemoryId::generate();
    let erin = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        mentor_relation(),
        EventPayload::MemoryCreated {
            id: dave,
            name: MemoryName::new("person/dave"),
        },
        EventPayload::MemoryCreated {
            id: erin,
            name: MemoryName::new("person/erin"),
        },
        // "erin is mentored_by dave" == "dave is mentor_of erin": same canonical edge.
        EventPayload::LinkCreated {
            from: erin,
            to: dave,
            relation: RelationName::new("mentored_by"),
            source: LinkSource::Agent,
        },
    ]);

    // One stored edge, canonicalized to dave --mentor_of--> erin.
    let links = graph.links(dave).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].from, dave);
    assert_eq!(links[0].to, erin);
    assert_eq!(links[0].relation, RelationName::new("mentor_of"));

    // Traversal reads both labels: dave mentors erin; erin is mentored by dave.
    let mentees = graph.outgoing(dave, "mentor_of").unwrap();
    assert_eq!(mentees.len(), 1);
    assert_eq!(mentees[0].id, erin);

    let mentors = graph.outgoing(erin, "mentored_by").unwrap();
    assert_eq!(mentors.len(), 1);
    assert_eq!(mentors[0].id, dave);

    // The forward label from the wrong end yields nothing.
    assert!(graph.outgoing(erin, "mentor_of").unwrap().is_empty());
}

#[test]
fn symmetric_link_is_order_independent() {
    let a = MemoryId::generate();
    let b = MemoryId::generate();
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
            name: MemoryName::new("person/phil@direct"),
        },
        EventPayload::MemoryCreated {
            id: b,
            name: MemoryName::new("person/phil@discord"),
        },
        EventPayload::LinkCreated {
            from: a,
            to: b,
            relation: RelationName::SameAs,
            source: LinkSource::Operator,
        },
        // Asserting the reverse direction is the same edge, not a second one.
        EventPayload::LinkCreated {
            from: b,
            to: a,
            relation: RelationName::SameAs,
            source: LinkSource::Operator,
        },
    ]);

    assert_eq!(graph.links(a).unwrap().len(), 1);
    // Traversable from either side.
    assert_eq!(graph.outgoing(a, "same_as").unwrap().len(), 1);
    assert_eq!(graph.outgoing(b, "same_as").unwrap().len(), 1);
}

#[test]
fn link_removed_and_deleted_endpoint_drop_from_traversal() {
    let dave = MemoryId::generate();
    let erin = MemoryId::generate();
    let frank = MemoryId::generate();
    let setup = || {
        vec![
            mentor_relation(),
            EventPayload::MemoryCreated {
                id: dave,
                name: MemoryName::new("person/dave"),
            },
            EventPayload::MemoryCreated {
                id: erin,
                name: MemoryName::new("person/erin"),
            },
            EventPayload::MemoryCreated {
                id: frank,
                name: MemoryName::new("person/frank"),
            },
            EventPayload::LinkCreated {
                from: dave,
                to: erin,
                relation: RelationName::new("mentor_of"),
                source: LinkSource::Agent,
            },
            EventPayload::LinkCreated {
                from: dave,
                to: frank,
                relation: RelationName::new("mentor_of"),
                source: LinkSource::Agent,
            },
        ]
    };

    // Removing one edge leaves the other.
    let mut removed = setup();
    removed.push(EventPayload::LinkRemoved {
        from: dave,
        to: erin,
        relation: RelationName::new("mentor_of"),
    });
    let (_s1, graph) = materialized(removed);
    let mentees = graph.outgoing(dave, "mentor_of").unwrap();
    assert_eq!(mentees.len(), 1);
    assert_eq!(mentees[0].id, frank);

    // Soft-deleting a neighbour hides the edge from traversal.
    let mut deleted = setup();
    deleted.push(EventPayload::MemoryDeleted { id: frank });
    let (_s2, graph) = materialized(deleted);
    let mentees = graph.outgoing(dave, "mentor_of").unwrap();
    assert_eq!(mentees.len(), 1);
    assert_eq!(mentees[0].id, erin);
    assert_eq!(graph.links(dave).unwrap().len(), 1); // frank edge hidden
}
