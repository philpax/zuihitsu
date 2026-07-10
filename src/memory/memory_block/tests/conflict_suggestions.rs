//! Name- and tag-collision suggestions: a `create` that collides surfaces the near-matching existing
//! handles (or tags) so the agent picks a distinguishing name rather than colliding blind.

use super::{Authority, MemoryError, block};
use crate::{
    clock::ManualClock,
    event::{EventPayload, Teller},
    graph::Graph,
    ids::{MemoryId, MemoryName, Namespace},
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::TagName,
};

/// A graph seeded with a committed memory per name in `names` — enough for the collision path to read
/// the namespace and rank its neighbours.
fn graph_with_names(names: &[&str]) -> Graph {
    let mut store = MemoryStore::new();
    let events = names
        .iter()
        .map(|name| EventPayload::memory_created(MemoryId::generate(), MemoryName::new(*name)))
        .collect();
    store.append(Timestamp::from_millis(1_000), events).unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    graph
}

/// A graph seeded with a committed tag per name in `names`.
fn graph_with_tags(names: &[&str]) -> Graph {
    let mut store = MemoryStore::new();
    let events = names
        .iter()
        .map(|name| EventPayload::tag_created(TagName::new(name), "a seeded purpose"))
        .collect();
    store.append(Timestamp::from_millis(1_000), events).unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    graph
}

fn platform_block(graph: Graph) -> super::MemoryBlock {
    block(
        graph,
        ManualClock::new(Timestamp::from_millis(2_000)),
        Teller::Agent,
        Authority::Platform,
    )
}

#[test]
fn a_name_collision_suggests_same_namespace_neighbours() {
    let graph = graph_with_names(&[
        "person/dave-chen",
        "person/dave-patel",
        "person/quinn",
        "place/dave-street",
    ]);
    let mut block = platform_block(graph);
    // Colliding on an existing handle surfaces its namespace neighbours, closest stem first, and never
    // the unrelated `person/quinn` or the same-stem handle in another namespace (`place/dave-street`).
    let error = block
        .create(MemoryName::new("person/dave-chen"), None)
        .unwrap_err();
    let MemoryError::NameExists { name, similar } = &error else {
        panic!("expected NameExists, got {error:?}");
    };
    assert_eq!(name.as_str(), "person/dave-chen");
    let similar: Vec<&str> = similar.iter().map(MemoryName::as_str).collect();
    assert_eq!(similar, vec!["person/dave-patel"]);
    assert!(
        error
            .to_string()
            .contains("similar existing handles: person/dave-patel")
    );
}

#[test]
fn a_name_collision_excludes_the_exact_collider_from_its_suggestions() {
    let graph = graph_with_names(&["person/dave", "person/dave-chen", "person/dave-patel"]);
    let mut block = platform_block(graph);
    let error = block
        .create(MemoryName::new("person/dave"), None)
        .unwrap_err();
    let MemoryError::NameExists { similar, .. } = &error else {
        panic!("expected NameExists, got {error:?}");
    };
    let similar: Vec<&str> = similar.iter().map(MemoryName::as_str).collect();
    assert!(
        !similar.contains(&"person/dave"),
        "the collider must not suggest itself"
    );
    assert_eq!(similar, vec!["person/dave-chen", "person/dave-patel"]);
}

#[test]
fn a_name_collision_with_no_neighbours_lists_nothing() {
    let graph = graph_with_names(&["person/quinn"]);
    let mut block = platform_block(graph);
    let error = block
        .create(MemoryName::new("person/quinn"), None)
        .unwrap_err();
    let MemoryError::NameExists { similar, .. } = &error else {
        panic!("expected NameExists, got {error:?}");
    };
    assert!(similar.is_empty());
    assert!(!error.to_string().contains("similar existing"));
}

#[test]
fn a_rename_collision_suggests_neighbours() {
    let graph = graph_with_names(&["topic/plan-q1", "topic/plan-q2"]);
    let mut block = platform_block(graph);
    let moving = block
        .create(Namespace::Topic.with_name("draft"), None)
        .unwrap();
    let error = block.rename(moving, "topic/plan-q1").unwrap_err();
    let MemoryError::NameExists { name, similar } = &error else {
        panic!("expected NameExists, got {error:?}");
    };
    assert_eq!(name.as_str(), "topic/plan-q1");
    let similar: Vec<&str> = similar.iter().map(MemoryName::as_str).collect();
    assert_eq!(similar, vec!["topic/plan-q2"]);
}

#[test]
fn a_tag_collision_suggests_similar_tags() {
    let graph = graph_with_tags(&["meeting", "meetings", "calendar"]);
    let mut block = platform_block(graph);
    let error = block
        .create_tag(TagName::new("meeting"), "a fresh purpose")
        .unwrap_err();
    let MemoryError::TagExists { name, similar } = &error else {
        panic!("expected TagExists, got {error:?}");
    };
    assert_eq!(name.as_str(), "meeting");
    let similar: Vec<&str> = similar.iter().map(TagName::as_str).collect();
    assert_eq!(similar, vec!["meetings"]);
    assert!(
        error
            .to_string()
            .contains("similar existing tags: meetings")
    );
}
