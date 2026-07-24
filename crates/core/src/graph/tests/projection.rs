use crate::{
    event::{EventPayload, Teller, Visibility, Volatility},
    graph::{EntryView, tests::materialized},
    ids::{EntryId, MemoryId, MemoryName, Namespace},
    time::Timestamp,
    vocabulary::TagName,
};

#[test]
fn projects_create_rename_and_content() {
    let id = MemoryId::generate();
    let entry = EntryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(id, Namespace::Person.with_name("dave")),
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
        EventPayload::memory_renamed(
            id,
            Namespace::Person.with_name("dave"),
            Namespace::Person.with_name("dave-chen"),
        ),
    ]);

    // The old name no longer resolves; the new one does, to the same id.
    assert!(
        graph
            .memory_by_name(Namespace::Person.with_name("dave"))
            .unwrap()
            .is_none()
    );
    let memory = graph
        .memory_by_name(Namespace::Person.with_name("dave-chen"))
        .unwrap()
        .unwrap();
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
        EventPayload::memory_created(id, Namespace::Topic.with_name("sourdough")),
        EventPayload::memory_deleted(id),
    ]);

    assert!(
        graph
            .memory_by_name(Namespace::Topic.with_name("sourdough"))
            .unwrap()
            .is_none()
    );
    assert!(graph.memory_by_id(id).unwrap().is_none());
    assert!(
        graph
            .memories_in_namespace(Namespace::Topic.prefix())
            .unwrap()
            .is_empty()
    );
    // Contents are preserved for replay/audit even though the memory is hidden.
    // (No entries appended here, so just assert the read path doesn't error.)
    assert!(graph.entries_local(id).unwrap().is_empty());
}

#[test]
fn description_and_volatility_update_in_place() {
    let id = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(id, MemoryName::new("project/atlas")),
        EventPayload::memory_description_regenerated(
            id,
            "An ongoing migration effort.".to_owned(),
            None,
        ),
        EventPayload::memory_volatility_set(id, Volatility::High),
    ]);

    let memory = graph.memory_by_id(id).unwrap().unwrap();
    assert_eq!(memory.description, "An ongoing migration effort.");
    assert_eq!(memory.volatility, Volatility::High);
}

#[test]
fn tag_create_apply_and_remove() {
    let id = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::tag_created(
            TagName::new("hobbies"),
            "Recreational activities and interests".to_owned(),
        ),
        EventPayload::tag_created(TagName::new("colleagues"), "People worked with"),
        EventPayload::memory_created(id, Namespace::Person.with_name("erin")),
        EventPayload::tag_applied_to_memory(id, TagName::new("hobbies")),
        EventPayload::tag_applied_to_memory(id, TagName::new("colleagues")),
        EventPayload::tag_removed_from_memory(id, TagName::new("hobbies")),
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
        EventPayload::memory_created(MemoryId::generate(), Namespace::Person.with_name("dave")),
        EventPayload::memory_created(MemoryId::generate(), Namespace::Person.with_name("erin")),
        EventPayload::memory_created(MemoryId::generate(), Namespace::Place.with_name("sydney")),
    ]);

    let people = graph
        .memories_in_namespace(Namespace::Person.prefix())
        .unwrap();
    let names: Vec<&MemoryName> = people.iter().map(|m| &m.name).collect();
    assert_eq!(
        names,
        vec![
            &Namespace::Person.with_name("dave").into(),
            &Namespace::Person.with_name("erin").into(),
        ]
    );
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
        EventPayload::memory_created(dave, Namespace::Person.with_name("dave")),
        appended(old, "Dave works at Hooli"),
        appended(new, "Dave works at Pied Piper"),
        EventPayload::memory_superseded(dave, old, new),
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

#[test]
fn a_retracted_entry_drops_from_live_reads_but_stays_in_history_with_its_reason() {
    let dave = MemoryId::generate();
    let kept = EntryId::generate();
    let withdrawn = EntryId::generate();
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
        EventPayload::memory_created(dave, Namespace::Person.with_name("dave")),
        appended(kept, "Dave leads the platform team"),
        appended(withdrawn, "Dave plays the cello"),
        EventPayload::entry_retracted(dave, withdrawn, "filed on the wrong person", None),
    ]);

    let texts = |entries: Vec<EntryView>| entries.into_iter().map(|e| e.text).collect::<Vec<_>>();

    // Live reads (local and class) drop the retracted entry; history keeps both in commit order.
    assert_eq!(
        texts(graph.entries_local(dave).unwrap()),
        ["Dave leads the platform team"]
    );
    assert_eq!(
        texts(graph.class_entries(dave).unwrap()),
        ["Dave leads the platform team"]
    );
    assert_eq!(
        texts(graph.entries_local_history(dave).unwrap()),
        ["Dave leads the platform team", "Dave plays the cello"]
    );

    // The retracted entry carries its reason in history and is tombstoned by a self-referential
    // superseded_by (its own id), so every live filter hides it with no successor to point at.
    let history = graph.entries_local_history(dave).unwrap();
    let tombstone = history.iter().find(|e| e.entry_id == withdrawn).unwrap();
    assert_eq!(
        tombstone.retracted_reason.as_deref(),
        Some("filed on the wrong person")
    );
    assert_eq!(tombstone.superseded_by, Some(withdrawn));
    let live = history.iter().find(|e| e.entry_id == kept).unwrap();
    assert_eq!(live.retracted_reason, None);
    assert_eq!(live.superseded_by, None);
}

