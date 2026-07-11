//! Name- and tag-collision suggestions: a `create` that collides surfaces the near-matching existing
//! handles (or tags) so the agent picks a distinguishing name rather than colliding blind.

use super::{Authority, MemoryError, block};
use crate::{
    clock::ManualClock,
    event::{EventPayload, Teller},
    graph::Graph,
    ids::{MemoryId, MemoryName, Namespace},
    memory::memory_block::suggest::most_similar,
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

#[test]
fn push_down_matches_the_full_namespace_ranking() {
    // The candidate fetch slices the namespace to the attempted subject's first character; this
    // must be invisible in the output — ranking the slice yields exactly what ranking the whole
    // namespace yields, since both relevance gates require that shared first character. The
    // population mixes a case variant, near and far stems, a metacharacter handle, a multi-byte
    // handle, and another namespace.
    let all_names = [
        "person/dave",
        "person/Dave-caps",
        "person/dave-chen",
        "person/dave-patel",
        "person/davina",
        "person/quinn",
        "person/_test",
        "person/émile",
        "place/dave-street",
    ];
    let graph = graph_with_names(&all_names);
    let mut block = platform_block(graph);
    let error = block
        .create(MemoryName::new("person/dave"), None)
        .unwrap_err();
    let MemoryError::NameExists { similar, .. } = &error else {
        panic!("expected NameExists, got {error:?}");
    };
    let prefix = Namespace::Person.prefix();
    let expected = most_similar(
        "dave",
        all_names
            .iter()
            .filter_map(|name| {
                let subject = name.strip_prefix(prefix)?;
                Some((subject.to_owned(), MemoryName::new(*name)))
            })
            .collect(),
    );
    assert!(
        !expected.is_empty(),
        "the equivalence check must not be vacuous"
    );
    assert_eq!(similar, &expected);
}

#[test]
fn a_multi_byte_first_character_collision_suggests_its_stem() {
    // The first-character slice is a character, not a byte — an accented stem fetches cleanly and
    // suggests its neighbour.
    let graph = graph_with_names(&["person/émile", "person/émile-b", "person/erin"]);
    let mut block = platform_block(graph);
    let error = block
        .create(MemoryName::new("person/émile"), None)
        .unwrap_err();
    let MemoryError::NameExists { similar, .. } = &error else {
        panic!("expected NameExists, got {error:?}");
    };
    let similar: Vec<&str> = similar.iter().map(MemoryName::as_str).collect();
    assert_eq!(similar, vec!["person/émile-b"]);
}

#[test]
fn a_metacharacter_handle_collision_matches_literally() {
    // A subject beginning with a LIKE metacharacter slices literally — `person/_test` fetches the
    // `_`-stem, never wildcard-matching `person/xtest`.
    let graph = graph_with_names(&["person/_test", "person/_test-b", "person/xtest"]);
    let mut block = platform_block(graph);
    let error = block
        .create(MemoryName::new("person/_test"), None)
        .unwrap_err();
    let MemoryError::NameExists { similar, .. } = &error else {
        panic!("expected NameExists, got {error:?}");
    };
    let similar: Vec<&str> = similar.iter().map(MemoryName::as_str).collect();
    assert_eq!(similar, vec!["person/_test-b"]);
}

#[test]
fn an_empty_subject_collision_falls_back_to_the_whole_namespace() {
    // A handle with an empty subject has no first character to slice on, so the fetch falls back
    // to ranking the whole namespace — the pre-push-down behavior.
    let graph = graph_with_names(&["person/", "person/a"]);
    let mut block = platform_block(graph);
    let error = block.create(MemoryName::new("person/"), None).unwrap_err();
    let MemoryError::NameExists { similar, .. } = &error else {
        panic!("expected NameExists, got {error:?}");
    };
    let similar: Vec<&str> = similar.iter().map(MemoryName::as_str).collect();
    assert_eq!(similar, vec!["person/a"]);
}
