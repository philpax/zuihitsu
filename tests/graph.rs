//! Materialized-graph tests: events appended to a store and projected into the graph produce the
//! expected queryable state. The materializer is the one subsystem replay can't self-heal (a buggy
//! handler reproduces faithfully), so it is exercised against materialized state (spec §Storage).

#![cfg(feature = "sqlite")]

use zuihitsu::event::EventPayload;
use zuihitsu::{
    EntryId, Graph, MemoryId, MemoryName, MemoryStore, Seq, Store, TagName, Timestamp, Volatility,
};

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
