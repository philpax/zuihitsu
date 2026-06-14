use super::materialized;
use crate::{
    event::{EventPayload, Teller, Visibility, Volatility},
    graph::EntryView,
    ids::{EntryId, MemoryId, MemoryName},
    time::Timestamp,
    vocabulary::TagName,
};

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
            occurred_at: None,
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

    let entries = graph.entries_local(id).unwrap();
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
    assert!(graph.entries_local(id).unwrap().is_empty());
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
fn a_superseded_entry_drops_from_live_reads_but_stays_in_history() {
    let dave = MemoryId::generate();
    let old = EntryId::generate();
    let new = EntryId::generate();
    let appended = |entry_id, text: &str| EventPayload::MemoryContentAppended {
        id: dave,
        entry_id,
        asserted_at: Timestamp::from_millis(900),
        occurred_at: None,
        text: text.to_owned(),
        told_by: Teller::Agent,
        told_in: None,
        visibility: Visibility::Public,
    };
    let (_store, graph) = materialized(vec![
        EventPayload::MemoryCreated {
            id: dave,
            name: MemoryName::new("person/dave"),
        },
        appended(old, "Dave works at Hooli"),
        appended(new, "Dave works at Pied Piper"),
        EventPayload::MemorySuperseded {
            id: dave,
            entry: old,
            superseded_by: new,
        },
    ]);

    let texts = |entries: Vec<EntryView>| entries.into_iter().map(|e| e.text).collect::<Vec<_>>();

    // Live reads (local and class) drop the superseded entry; history keeps both in commit order.
    assert_eq!(
        texts(graph.entries_local(dave).unwrap()),
        ["Dave works at Pied Piper"]
    );
    assert_eq!(
        texts(graph.class_entries(dave).unwrap()),
        ["Dave works at Pied Piper"]
    );
    assert_eq!(
        texts(graph.entries_local_history(dave).unwrap()),
        ["Dave works at Hooli", "Dave works at Pied Piper"]
    );
    assert_eq!(
        texts(graph.class_history(dave).unwrap()),
        ["Dave works at Hooli", "Dave works at Pied Piper"]
    );

    // The superseded entry's pointer is stamped in history; the live one is unmarked.
    let history = graph.entries_local_history(dave).unwrap();
    let superseded = history.iter().find(|e| e.entry_id == old).unwrap();
    assert_eq!(superseded.superseded_by, Some(new));
    let live = history.iter().find(|e| e.entry_id == new).unwrap();
    assert_eq!(live.superseded_by, None);

    // A direct entry lookup still resolves the superseded entry (the search path filters it through
    // the visibility predicate, not the lookup).
    let (_memory, entry) = graph.entry_by_id(old).unwrap().unwrap();
    assert_eq!(entry.superseded_by, Some(new));
}