#[test]
fn name_prefix_query_matches_literally_and_orders_by_name() {
    // The range fetch compares literally, so a prefix carrying a LIKE metacharacter matches that
    // character rather than wildcarding — `person/_` must not match `person/xtest` the way an
    // unescaped LIKE pattern would.
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(MemoryId::generate(), MemoryName::new("person/_test-b")),
        EventPayload::memory_created(MemoryId::generate(), MemoryName::new("person/_test")),
        EventPayload::memory_created(MemoryId::generate(), MemoryName::new("person/xtest")),
        EventPayload::memory_created(MemoryId::generate(), MemoryName::new("person/100%-effort")),
    ]);
    let names: Vec<String> = graph
        .memory_names_with_prefix("person/_")
        .unwrap()
        .into_iter()
        .map(|name| name.as_str().to_owned())
        .collect();
    assert_eq!(names, vec!["person/_test", "person/_test-b"]);
    let names: Vec<String> = graph
        .memory_names_with_prefix("person/1")
        .unwrap()
        .into_iter()
        .map(|name| name.as_str().to_owned())
        .collect();
    assert_eq!(names, vec!["person/100%-effort"]);
}

#[test]
fn name_prefix_query_handles_a_multi_byte_first_character() {
    // A multi-byte first character must slice as a character, not a byte — the range's upper bound
    // is the next Unicode scalar value, so the fetch returns exactly the accented stem and never a
    // malformed pattern or a neighboring code point.
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(MemoryId::generate(), MemoryName::new("person/émile")),
        EventPayload::memory_created(MemoryId::generate(), MemoryName::new("person/émile-b")),
        EventPayload::memory_created(MemoryId::generate(), MemoryName::new("person/erin")),
        EventPayload::memory_created(MemoryId::generate(), MemoryName::new("person/êtienne")),
    ]);
    let names: Vec<String> = graph
        .memory_names_with_prefix("person/é")
        .unwrap()
        .into_iter()
        .map(|name| name.as_str().to_owned())
        .collect();
    assert_eq!(names, vec!["person/émile", "person/émile-b"]);
}

#[test]
fn name_prefix_query_excludes_deleted_and_scopes_by_namespace() {
    let deleted = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(MemoryId::generate(), MemoryName::new("person/dave")),
        EventPayload::memory_created(deleted, MemoryName::new("person/dave-old")),
        EventPayload::memory_created(MemoryId::generate(), MemoryName::new("place/dave-street")),
        EventPayload::memory_deleted(deleted),
    ]);
    let names: Vec<String> = graph
        .memory_names_with_prefix("person/d")
        .unwrap()
        .into_iter()
        .map(|name| name.as_str().to_owned())
        .collect();
    assert_eq!(names, vec!["person/dave"]);
}

#[test]
fn name_prefix_query_uses_the_name_index() {
    // The point of the range form over LIKE: the `name` column's BINARY collation keeps the
    // (ASCII-case-insensitive) LIKE prefix optimization off, so a LIKE fetch scans the table, while
    // the explicit range is an index search. Guard the plan so a query rewrite cannot silently
    // regress the collision-suggestion fetch to a scan.
    let (_store, graph) = materialized(vec![EventPayload::memory_created(
        MemoryId::generate(),
        MemoryName::new("person/dave"),
    )]);
    let mut stmt = graph
        .conn
        .prepare(
            "EXPLAIN QUERY PLAN SELECT name FROM memories
             WHERE name >= ?1 AND name < ?2 AND deleted = 0 ORDER BY name",
        )
        .unwrap();
    let columns = stmt.column_count();
    let mut rows = stmt
        .query(rusqlite::params!["person/d", "person/e"])
        .unwrap();
    let mut details = Vec::new();
    while let Some(row) = rows.next().unwrap() {
        details.push(row.get::<_, String>(columns - 1).unwrap());
    }
    let plan = details.join("; ");
    assert!(
        plan.contains("USING INDEX") && !plan.contains("SCAN memories"),
        "the name-prefix fetch should search the name index, got: {plan}"
    );
}
