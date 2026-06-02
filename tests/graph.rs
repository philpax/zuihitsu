//! Materialized-graph tests: events appended to a store and projected into the graph produce the
//! expected queryable state. The materializer is the one subsystem replay can't self-heal (a buggy
//! handler reproduces faithfully), so it is exercised against materialized state (spec §Storage).

#![cfg(feature = "sqlite")]

use zuihitsu::{
    Cardinality, EntryId, Graph, LinkSource, MemoryId, MemoryName, MemoryStore, RelationName, Seq,
    Store, TagName, Teller, Timestamp, Visibility, Volatility, event::EventPayload,
};

/// Standard mentor relation for the link tests: asymmetric, many-to-many.
fn mentor_relation() -> EventPayload {
    EventPayload::LinkTypeRegistered {
        name: RelationName::new("mentor_of"),
        inverse: RelationName::new("mentored_by"),
        from_card: Cardinality::Many,
        to_card: Cardinality::Many,
        symmetric: false,
        reflexive: false,
    }
}

/// Build a store, append `payloads`, materialize a fresh in-memory graph from it, and return both.
fn materialized(payloads: Vec<EventPayload>) -> (MemoryStore, Graph) {
    let mut store = MemoryStore::new();
    store
        .append(Timestamp::from_millis(1_000), payloads)
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    (store, graph)
}

#[test]
fn projects_create_rename_and_content() {
    let id = MemoryId::generate();
    let entry = EntryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::MemoryCreated {
            id,
            name: MemoryName::new("person/dave"),
        },
        EventPayload::MemoryContentAppended {
            id,
            entry_id: entry,
            asserted_at: Timestamp::from_millis(900),
            text: "Met at the climbing gym".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
        EventPayload::MemoryRenamed {
            id,
            old_name: MemoryName::new("person/dave"),
            new_name: MemoryName::new("person/dave-chen"),
        },
    ]);

    // The old name no longer resolves; the new one does, to the same id.
    assert!(graph.memory_by_name("person/dave").unwrap().is_none());
    let memory = graph.memory_by_name("person/dave-chen").unwrap().unwrap();
    assert_eq!(memory.id, id);
    assert_eq!(memory.volatility, Volatility::Medium); // default
    assert_eq!(memory.description, ""); // no regeneration yet

    let entries = graph.entries(id).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].entry_id, entry);
    assert_eq!(entries[0].text, "Met at the climbing gym");
}

#[test]
fn soft_delete_hides_from_reads() {
    let id = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::MemoryCreated {
            id,
            name: MemoryName::new("topic/sourdough"),
        },
        EventPayload::MemoryDeleted { id },
    ]);

    assert!(graph.memory_by_name("topic/sourdough").unwrap().is_none());
    assert!(graph.memory_by_id(id).unwrap().is_none());
    assert!(graph.memories_in_namespace("topic/").unwrap().is_empty());
    // Contents are preserved for replay/audit even though the memory is hidden.
    // (No entries appended here, so just assert the read path doesn't error.)
    assert!(graph.entries(id).unwrap().is_empty());
}

#[test]
fn description_and_volatility_update_in_place() {
    let id = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::MemoryCreated {
            id,
            name: MemoryName::new("project/atlas"),
        },
        EventPayload::MemoryDescriptionRegenerated {
            id,
            new_text: "An ongoing migration effort.".to_owned(),
            produced_by: None,
        },
        EventPayload::MemoryVolatilitySet {
            id,
            volatility: Volatility::High,
        },
    ]);

    let memory = graph.memory_by_id(id).unwrap().unwrap();
    assert_eq!(memory.description, "An ongoing migration effort.");
    assert_eq!(memory.volatility, Volatility::High);
}

#[test]
fn tag_create_apply_and_remove() {
    let id = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::TagCreated {
            name: TagName::new("hobbies"),
            description: "Recreational activities and interests".to_owned(),
        },
        EventPayload::TagCreated {
            name: TagName::new("colleagues"),
            description: "People worked with".to_owned(),
        },
        EventPayload::MemoryCreated {
            id,
            name: MemoryName::new("person/erin"),
        },
        EventPayload::TagAppliedToMemory {
            memory: id,
            tag: TagName::new("hobbies"),
        },
        EventPayload::TagAppliedToMemory {
            memory: id,
            tag: TagName::new("colleagues"),
        },
        EventPayload::TagRemovedFromMemory {
            memory: id,
            tag: TagName::new("hobbies"),
        },
    ]);

    // Application never mutates the tag's own description (the create/apply split).
    assert_eq!(
        graph.tag_description("hobbies").unwrap().as_deref(),
        Some("Recreational activities and interests")
    );
    let memory = graph.memory_by_id(id).unwrap().unwrap();
    assert_eq!(memory.tags, vec![TagName::new("colleagues")]); // hobbies removed; sorted
}

