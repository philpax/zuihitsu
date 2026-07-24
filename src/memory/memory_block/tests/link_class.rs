//! Class-aware link readers and the class-wide redundancy guard: the agent-facing readers
//! (`outgoing`/`incoming`/`links`) resolve every far endpoint (and teller) to its canonical class
//! primary and collapse parallel edges reaching one identity, and a re-link against a different member
//! of an already-linked identity is recognised as redundant rather than minting a parallel edge.

use crate::{
    clock::ManualClock,
    event::{Cardinality, EventPayload, EventSource, LinkPosture, LinkSource, Teller, Visibility},
    graph::Graph,
    ids::{MemoryId, Namespace},
    memory::memory_block::{
        LinkDirection, LinkOptions,
        tests::{Authority, MemoryBlock, VisibilityChoice, block, block_without_conversation},
    },
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::RelationName,
};

/// Materialize a fresh in-memory graph from `events` — the committed state a block reads against.
fn graph_from(events: Vec<EventPayload>) -> Graph {
    let mut store = MemoryStore::new();
    store
        .append(Timestamp::from_millis(1_000), EventSource::Agent, events)
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    graph
}

fn same_as_relation() -> EventPayload {
    EventPayload::LinkTypeRegistered {
        name: RelationName::SameAs,
        inverse: RelationName::SameAs,
        from_card: Cardinality::Many,
        to_card: Cardinality::Many,
        symmetric: true,
        reflexive: false,
        description: String::new(),
    }
}

fn mentor_relation() -> EventPayload {
    EventPayload::LinkTypeRegistered {
        name: RelationName::new("mentor_of"),
        inverse: RelationName::new("mentored_by"),
        from_card: Cardinality::Many,
        to_card: Cardinality::Many,
        symmetric: false,
        reflexive: false,
        description: String::new(),
    }
}

fn knows_relation() -> EventPayload {
    EventPayload::LinkTypeRegistered {
        name: RelationName::new("knows"),
        inverse: RelationName::new("known_by"),
        from_card: Cardinality::Many,
        to_card: Cardinality::Many,
        symmetric: false,
        reflexive: false,
        description: String::new(),
    }
}

fn same_as(a: MemoryId, b: MemoryId) -> EventPayload {
    EventPayload::link_created(
        a,
        b,
        RelationName::SameAs,
        LinkPosture {
            source: LinkSource::Operator,
            told_by: None,
            told_in: None,
            visibility: Visibility::Public,
        },
    )
}

/// A public `mentor_of` edge attributed to `teller`.
fn mentors(a: MemoryId, b: MemoryId, teller: Option<Teller>) -> EventPayload {
    EventPayload::link_created(
        a,
        b,
        RelationName::new("mentor_of"),
        LinkPosture {
            source: LinkSource::Agent,
            told_by: teller,
            told_in: None,
            visibility: Visibility::Public,
        },
    )
}

/// A merged fixture: a queried identity (`rowan` + a platform stub, `rowan` the designated primary), a
/// far identity (`erin`, a lone person), a teller identity (`quinn` + a platform stub, `quinn` the
/// primary), and a third lone person (`sam`). The `mentor_of` edges hang off the *stubs*, so the readers
/// must canonicalize to surface them under the readable primaries. Returns the graph and the ids.
struct Merged {
    graph: Graph,
    rowan: MemoryId,
    rowan_stub: MemoryId,
    erin: MemoryId,
    sam: MemoryId,
}

fn merged() -> Merged {
    let rowan = MemoryId::generate();
    let rowan_stub = MemoryId::generate();
    let erin = MemoryId::generate();
    let quinn = MemoryId::generate();
    let quinn_stub = MemoryId::generate();
    let sam = MemoryId::generate();
    let graph = graph_from(vec![
        same_as_relation(),
        mentor_relation(),
        EventPayload::memory_created(rowan, Namespace::Person.with_name("rowan")),
        EventPayload::memory_created(rowan_stub, Namespace::Person.with_name("9001@testplat")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        EventPayload::memory_created(quinn, Namespace::Person.with_name("quinn")),
        EventPayload::memory_created(quinn_stub, Namespace::Person.with_name("9002@testplat")),
        EventPayload::memory_created(sam, Namespace::Person.with_name("sam")),
        same_as(rowan, rowan_stub),
        same_as(quinn, quinn_stub),
        EventPayload::class_primary_designated(rowan, true),
        EventPayload::class_primary_designated(quinn, true),
        // The relationship hangs off the stub and is attributed to the teller's stub. A redundant copy
        // between the primaries exercises the dedup.
        mentors(rowan_stub, erin, Some(Teller::Participant(quinn_stub))),
        mentors(rowan, erin, Some(Teller::Participant(quinn_stub))),
        // A within-class edge (both ends in the queried identity) — identity plumbing, never a
        // relationship the reader surfaces.
        mentors(rowan_stub, rowan, None),
        // A third person mentors the queried identity through its stub.
        mentors(sam, rowan_stub, None),
    ]);
    Merged {
        graph,
        rowan,
        rowan_stub,
        erin,
        sam,
    }
}

#[test]
fn readers_render_the_far_endpoint_and_teller_under_their_primaries() {
    let m = merged();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(m.graph, clock, Teller::Agent, Authority::Platform);

    // `outgoing` from the primary: one mentee, the far primary's canonical handle, the teller resolved
    // through its own class primary — not the raw stub snowflakes either edge was stored against.
    let out = block.outgoing(m.rowan, "mentor_of").unwrap();
    assert_eq!(out.len(), 1, "the parallel mentor edges collapse to one");
    assert_eq!(out[0].other, m.erin);
    assert_eq!(out[0].other_name.as_str(), "person/erin");
    assert_eq!(out[0].direction, LinkDirection::Outgoing);
    assert_eq!(
        out[0].told_by.as_deref(),
        Some("person/quinn"),
        "the stub teller reads under its canonical primary name",
    );

    // `incoming` from the primary: sam mentors the identity, recorded against the stub, read here.
    let incoming = block.incoming(m.rowan, "mentor_of").unwrap();
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0].other_name.as_str(), "person/sam");
    assert_eq!(incoming[0].direction, LinkDirection::Incoming);

    // `links` gathers both directions and never the within-class plumbing edge.
    let links = block.links(m.rowan).unwrap();
    assert_eq!(links.len(), 2, "the within-class edge is dropped");
    let names: Vec<&str> = links.iter().map(|l| l.other_name.as_str()).collect();
    assert!(names.contains(&"person/erin") && names.contains(&"person/sam"));
    assert!(
        !names.iter().any(|n| n.contains("@testplat")),
        "no raw stub snowflake leaks into a rendered link",
    );
}

