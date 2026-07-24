//! The class-spanning write redirect: a clean, platform-agnostic handle lands its class-level writes on
//! the class primary — the earliest-ULID stub, or the operator-designated one — while a
//! platform-qualified handle, a same-block create, and the operator anchor are each left untouched.

use super::{
    AppendOptions, Authority, VisibilityChoice, block, designated_primary_seed, graph_from,
};
use crate::{
    clock::ManualClock,
    event::{Cardinality, EventPayload, EventSource, LinkPosture, LinkSource, Teller, Visibility},
    graph::Graph,
    ids::{MemoryId, Namespace},
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::RelationName,
};

#[test]
fn class_handle_write_lands_on_the_primary_stub() {
    // Spec appendix scenario 15 (write half): the clean, unqualified handle is the merged class's
    // primary stub (its earliest ULID), so recording a platform-agnostic human-fact through
    // `memory.get("person/<name>")` lands the append on the primary. Writes are not traversed onto the
    // primary — the clean handle simply *is* it — and the fact then surfaces for the whole class.
    // The clean name is bound to the earlier of two ULIDs so it is deterministically the primary,
    // regardless of the random low bits `MemoryId::generate` mints within one millisecond.
    let mut ids = [MemoryId::generate(), MemoryId::generate()];
    ids.sort();
    let [primary, chat_stub] = ids;
    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![
                EventPayload::LinkTypeRegistered {
                    name: RelationName::SameAs,
                    inverse: RelationName::SameAs,
                    from_card: Cardinality::Many,
                    to_card: Cardinality::Many,
                    symmetric: true,
                    reflexive: false,
                    description: String::new(),
                },
                EventPayload::memory_created(primary, Namespace::Person.with_name("quinn")),
                EventPayload::memory_created(chat_stub, Namespace::Person.with_name("quinn@chat")),
                EventPayload::link_created(
                    primary,
                    chat_stub,
                    RelationName::SameAs,
                    LinkPosture {
                        source: LinkSource::Operator,
                        told_by: None,
                        told_in: None,
                        visibility: Visibility::Public,
                    },
                ),
            ],
        )
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();

    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);

    // The clean handle resolves to the primary stub, the earliest ULID of the class.
    let (resolved, former) = block.get("person/quinn").unwrap().unwrap();
    assert!(!former);
    assert_eq!(resolved, primary);

    // The append through the class handle lands without error, and its event is stamped with the
    // primary stub — no rewrite onto some other member.
    block
        .append(
            resolved,
            "prefers async standups",
            AppendOptions {
                visibility: Some(VisibilityChoice::Public),
                ..AppendOptions::default()
            },
        )
        .unwrap();

    // And it composes across the class: a live read from the other stub surfaces the same fact.
    let from_chat = block.entries(chat_stub).unwrap();
    assert!(
        from_chat
            .iter()
            .any(|entry| entry.text == "prefers async standups"),
        "the fact should surface for the whole class"
    );

    let landed_on: Vec<MemoryId> = block
        .into_effects()
        .events
        .into_iter()
        .filter_map(|event| match event {
            EventPayload::MemoryContentAppended { id, .. } => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(landed_on, vec![primary]);
}

#[test]
fn class_handle_write_redirects_to_the_designated_primary() {
    // The clean handle `person/dave` resolves to its own (non-primary) stub, but a class-level fact told
    // through it belongs on the class primary the operator designated (`person/marcus`) — so the append
    // is redirected there rather than attaching to the member the name happens to resolve to.
    let (seed, dave, marcus) = designated_primary_seed();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph_from(seed), clock, Teller::Agent, Authority::Platform);

    let (resolved, _) = block.get("person/dave").unwrap().unwrap();
    assert_eq!(
        resolved, dave,
        "the clean handle resolves to its exact stub"
    );

    block
        .append(
            resolved,
            "ships on Fridays",
            AppendOptions {
                visibility: Some(VisibilityChoice::Public),
                ..AppendOptions::default()
            },
        )
        .unwrap();

    let landed_on: Vec<MemoryId> = block
        .into_effects()
        .events
        .into_iter()
        .filter_map(|event| match event {
            EventPayload::MemoryContentAppended { id, .. } => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(
        landed_on,
        vec![marcus],
        "the write should redirect to the designated primary, not land on person/dave"
    );
}

#[test]
fn a_platform_qualified_handle_write_stays_on_its_exact_stub() {
    // `person/quinn@chat` names one specific platform binding; a fact deliberately scoped to it stays
    // on that stub even though its class primary (`person/quinn`) is another member — the redirect is for
    // the clean, class-spanning handle only.
    let (graph, quinn, quinn_chat) = super::graph_with_merged_pair();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);

    block
        .append(
            quinn_chat,
            "logs in from the Berlin office",
            AppendOptions {
                visibility: Some(VisibilityChoice::Public),
                ..AppendOptions::default()
            },
        )
        .unwrap();

    let landed_on: Vec<MemoryId> = block
        .into_effects()
        .events
        .into_iter()
        .filter_map(|event| match event {
            EventPayload::MemoryContentAppended { id, .. } => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(
        landed_on,
        vec![quinn_chat],
        "a platform-qualified handle keeps its write, never widening to the class primary {quinn:?}"
    );
}

#[test]
fn a_same_block_create_write_is_not_redirected() {
    // A memory created this block is not yet committed, so it has no class — the append to its fresh stub
    // must land on it, never widen to a primary the uncommitted create cannot have.
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(
        Graph::open_in_memory().unwrap(),
        clock,
        Teller::Agent,
        Authority::Platform,
    );
    let dana = block
        .create(Namespace::Person.with_name("dana"), None)
        .unwrap();
    block
        .append(
            dana,
            "keeps a bullet journal",
            AppendOptions {
                visibility: Some(VisibilityChoice::Public),
                ..AppendOptions::default()
            },
        )
        .unwrap();
    let landed_on: Vec<MemoryId> = block
        .into_effects()
        .events
        .into_iter()
        .filter_map(|event| match event {
            EventPayload::MemoryContentAppended { id, .. } => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(landed_on, vec![dana]);
}

#[test]
fn a_write_is_never_redirected_onto_the_operator_anchor() {
    // The operator anchor (`person/operator`) holds no content by design and is the earliest-ULID
    // primary of the operator's class, so a class-spanning write on the operator's real `person/<name>`
    // profile must stay on that profile — never redirect onto the anchor `guard_operator` forbids.
    let mut ids = [MemoryId::generate(), MemoryId::generate()];
    ids.sort();
    let [anchor, dana] = ids;
    let seed = vec![
        EventPayload::LinkTypeRegistered {
            name: RelationName::SameAs,
            inverse: RelationName::SameAs,
            from_card: Cardinality::Many,
            to_card: Cardinality::Many,
            symmetric: true,
            reflexive: false,
            description: String::new(),
        },
        EventPayload::memory_created(anchor, Namespace::Person.with_name("operator")),
        EventPayload::memory_created(dana, Namespace::Person.with_name("dana")),
        EventPayload::link_created(
            anchor,
            dana,
            RelationName::SameAs,
            LinkPosture {
                source: LinkSource::Operator,
                told_by: None,
                told_in: None,
                visibility: Visibility::Public,
            },
        ),
    ];
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph_from(seed), clock, Teller::Agent, Authority::Operator);

    // The anchor is the earliest ULID, so it is the class primary — the case that would misfire without
    // the carve-out.
    block
        .append(
            dana,
            "prefers evening syncs",
            AppendOptions {
                visibility: Some(VisibilityChoice::Public),
                ..AppendOptions::default()
            },
        )
        .expect("the operator write should land, not be forbidden by the anchor guard");

    let landed_on: Vec<MemoryId> = block
        .into_effects()
        .events
        .into_iter()
        .filter_map(|event| match event {
            EventPayload::MemoryContentAppended { id, .. } => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(
        landed_on,
        vec![dana],
        "the operator fact stays on person/dana, never on the anchor {anchor:?}"
    );
}

#[test]
fn supersede_and_set_volatility_redirect_to_the_designated_primary() {
    // supersede and set_volatility are class-level writes like append, so they too attribute to the
    // designated primary when told through the clean, non-primary handle.
    let (seed, dave, marcus) = designated_primary_seed();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph_from(seed), clock, Teller::Agent, Authority::Platform);

    let opts = || AppendOptions {
        visibility: Some(VisibilityChoice::Public),
        ..AppendOptions::default()
    };
    let old = block.append(dave, "on the mobile team", opts()).unwrap();
    let new = block.append(dave, "on the platform team", opts()).unwrap();
    block.supersede(dave, old, new).unwrap();
    block.set_volatility(dave, "high").unwrap();

    let effects = block.into_effects();
    let superseded_on = effects.events.iter().find_map(|event| match event {
        EventPayload::MemorySuperseded { id, .. } => Some(*id),
        _ => None,
    });
    let volatility_on = effects.events.iter().find_map(|event| match event {
        EventPayload::MemoryVolatilitySet { id, .. } => Some(*id),
        _ => None,
    });
    assert_eq!(superseded_on, Some(marcus));
    assert_eq!(volatility_on, Some(marcus));
}