#[test]
fn namespace_query_scopes_by_prefix() {
    let (_store, graph) = materialized(vec![
        EventPayload::MemoryCreated {
            id: MemoryId::generate(),
            name: MemoryName::new("person/dave"),
        },
        EventPayload::MemoryCreated {
            id: MemoryId::generate(),
            name: MemoryName::new("person/erin"),
        },
        EventPayload::MemoryCreated {
            id: MemoryId::generate(),
            name: MemoryName::new("place/sydney"),
        },
    ]);

    let people = graph.memories_in_namespace("person/").unwrap();
    let names: Vec<&str> = people.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, vec!["person/dave", "person/erin"]);
}

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
            name: RelationName::new("same_as"),
            inverse: RelationName::new("same_as"),
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
            relation: RelationName::new("same_as"),
            source: LinkSource::Debugger,
        },
        // Asserting the reverse direction is the same edge, not a second one.
        EventPayload::LinkCreated {
            from: b,
            to: a,
            relation: RelationName::new("same_as"),
            source: LinkSource::Debugger,
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

#[test]
fn search_matches_name_description_and_content() {
    let dave = MemoryId::generate();
    let erin = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::MemoryCreated {
            id: dave,
            name: MemoryName::new("person/dave"),
        },
        EventPayload::MemoryContentAppended {
            id: dave,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(1),
            text: "Met at the climbing gym".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
        EventPayload::MemoryCreated {
            id: erin,
            name: MemoryName::new("person/erin"),
        },
        EventPayload::MemoryDescriptionRegenerated {
            id: erin,
            new_text: "An avid rock climber.".to_owned(),
            produced_by: None,
        },
    ]);

    // Content hit.
    let gym = graph.search("climbing", 10).unwrap();
    let gym_ids: Vec<MemoryId> = gym.iter().map(|m| m.id).collect();
    assert!(gym_ids.contains(&dave));

    // Description hit.
    let climber = graph.search("rock", 10).unwrap();
    assert_eq!(climber.iter().map(|m| m.id).collect::<Vec<_>>(), vec![erin]);

    // Name hit, and an empty query returns nothing.
    assert!(!graph.search("dave", 10).unwrap().is_empty());
    assert!(graph.search("", 10).unwrap().is_empty());
}

#[test]
fn search_excludes_soft_deleted() {
    let id = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::MemoryCreated {
            id,
            name: MemoryName::new("topic/quantum-knitting"),
        },
        EventPayload::MemoryDeleted { id },
    ]);
    assert!(graph.search("quantum-knitting", 10).unwrap().is_empty());
}

#[test]
fn materialize_is_incremental() {
    let id = MemoryId::generate();
    let mut store = MemoryStore::new();
    let mut graph = Graph::open_in_memory().unwrap();

    store
        .append(
            Timestamp::from_millis(1),
            vec![EventPayload::MemoryCreated {
                id,
                name: MemoryName::new("concept/recursion"),
            }],
        )
        .unwrap();
    assert_eq!(graph.materialize_from(&store).unwrap(), 1);
    assert_eq!(graph.head().unwrap(), Seq(1));

    // A second pass with no new events applies nothing and leaves the head where it was.
    assert_eq!(graph.materialize_from(&store).unwrap(), 0);

    store
        .append(
            Timestamp::from_millis(2),
            vec![EventPayload::MemoryDescriptionRegenerated {
                id,
                new_text: "A function defined in terms of itself.".to_owned(),
                produced_by: None,
            }],
        )
        .unwrap();
    assert_eq!(graph.materialize_from(&store).unwrap(), 1);
    assert_eq!(graph.head().unwrap(), Seq(2));
    assert_eq!(
        graph.memory_by_id(id).unwrap().unwrap().description,
        "A function defined in terms of itself."
    );
}
