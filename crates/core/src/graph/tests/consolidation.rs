use crate::{
    event::{EventPayload, Teller, Visibility},
    graph::tests::materialized,
    ids::{EntryId, MemoryId, Namespace},
    time::Timestamp,
};

#[test]
fn entries_consolidated_tombstones_sources_and_records_the_relationship() {
    let id = MemoryId::generate();
    let source_a = EntryId::generate();
    let source_b = EntryId::generate();
    let replacement = EntryId::generate();
    let now = Timestamp::from_millis(1_000);

    let append = |entry, text: &str| EventPayload::MemoryContentAppended {
        id,
        entry_id: entry,
        asserted_at: now,
        occurred_at: None,
        text: text.to_owned(),
        told_by: Teller::Agent,
        told_in: None,
        visibility: Visibility::Public,
    };

    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(id, Namespace::Person.with_name("person/alex")),
        append(source_a, "Alex is a backend engineer"),
        append(source_b, "Alex works on the backend"),
        // The replacement entry is appended first, as a normal content append.
        append(
            replacement,
            "Alex is a backend engineer who works on the backend",
        ),
        // Then the consolidation event tombstones the sources.
        EventPayload::entries_consolidated(id, vec![source_a, source_b], replacement, None),
    ]);

    // The replacement is live (no superseded_by).
    let live = graph.class_entries(id).unwrap();
    assert_eq!(live.len(), 1, "only the replacement is live");
    assert_eq!(live[0].entry_id, replacement);

    // The sources are tombstoned — their `superseded_by` points to the replacement.
    let sources = graph.consolidation_sources(replacement).unwrap();
    assert_eq!(sources.len(), 2, "both sources are recoverable");
    let source_ids: Vec<_> = sources.iter().map(|e| e.entry_id).collect();
    assert!(source_ids.contains(&source_a));
    assert!(source_ids.contains(&source_b));
}

#[test]
fn entries_consolidated_with_no_sources_is_a_noop_on_live_entries() {
    // An EntriesConsolidated with an empty sources list tombstones nothing — the replacement (if
    // it was appended) stays live alongside any pre-existing entries, and consolidation_sources
    // returns nothing.
    let id = MemoryId::generate();
    let replacement = EntryId::generate();
    let now = Timestamp::from_millis(1_000);

    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(id, Namespace::Person.with_name("person/blake")),
        EventPayload::MemoryContentAppended {
            id,
            entry_id: replacement,
            asserted_at: now,
            occurred_at: None,
            text: "standalone entry".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
        EventPayload::entries_consolidated(id, Vec::new(), replacement, None),
    ]);

    let live = graph.class_entries(id).unwrap();
    assert_eq!(live.len(), 1, "the replacement is live");
    assert_eq!(live[0].entry_id, replacement);
    assert!(
        graph.consolidation_sources(replacement).unwrap().is_empty(),
        "no sources to recover"
    );
}
