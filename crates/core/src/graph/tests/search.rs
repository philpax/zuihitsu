use super::materialized;
use crate::{
    event::{EventPayload, Teller, Visibility},
    ids::{EntryId, MemoryId, Namespace},
    time::Timestamp,
};

#[test]
fn search_matches_name_description_and_content() {
    let dave = MemoryId::generate();
    let erin = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(dave, Namespace::Person.with_name("dave")),
        EventPayload::MemoryContentAppended {
            id: dave,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(1),
            occurred_at: None,
            text: "Met at the climbing gym".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        EventPayload::memory_description_regenerated(
            erin,
            "An avid rock climber.".to_owned(),
            None,
        ),
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
        EventPayload::memory_created(id, Namespace::Topic.with_name("quantum-knitting")),
        EventPayload::memory_deleted(id),
    ]);
    assert!(graph.search("quantum-knitting", 10).unwrap().is_empty());
}

#[test]
fn search_lexical_marks_content_bearing_hits() {
    // A memory with name and public content: a query matching content only should be content-bearing,
    // and a query matching the name only should not.
    let dave = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(dave, Namespace::Person.with_name("dave")),
        EventPayload::MemoryContentAppended {
            id: dave,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(1),
            occurred_at: None,
            text: "Met at the climbing gym".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
    ]);
    // "climbing" matches content only → content_bearing = true, snippet is from content.
    let hits = graph.search_lexical("climbing", 10).unwrap();
    assert!(
        hits.iter()
            .any(|h| h.content_bearing && h.snippet.contains("climbing")),
        "a content match should be content-bearing with a content snippet: {hits:?}"
    );
    // "dave" matches name only → content_bearing = false, snippet is the name.
    let hits = graph.search_lexical("dave", 10).unwrap();
    assert!(
        hits.iter().any(|h| !h.content_bearing),
        "a name-only match should not be content-bearing: {hits:?}"
    );
}
