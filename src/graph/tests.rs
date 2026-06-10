//! Materialized-graph tests: events appended to a store and projected into the graph produce the
//! expected queryable state. The materializer is the one subsystem replay can't self-heal (a buggy
//! handler reproduces faithfully), so it is exercised against materialized state (spec §Storage).

use super::{EntryView, Graph};
use crate::{
    event::{Cardinality, EventPayload, LinkSource, Teller, Visibility, Volatility},
    ids::{
        ConversationId, ConversationLocator, EntryId, MemoryId, MemoryName, Seq, SessionId, TurnId,
    },
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::{RelationName, TagName},
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
            source: LinkSource::Debugger,
        },
        // Asserting the reverse direction is the same edge, not a second one.
        EventPayload::LinkCreated {
            from: b,
            to: a,
            relation: RelationName::SameAs,
            source: LinkSource::Debugger,
        },
    ]);

    assert_eq!(graph.links(a).unwrap().len(), 1);
    // Traversable from either side.
    assert_eq!(graph.outgoing(a, "same_as").unwrap().len(), 1);
    assert_eq!(graph.outgoing(b, "same_as").unwrap().len(), 1);
}

#[test]
fn same_as_merges_stubs_into_one_class() {
    let a = MemoryId::generate();
    let b = MemoryId::generate();
    let c = MemoryId::generate();
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
        EventPayload::MemoryCreated {
            id: c,
            name: MemoryName::new("person/dave@direct"),
        },
        EventPayload::LinkCreated {
            from: a,
            to: b,
            relation: RelationName::SameAs,
            source: LinkSource::Debugger,
        },
    ]);

    // The two Phil stubs share one class whose id is the earliest member by ULID (the primary);
    // Dave is his own class.
    let class = graph.class_id(a).unwrap().unwrap();
    assert_eq!(graph.class_id(b).unwrap().unwrap(), class);
    assert_eq!(class, a.min(b));
    assert_eq!(graph.class_id(c).unwrap().unwrap(), c);
    assert_ne!(graph.class_id(c).unwrap().unwrap(), class);

    // Class membership is the whole class, deduplicated and ordered; a lone stub is just itself.
    let mut phil = vec![a, b];
    phil.sort();
    assert_eq!(graph.class_members(a).unwrap(), phil);
    assert_eq!(graph.class_members(b).unwrap(), phil);
    assert_eq!(graph.class_members(c).unwrap(), vec![c]);
}

#[test]
fn class_entries_compose_across_a_merged_class_in_commit_order() {
    let a = MemoryId::generate();
    let b = MemoryId::generate();
    let c = MemoryId::generate();
    let appended = |id, text: &str| EventPayload::MemoryContentAppended {
        id,
        entry_id: EntryId::generate(),
        asserted_at: Timestamp::from_millis(900),
        occurred_at: None,
        text: text.to_owned(),
        told_by: Teller::Agent,
        told_in: None,
        visibility: Visibility::Public,
    };
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
        EventPayload::MemoryCreated {
            id: c,
            name: MemoryName::new("person/dave@direct"),
        },
        // Appended interleaved across the two Phil stubs to prove the union is ordered by global
        // commit order (seq), not grouped by stub.
        appended(a, "phil one"),
        appended(b, "phil two"),
        appended(a, "phil three"),
        appended(c, "dave only"),
        EventPayload::LinkCreated {
            from: a,
            to: b,
            relation: RelationName::SameAs,
            source: LinkSource::Debugger,
        },
    ]);

    let texts = |entries: Vec<EntryView>| entries.into_iter().map(|e| e.text).collect::<Vec<_>>();

    // The class read unions both stubs in commit order, from either member.
    assert_eq!(
        texts(graph.class_entries(a).unwrap()),
        ["phil one", "phil two", "phil three"]
    );
    assert_eq!(
        texts(graph.class_entries(b).unwrap()),
        ["phil one", "phil two", "phil three"]
    );
    // The local read sees only its own stub.
    assert_eq!(
        texts(graph.entries_local(a).unwrap()),
        ["phil one", "phil three"]
    );
    assert_eq!(texts(graph.entries_local(b).unwrap()), ["phil two"]);
    // A singleton class: the class read equals the local read.
    assert_eq!(texts(graph.class_entries(c).unwrap()), ["dave only"]);
    assert_eq!(texts(graph.entries_local(c).unwrap()), ["dave only"]);
}

