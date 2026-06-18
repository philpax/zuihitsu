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
        EventPayload::MemoryCreated {
            id: dave,
            name: Namespace::Person.handle("dave"),
        },
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
        EventPayload::MemoryCreated {
            id: erin,
            name: Namespace::Person.handle("erin"),
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
            name: Namespace::Topic.handle("quantum-knitting"),
        },
        EventPayload::MemoryDeleted { id },
    ]);
    assert!(graph.search("quantum-knitting", 10).unwrap().is_empty());
}