#[test]
fn a_reader_on_the_stub_matches_the_primary() {
    // The class is one identity, so reading from any member yields the same collapsed set.
    let m = merged();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(m.graph, clock, Teller::Agent, Authority::Platform);
    let from_stub = block.links(m.rowan_stub).unwrap();
    assert_eq!(from_stub.len(), 2);
    assert!(
        from_stub
            .iter()
            .all(|l| !l.other_name.as_str().contains("@testplat")),
        "reading from the stub still canonicalizes the far endpoints",
    );
}

#[test]
fn a_third_memory_reads_the_queried_class_under_its_primary() {
    // From sam's side, the mentored identity was recorded against the stub; the reader renders it under
    // the queried class's canonical primary, not the snowflake sam's edge actually points at.
    let m = merged();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(m.graph, clock, Teller::Agent, Authority::Platform);
    let out = block.outgoing(m.sam, "mentor_of").unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].other, m.rowan,
        "resolved to the queried class's primary"
    );
    assert_eq!(out[0].other_name.as_str(), "person/rowan");
}

#[test]
fn a_redundant_class_equivalent_relink_is_dropped_but_a_differing_one_records() {
    // A relationship recorded against a platform stub, then re-asserted against the identity's canonical
    // primary. With the identical posture the re-link is recognised as redundant (the class already holds
    // the edge) and mints nothing; with a differing posture it records, so the "make this public" path
    // still works — the fold writes the exact endpoints named, leaving the stub's edge intact.
    let alpha = MemoryId::generate();
    let alpha_stub = MemoryId::generate();
    let beta = MemoryId::generate();
    // The posture a `block_without_conversation` under `Authority::Platform`/`Teller::Agent` would write
    // for a public link: agent source, agent teller, no room. Storing it against the stub sets up the
    // exact collision a re-link against the primary must recognise.
    let stub_posture = LinkPosture {
        source: LinkSource::Agent,
        told_by: Some(Teller::Agent),
        told_in: None,
        visibility: Visibility::Public,
    };
    let base = vec![
        same_as_relation(),
        knows_relation(),
        EventPayload::memory_created(alpha, Namespace::Person.with_name("alpha")),
        EventPayload::memory_created(alpha_stub, Namespace::Person.with_name("9003@testplat")),
        EventPayload::memory_created(beta, Namespace::Person.with_name("beta")),
        same_as(alpha, alpha_stub),
        EventPayload::class_primary_designated(alpha, true),
        EventPayload::link_created(alpha_stub, beta, RelationName::new("knows"), stub_posture),
    ];

    let forced = |choice| {
        Some(LinkOptions {
            visibility: Some(choice),
            exclude: None,
        })
    };
    let created_links = |block: MemoryBlock| {
        block
            .into_effects()
            .events
            .into_iter()
            .filter(|event| matches!(event, EventPayload::LinkCreated { .. }))
            .collect::<Vec<_>>()
    };

    // Identical-posture re-link against the primary: redundant, nothing recorded.
    let mut redundant = block_without_conversation(
        graph_from(base.clone()),
        ManualClock::new(Timestamp::from_millis(2_000)),
        Teller::Agent,
        Authority::Platform,
    );
    redundant
        .link(
            alpha,
            beta,
            RelationName::new("knows"),
            forced(VisibilityChoice::Public),
        )
        .unwrap();
    assert!(
        created_links(redundant).is_empty(),
        "a same-posture re-link against another member of the identity is redundant",
    );

    // Differing-posture re-link: records against the exact endpoints named.
    let mut differing = block_without_conversation(
        graph_from(base),
        ManualClock::new(Timestamp::from_millis(3_000)),
        Teller::Agent,
        Authority::Platform,
    );
    differing
        .link(
            alpha,
            beta,
            RelationName::new("knows"),
            forced(VisibilityChoice::Private),
        )
        .unwrap();
    assert_eq!(
        created_links(differing).len(),
        1,
        "a differing-posture re-link still records",
    );
}