#[test]
fn a_snapshot_round_trips_the_graph_and_its_head() {
    let id = MemoryId::generate();
    let (store, graph) = materialized(vec![
        EventPayload::MemoryCreated {
            id,
            name: MemoryName::new("person/dave"),
        },
        EventPayload::MemoryContentAppended {
            id,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(900),
            occurred_at: None,
            text: "Met at the climbing gym".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
    ]);
    let head = graph.head().unwrap();
    assert!(head > Seq::ZERO);

    // VACUUM INTO a fresh file, then open it as a graph: its whole logical state round-trips.
    let path = std::env::temp_dir().join(format!(
        "zuihitsu-graphsnap-{}.sqlite",
        MemoryId::generate().0
    ));
    graph.snapshot_into(&path).unwrap();
    let mut restored = Graph::open(&path).unwrap();
    assert_eq!(restored.head().unwrap(), head);
    // The content fingerprint matches exactly — the entire logical state round-tripped, not just the
    // few fields a spot check would cover.
    assert_eq!(
        restored.fingerprint().unwrap(),
        graph.fingerprint().unwrap()
    );
    // Materializing the restored graph against the same log is a no-op — it is already at head, so a
    // boot from this snapshot replays only the (empty here) tail rather than the whole log.
    assert_eq!(restored.materialize_from(&store).unwrap(), 0);

    std::fs::remove_file(&path).unwrap();
}

#[test]
fn fingerprint_equals_for_identical_state_and_differs_on_change() {
    let id = MemoryId::generate();
    let base = vec![
        EventPayload::MemoryCreated {
            id,
            name: MemoryName::new("person/dave"),
        },
        EventPayload::MemoryContentAppended {
            id,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(900),
            occurred_at: None,
            text: "Met at the gym".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
    ];

    // Two graphs materialized from the same events (same ids) fingerprint identically.
    let (_store_a, a) = materialized(base.clone());
    let (_store_b, b) = materialized(base.clone());
    assert_eq!(a.fingerprint().unwrap(), b.fingerprint().unwrap());

    // One more event — and the head it advances — diverges the fingerprint.
    let mut more = base;
    more.push(EventPayload::MemoryVolatilitySet {
        id,
        volatility: Volatility::High,
    });
    let (_store_c, c) = materialized(more);
    assert_ne!(a.fingerprint().unwrap(), c.fingerprint().unwrap());
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
            occurred_at: None,
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

#[test]
fn conversations_and_sessions_project() {
    let conv = ConversationId::generate();
    let context = MemoryId::generate();
    let (s1, s2) = (SessionId::generate(), SessionId::generate());
    let alice = MemoryId::generate();
    let bob = MemoryId::generate();
    let carol = MemoryId::generate();
    let join_turn = TurnId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::MemoryCreated {
            id: context,
            name: MemoryName::new("context/discord:guild/42/chan/leads"),
        },
        EventPayload::ConversationStarted {
            id: conv,
            locator: ConversationLocator::new("discord", "guild/42/chan/leads"),
            context_memory: context,
        },
        EventPayload::SessionStarted {
            conversation: conv,
            id: s1,
            participants: vec![alice, bob],
            started_at: Timestamp::from_millis(1_000),
            seeded_from_turn: None,
            brief: "first brief".to_owned(),
        },
        EventPayload::ParticipantJoined {
            conversation: conv,
            session: s1,
            participant: carol,
            at_turn: join_turn,
        },
        EventPayload::SessionEnded {
            conversation: conv,
            id: s1,
        },
        // A second session opened via compaction carries the carryover extent.
        EventPayload::SessionStarted {
            conversation: conv,
            id: s2,
            participants: vec![alice],
            started_at: Timestamp::from_millis(5_000),
            seeded_from_turn: Some(join_turn),
            brief: "second brief".to_owned(),
        },
    ]);

    // The locator resolves to the room; an unseen locator does not.
    assert_eq!(
        graph
            .conversation_for_locator(&ConversationLocator::new("discord", "guild/42/chan/leads"))
            .unwrap(),
        Some(conv)
    );
    assert!(
        graph
            .conversation_for_locator(&ConversationLocator::new("discord", "elsewhere"))
            .unwrap()
            .is_none()
    );
    // The room resolves to its eagerly-minted context memory.
    assert_eq!(graph.context_for_conversation(conv).unwrap(), Some(context));

    // Sessions project in commit order, carrying the brief and the carryover extent.
    let sessions = graph.sessions_in(conv).unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].id, s1);
    assert_eq!(sessions[0].brief, "first brief");
    assert_eq!(sessions[0].seeded_from_turn, None);
    assert_eq!(sessions[1].id, s2);
    assert_eq!(sessions[1].seeded_from_turn, Some(join_turn));

    // The first session's participants are the open set plus the mid-session joiner.
    let mut expected = vec![alice, bob, carol];
    expected.sort();
    assert_eq!(graph.session_participants(s1).unwrap(), expected);
    assert_eq!(graph.session(s1).unwrap().unwrap().participants, expected);
    // The second session has only its open participant.
    assert_eq!(graph.session_participants(s2).unwrap(), vec![alice]);
}

/// Bi-temporal `occurred_at` (Stage 9): the materializer denormalizes each entry's typed occurrence
/// into the sortable `occurred_sort` column read-side views expose.
mod occurrence {
    use super::*;
    use crate::time::{BEFORE_AFTER_EPSILON_MILLIS, CivilDate, Direction, Rrule, TemporalRef};

    const DAY: i64 = 86_400_000;

    fn created(id: MemoryId, name: &str) -> EventPayload {
        EventPayload::MemoryCreated {
            id,
            name: MemoryName::new(name),
        }
    }

    fn appended(id: MemoryId, entry_id: EntryId, occurred_at: Option<TemporalRef>) -> EventPayload {
        EventPayload::MemoryContentAppended {
            id,
            entry_id,
            asserted_at: Timestamp::from_millis(1),
            occurred_at,
            text: "fact".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        }
    }

    #[test]
    fn denormalizes_each_variant_to_its_representative_instant() {
        let id = MemoryId::generate();
        let refs = [
            Some(TemporalRef::Instant(Timestamp::from_millis(1_000))),
            Some(TemporalRef::Day(CivilDate("2026-06-03".into()))),
            Some(TemporalRef::Approx {
                center: Timestamp::from_millis(10 * DAY),
                fuzz_days: 2,
            }),
            Some(TemporalRef::Range {
                start: Timestamp::from_millis(0),
                end: Timestamp::from_millis(100),
            }),
            // A recurring rule has no fixed instant, and a plain entry no occurrence at all: both NULL.
            Some(TemporalRef::Recurring(Rrule("FREQ=WEEKLY".into()))),
            None,
        ];
        let entry_ids: Vec<EntryId> = refs.iter().map(|_| EntryId::generate()).collect();
        let mut events = vec![created(id, "topic/dated")];
        for (entry_id, reference) in entry_ids.iter().zip(refs.iter()) {
            events.push(appended(id, *entry_id, reference.clone()));
        }

        let (_store, graph) = materialized(events);
        let entries = graph.entries_local(id).unwrap();
        assert_eq!(entries.len(), refs.len());
        for (entry, reference) in entries.iter().zip(refs.iter()) {
            let expected = reference
                .as_ref()
                .and_then(|r| r.bounds(None, BEFORE_AFTER_EPSILON_MILLIS).sort);
            assert_eq!(entry.occurred_sort, expected);
        }
    }

    #[test]
    fn before_after_resolves_against_its_anchor() {
        let anchor = MemoryId::generate();
        let dependent = MemoryId::generate();
        let anchor_at = 1_000_000;
        let dep_entry = EntryId::generate();
        let (_store, graph) = materialized(vec![
            created(anchor, "event/wedding"),
            appended(
                anchor,
                EntryId::generate(),
                Some(TemporalRef::Instant(Timestamp::from_millis(anchor_at))),
            ),
            created(dependent, "event/reception"),
            appended(
                dependent,
                dep_entry,
                Some(TemporalRef::BeforeAfter {
                    dir: Direction::After,
                    anchor: MemoryName::new("event/wedding"),
                }),
            ),
        ]);
        let entries = graph.entries_local(dependent).unwrap();
        assert_eq!(
            entries[0].occurred_sort,
            Some(Timestamp::from_millis(
                anchor_at + BEFORE_AFTER_EPSILON_MILLIS
            ))
        );
    }

    #[test]
    fn before_after_resolves_a_soft_deleted_anchor() {
        // MemoryDeleted preserves contents, so an anchor deleted before the dependent's append still
        // resolves — the spec's load-bearing watch-list case.
        let anchor = MemoryId::generate();
        let dependent = MemoryId::generate();
        let anchor_at = 2_000_000;
        let (_store, graph) = materialized(vec![
            created(anchor, "event/move"),
            appended(
                anchor,
                EntryId::generate(),
                Some(TemporalRef::Instant(Timestamp::from_millis(anchor_at))),
            ),
            EventPayload::MemoryDeleted { id: anchor },
            created(dependent, "event/housewarming"),
            appended(
                dependent,
                EntryId::generate(),
                Some(TemporalRef::BeforeAfter {
                    dir: Direction::After,
                    anchor: MemoryName::new("event/move"),
                }),
            ),
        ]);
        let entries = graph.entries_local(dependent).unwrap();
        assert_eq!(
            entries[0].occurred_sort,
            Some(Timestamp::from_millis(
                anchor_at + BEFORE_AFTER_EPSILON_MILLIS
            ))
        );
    }

    #[test]
    fn before_after_with_an_unknown_anchor_is_untimed() {
        let dependent = MemoryId::generate();
        let (_store, graph) = materialized(vec![
            created(dependent, "event/orphan"),
            appended(
                dependent,
                EntryId::generate(),
                Some(TemporalRef::BeforeAfter {
                    dir: Direction::After,
                    anchor: MemoryName::new("event/never-created"),
                }),
            ),
        ]);
        let entries = graph.entries_local(dependent).unwrap();
        assert_eq!(entries[0].occurred_sort, None);
    }

    #[test]
    fn occurrence_columns_survive_replay() {
        let id = MemoryId::generate();
        let (store, graph) = materialized(vec![
            created(id, "topic/dated"),
            appended(
                id,
                EntryId::generate(),
                Some(TemporalRef::Day(CivilDate("2026-06-03".into()))),
            ),
        ]);
        // A second projection of the same log reproduces the denormalized columns exactly.
        let mut replayed = Graph::open_in_memory().unwrap();
        replayed.materialize_from(&store).unwrap();
        let original: Vec<_> = graph
            .entries_local(id)
            .unwrap()
            .iter()
            .map(|e| e.occurred_sort)
            .collect();
        let again: Vec<_> = replayed
            .entries_local(id)
            .unwrap()
            .iter()
            .map(|e| e.occurred_sort)
            .collect();
        assert_eq!(original, again);
        assert!(original[0].is_some());
    }

    #[test]
    fn occurrences_in_window_returns_in_window_pairs_soonest_first() {
        let a = MemoryId::generate();
        let b = MemoryId::generate();
        let c = MemoryId::generate();
        let (_store, graph) = materialized(vec![
            created(a, "event/a"),
            appended(
                a,
                EntryId::generate(),
                Some(TemporalRef::Instant(Timestamp::from_millis(300))),
            ),
            // An untimed entry on the same memory must not pull it in twice.
            appended(a, EntryId::generate(), None),
            created(b, "event/b"),
            appended(
                b,
                EntryId::generate(),
                Some(TemporalRef::Instant(Timestamp::from_millis(100))),
            ),
            created(c, "event/c"),
            appended(
                c,
                EntryId::generate(),
                Some(TemporalRef::Instant(Timestamp::from_millis(5_000))),
            ),
        ]);
        let hits = graph
            .occurrences_in_window(Timestamp::from_millis(0), Timestamp::from_millis(1_000))
            .unwrap();
        let names: Vec<_> = hits
            .iter()
            .map(|(memory, _)| memory.name.as_str().to_owned())
            .collect();
        // Ordered by occurrence (100 then 300); c at 5_000 is out of the window.
        assert_eq!(names, vec!["event/b", "event/a"]);
    }

    #[test]
    fn occurrences_in_window_excludes_soft_deleted() {
        let a = MemoryId::generate();
        let (_store, graph) = materialized(vec![
            created(a, "event/a"),
            appended(
                a,
                EntryId::generate(),
                Some(TemporalRef::Instant(Timestamp::from_millis(100))),
            ),
            EventPayload::MemoryDeleted { id: a },
        ]);
        assert!(
            graph
                .occurrences_in_window(Timestamp::from_millis(0), Timestamp::from_millis(1_000))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn recurring_memories_lists_only_true_recurrences() {
        let standup = MemoryId::generate();
        let concrete = MemoryId::generate();
        let dangling = MemoryId::generate();
        let (_store, graph) = materialized(vec![
            created(standup, "event/standup"),
            appended(
                standup,
                EntryId::generate(),
                Some(TemporalRef::Recurring(Rrule("FREQ=WEEKLY".into()))),
            ),
            created(concrete, "event/concrete"),
            appended(
                concrete,
                EntryId::generate(),
                Some(TemporalRef::Day(CivilDate("2026-06-03".into()))),
            ),
            // An unresolved BeforeAfter is also sort-null but is not a recurrence.
            created(dangling, "event/dangling"),
            appended(
                dangling,
                EntryId::generate(),
                Some(TemporalRef::BeforeAfter {
                    dir: Direction::After,
                    anchor: MemoryName::new("event/never-created"),
                }),
            ),
        ]);
        let names: Vec<_> = graph
            .recurring_memories()
            .unwrap()
            .iter()
            .map(|memory| memory.name.as_str().to_owned())
            .collect();
        assert_eq!(names, vec!["event/standup"]);
    }
}
