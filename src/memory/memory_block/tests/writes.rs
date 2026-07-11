//! The write-path basics and teachable write errors.

use super::{AppendOptions, Authority, MemoryError, VisibilityChoice, block};
use crate::{
    clock::ManualClock,
    event::{Cardinality, EventPayload, EventSource, LinkSource, Teller, Visibility},
    graph::{Graph, GraphError},
    ids::{EntryId, MemoryId, Namespace},
    memory::memory_block::{LinkOptions, RelationSpec},
    store::{MemoryStore, Store},
    time::{Rrule, TemporalRef, Timestamp},
    vocabulary::RelationName,
};

#[test]
fn create_rejects_a_duplicate_name() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let plan = Namespace::Topic.with_name("plan");
    block.create(&plan, None).unwrap();
    // Caught against the block's own pending create (read-your-writes), before any commit.
    let error = block.create(&plan, None).unwrap_err();
    assert!(matches!(error, MemoryError::NameExists { .. }));
}

#[test]
fn link_rejects_an_unregistered_relation() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let a = block.create(Namespace::Topic.with_name("a"), None).unwrap();
    let b = block.create(Namespace::Topic.with_name("b"), None).unwrap();
    let error = block
        .link(a, b, RelationName::Other("bogus_relation".into()), None)
        .unwrap_err();
    assert!(matches!(error, MemoryError::UnknownRelation(_)));
}

#[test]
fn an_aside_about_another_person_defaults_private() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let speaker = MemoryId::generate();
    let mut block = block(
        graph,
        clock,
        Teller::Participant(speaker),
        Authority::Platform,
    );
    // The speaker (the teller) is not the subject of person/marcus, so the default is private.
    let marcus = block
        .create(Namespace::Person.with_name("marcus"), None)
        .unwrap();
    block
        .append(marcus, "is being managed out", AppendOptions::default())
        .unwrap();

    let visibility = block
        .into_effects()
        .events
        .into_iter()
        .find_map(|event| match event {
            EventPayload::MemoryContentAppended { visibility, .. } => Some(visibility),
            _ => None,
        })
        .unwrap();
    assert_eq!(visibility, Visibility::PrivateToTeller);
}

#[test]
fn agent_authored_writes_about_a_person_require_explicit_visibility() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);

    // An agent-authored entry about a person has no protective default, so it must be classified:
    // both a create-with-content and a bare append fail teachably without an explicit visibility.
    let erin = Namespace::Person.with_name("erin");
    assert!(matches!(
        block
            .create(&erin, Some("may be leaving the team"))
            .unwrap_err(),
        MemoryError::VisibilityRequired
    ));
    let erin = block.create(&erin, None).unwrap();
    assert!(matches!(
        block
            .append(erin, "may be leaving the team", AppendOptions::default())
            .unwrap_err(),
        MemoryError::VisibilityRequired
    ));

    // Once classified it succeeds; and a non-person memory has no subject to guard, so the agent's
    // write there keeps the public default with no classification required.
    block
        .append(
            erin,
            "may be leaving the team",
            AppendOptions {
                visibility: Some(VisibilityChoice::Private),
                ..AppendOptions::default()
            },
        )
        .unwrap();
    let roadmap = block
        .create(
            Namespace::Topic.with_name("roadmap"),
            Some("ship on Friday"),
        )
        .unwrap();
    block
        .append(roadmap, "migration first", AppendOptions::default())
        .unwrap();
}

#[test]
fn append_with_exclude_records_the_named_parties() {
    // The exclude opt records the entry as a confidence additionally withheld whenever a named party
    // is present — a Visibility::Exclude over the resolved ids. It also classifies the entry, so an
    // agent-authored write about a person (which otherwise has no safe default and would be a
    // VisibilityRequired error) is accepted: an exclude is itself an explicit classification.
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let dave = MemoryId::generate();
    let frank = MemoryId::generate();
    let erin = block
        .create(Namespace::Person.with_name("erin"), None)
        .unwrap();
    block
        .append(
            erin,
            "everyone but them is chipping in on the gift",
            AppendOptions {
                exclude: Some(vec![dave, frank]),
                ..AppendOptions::default()
            },
        )
        .unwrap();

    let visibility = block
        .into_effects()
        .events
        .into_iter()
        .find_map(|event| match event {
            EventPayload::MemoryContentAppended { visibility, .. } => Some(visibility),
            _ => None,
        })
        .unwrap();
    assert_eq!(visibility, Visibility::Exclude(vec![dave, frank]));
}

#[test]
fn create_with_content_honors_the_exclude_opt() {
    // A memory created with seed content takes the same overrides as an append, so a guarded fact's
    // first entry is classified at creation — an unclassified seed entry on a non-person memory would
    // otherwise take the Public default and leak beside its excluded siblings.
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let dave = MemoryId::generate();
    block
        .create_with_opts(
            Namespace::Topic.with_name("gift"),
            Some("collecting for a farewell gift"),
            Some(AppendOptions {
                exclude: Some(vec![dave]),
                ..AppendOptions::default()
            }),
        )
        .unwrap();

    let visibility = block
        .into_effects()
        .events
        .into_iter()
        .find_map(|event| match event {
            EventPayload::MemoryContentAppended { visibility, .. } => Some(visibility),
            _ => None,
        })
        .unwrap();
    assert_eq!(visibility, Visibility::Exclude(vec![dave]));
}

#[test]
fn exclude_beside_a_defaulted_open_seed_is_a_teachable_error() {
    // The one-plain-copy leak, caught at its point of failure: a memory created with unclassified
    // seed content (which lands Public on a non-person memory) may not take an exclude append in the
    // same block — the open seed beside the guard undoes it. The agent reissues with the seed
    // classified, or created bare.
    let mut block = block(
        Graph::open_in_memory().unwrap(),
        ManualClock::new(Timestamp::from_millis(1_000)),
        Teller::Agent,
        Authority::Platform,
    );
    let dave = MemoryId::generate();
    let plan = block
        .create(
            Namespace::Topic.with_name("celebration"),
            Some("planning the anniversary do"),
        )
        .unwrap();
    let error = block
        .append(
            plan,
            "it is a surprise — keep it from them",
            AppendOptions {
                exclude: Some(vec![dave]),
                ..AppendOptions::default()
            },
        )
        .unwrap_err();
    assert!(
        matches!(error, MemoryError::UnguardedSeedBesideExclude),
        "{error:?}"
    );
    assert!(
        error.to_string().contains("create the memory bare"),
        "the error should name the fix: {error}"
    );
}

#[test]
fn exclude_is_accepted_beside_a_deliberately_classified_seed() {
    // The guard is scoped to the *unforced* default: a seed explicitly classified public is the
    // agent's own call, a seed classified exclude is already guarded, and a bare create has no seed
    // at all — all three take a same-block exclude append.
    let mut block = block(
        Graph::open_in_memory().unwrap(),
        ManualClock::new(Timestamp::from_millis(1_000)),
        Teller::Agent,
        Authority::Platform,
    );
    let dave = MemoryId::generate();
    let exclude_opts = || AppendOptions {
        exclude: Some(vec![dave]),
        ..AppendOptions::default()
    };

    let deliberate = block
        .create_with_opts(
            Namespace::Topic.with_name("openly-public"),
            Some("a stated public fact"),
            Some(AppendOptions {
                visibility: Some(VisibilityChoice::Public),
                ..AppendOptions::default()
            }),
        )
        .unwrap();
    block
        .append(deliberate, "a guarded detail", exclude_opts())
        .unwrap();

    let guarded = block
        .create_with_opts(
            Namespace::Topic.with_name("guarded-seed"),
            Some("a guarded seed"),
            Some(exclude_opts()),
        )
        .unwrap();
    block
        .append(guarded, "another guarded detail", exclude_opts())
        .unwrap();

    let bare = block
        .create(Namespace::Topic.with_name("bare"), None)
        .unwrap();
    block
        .append(bare, "a guarded detail", exclude_opts())
        .unwrap();
}

#[test]
fn exclude_is_accepted_when_the_defaulted_seed_landed_private() {
    // A participant's aside about another person defaults PrivateToTeller — not open — so a
    // same-block exclude append is not a leak and passes.
    let speaker = MemoryId::generate();
    let dave = MemoryId::generate();
    let mut block = block(
        Graph::open_in_memory().unwrap(),
        ManualClock::new(Timestamp::from_millis(1_000)),
        Teller::Participant(speaker),
        Authority::Platform,
    );
    let marcus = block
        .create(
            Namespace::Person.with_name("marcus"),
            Some("mentioned in passing"),
        )
        .unwrap();
    block
        .append(
            marcus,
            "keep this from the named party",
            AppendOptions {
                exclude: Some(vec![dave]),
                ..AppendOptions::default()
            },
        )
        .unwrap();
}

#[test]
fn exclude_is_accepted_on_a_previously_committed_memory() {
    // The guard is same-block only: a memory whose seed committed in an earlier block is standing
    // state, not this block's slip, so an exclude append to it passes. (The Public-copy hazard there
    // is real but historical — rejecting the append would not remove the committed copy.)
    let mut store = MemoryStore::new();
    let plan = MemoryId::generate();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![
                EventPayload::memory_created(plan, Namespace::Topic.with_name("celebration")),
                EventPayload::MemoryContentAppended {
                    id: plan,
                    entry_id: EntryId::generate(),
                    asserted_at: Timestamp::from_millis(1_000),
                    occurred_at: None,
                    text: "planning the anniversary do".to_owned(),
                    told_by: Teller::Agent,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            ],
        )
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    let mut block = block(
        graph,
        ManualClock::new(Timestamp::from_millis(2_000)),
        Teller::Agent,
        Authority::Platform,
    );
    block
        .append(
            plan,
            "it is a surprise — keep it from them",
            AppendOptions {
                exclude: Some(vec![MemoryId::generate()]),
                ..AppendOptions::default()
            },
        )
        .unwrap();
}

#[test]
fn exclude_and_visibility_together_is_a_conflict() {
    // An exclude is already a private posture, so pairing it with an explicit visibility is
    // contradictory — a teachable error, never a silent precedence.
    let mut block = block(
        Graph::open_in_memory().unwrap(),
        ManualClock::new(Timestamp::from_millis(1_000)),
        Teller::Agent,
        Authority::Platform,
    );
    let plan = block
        .create(Namespace::Event.with_name("plan"), None)
        .unwrap();
    let error = block
        .append(
            plan,
            "logistics",
            AppendOptions {
                visibility: Some(VisibilityChoice::Private),
                exclude: Some(vec![MemoryId::generate()]),
                ..AppendOptions::default()
            },
        )
        .unwrap_err();
    assert!(
        matches!(error, MemoryError::VisibilityConflict),
        "{error:?}"
    );
}

#[test]
fn an_empty_exclude_is_a_teachable_error() {
    // An exclude naming no one is just a private confidence for its teller, so it is rejected pointing
    // the agent at visibility = "private" for that case.
    let mut block = block(
        Graph::open_in_memory().unwrap(),
        ManualClock::new(Timestamp::from_millis(1_000)),
        Teller::Agent,
        Authority::Platform,
    );
    let plan = block
        .create(Namespace::Event.with_name("plan"), None)
        .unwrap();
    let error = block
        .append(
            plan,
            "logistics",
            AppendOptions {
                exclude: Some(Vec::new()),
                ..AppendOptions::default()
            },
        )
        .unwrap_err();
    assert!(matches!(error, MemoryError::ExcludeEmpty), "{error:?}");
    assert!(
        error.to_string().contains("visibility = \"private\""),
        "the error should point at the private posture: {error}"
    );
}

#[test]
fn link_with_exclude_records_the_named_parties() {
    // The same exclude opt on links.create records the edge as a Visibility::Exclude over the named
    // ids — so a relationship everyone but one person may know is held as a link-level confidence.
    let mut block = block(
        Graph::open_in_memory().unwrap(),
        ManualClock::new(Timestamp::from_millis(1_000)),
        Teller::Agent,
        Authority::Platform,
    );
    block
        .register_relation(RelationSpec {
            name: "plans".to_owned(),
            inverse: "planned_by".to_owned(),
            from_card: "many".to_owned(),
            to_card: "many".to_owned(),
            symmetric: false,
            reflexive: false,
            description: String::new(),
        })
        .unwrap();
    let erin = block
        .create(Namespace::Person.with_name("erin"), None)
        .unwrap();
    let party = block
        .create(Namespace::Event.with_name("party"), None)
        .unwrap();
    let dave = MemoryId::generate();
    block
        .link(
            erin,
            party,
            RelationName::Other("plans".into()),
            Some(LinkOptions {
                visibility: None,
                exclude: Some(vec![dave]),
            }),
        )
        .unwrap();

    let visibility = block
        .into_effects()
        .events
        .into_iter()
        .find_map(|event| match event {
            EventPayload::LinkCreated { visibility, .. } => Some(visibility),
            _ => None,
        })
        .unwrap();
    assert_eq!(visibility, Visibility::Exclude(vec![dave]));
}

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
    let [primary, discord_stub] = ids;
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
                EventPayload::memory_created(
                    discord_stub,
                    Namespace::Person.with_name("quinn@discord"),
                ),
                EventPayload::link_created(
                    primary,
                    discord_stub,
                    RelationName::SameAs,
                    LinkSource::Operator,
                    None,
                    None,
                    Visibility::Public,
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
    let from_discord = block.entries(discord_stub).unwrap();
    assert!(
        from_discord
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

/// The seed events for a `same_as` class of two clean, platform-agnostic person handles with the
/// *later*-ULID stub pinned as the class primary by a `ClassPrimaryDesignated` — so the earliest-ULID
/// clean handle resolves to a *non-primary* member, the shape the class-spanning write redirect turns
/// on. Returns the seed events, the earliest-ULID stub (`person/dave`), and the designated primary
/// (`person/marcus`). The clean handles are bound to sorted ULIDs so the designation, not ULID order,
/// decides the primary regardless of the random low bits minted within one millisecond.
fn designated_primary_seed() -> (Vec<EventPayload>, MemoryId, MemoryId) {
    let mut ids = [MemoryId::generate(), MemoryId::generate()];
    ids.sort();
    let [earliest, later] = ids;
    let events = vec![
        EventPayload::LinkTypeRegistered {
            name: RelationName::SameAs,
            inverse: RelationName::SameAs,
            from_card: Cardinality::Many,
            to_card: Cardinality::Many,
            symmetric: true,
            reflexive: false,
            description: String::new(),
        },
        EventPayload::memory_created(earliest, Namespace::Person.with_name("dave")),
        EventPayload::memory_created(later, Namespace::Person.with_name("marcus")),
        EventPayload::link_created(
            earliest,
            later,
            RelationName::SameAs,
            LinkSource::Operator,
            None,
            None,
            Visibility::Public,
        ),
        EventPayload::class_primary_designated(later, true),
    ];
    (events, earliest, later)
}

/// Materialize a set of seed events into a fresh in-memory graph — the committed state a block resolves
/// its write targets against.
fn graph_from(events: Vec<EventPayload>) -> Graph {
    let mut store = MemoryStore::new();
    store
        .append(Timestamp::from_millis(1_000), EventSource::Agent, events)
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    graph
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
    // `person/quinn@discord` names one specific platform binding; a fact deliberately scoped to it stays
    // on that stub even though its class primary (`person/quinn`) is another member — the redirect is for
    // the clean, class-spanning handle only.
    let (graph, quinn, quinn_discord) = super::graph_with_merged_pair();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);

    block
        .append(
            quinn_discord,
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
        vec![quinn_discord],
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
            LinkSource::Operator,
            None,
            None,
            Visibility::Public,
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

#[test]
fn a_redirected_write_replays_deterministically() {
    // The redirect reads committed state and emits an event carrying the concrete primary id, so
    // refolding the log reproduces the same placement: the entry sits on the primary and reads back
    // across the whole class from either handle.
    let (seed, dave, marcus) = designated_primary_seed();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(
        graph_from(seed.clone()),
        clock,
        Teller::Agent,
        Authority::Platform,
    );
    block
        .append(
            dave,
            "ships on Fridays",
            AppendOptions {
                visibility: Some(VisibilityChoice::Public),
                ..AppendOptions::default()
            },
        )
        .unwrap();
    let mut replayed = seed;
    replayed.extend(block.into_effects().events);
    let graph = graph_from(replayed);

    let on_primary = graph.class_entries(marcus).unwrap();
    assert!(
        on_primary
            .iter()
            .any(|entry| entry.text == "ships on Fridays"),
        "the refolded entry sits on the primary"
    );
    // The clean, non-primary handle reads the same class, so the fact surfaces from it too.
    let from_dave = graph.class_entries(dave).unwrap();
    assert!(
        from_dave
            .iter()
            .any(|entry| entry.text == "ships on Fridays"),
        "the fact reads back across the whole class"
    );
}

#[test]
fn append_rejects_an_unsupported_recurrence_with_a_teachable_error() {
    // A free-phrased cadence the model emits in place of an rrule arms no wake-up, so the write is
    // rejected for the agent to reissue — surfaced as a teachable error, not swallowed.
    let mut block = block(
        Graph::open_in_memory().unwrap(),
        ManualClock::new(Timestamp::from_millis(1_000)),
        Teller::Agent,
        Authority::Platform,
    );
    let standup = block
        .create(Namespace::Event.with_name("standup"), None)
        .unwrap();
    let err = block
        .append(
            standup,
            "every Monday",
            AppendOptions {
                occurred_at: Some(TemporalRef::Recurring(Rrule("every Monday".into()))),
                ..AppendOptions::default()
            },
        )
        .unwrap_err();
    assert!(
        matches!(err, MemoryError::UnsupportedRecurrence(ref rule) if rule == "every Monday"),
        "{err:?}"
    );
    assert!(
        err.to_string().contains("FREQ"),
        "the error should point at a supported rule: {err}"
    );

    // A supported rule is accepted, and arms a wake-up the scheduler can derive.
    block
        .append(
            standup,
            "team standup",
            AppendOptions {
                occurred_at: Some(TemporalRef::Recurring(Rrule("FREQ=WEEKLY;BYDAY=MO".into()))),
                ..AppendOptions::default()
            },
        )
        .unwrap();
}

#[test]
fn revise_rolls_back_the_append_when_the_supersede_fails() {
    // revise is append-then-supersede; a failed supersede must not leave the append's buffered event
    // behind. Without the transaction, a caught error (a Lua `pcall`) would commit the orphaned new
    // entry beside the stale value it was meant to replace. The transaction rolls the buffer back to
    // before the append.
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let a = block.create(Namespace::Topic.with_name("a"), None).unwrap();
    block
        .append(a, "original", AppendOptions::default())
        .unwrap();
    // A foreign entry (from a different memory) is not a live entry of `a`, so the supersede fails.
    let b = block.create(Namespace::Topic.with_name("b"), None).unwrap();
    let foreign = block
        .append(b, "b content", AppendOptions::default())
        .unwrap();
    let error = block
        .revise(a, foreign, "replacement", AppendOptions::default())
        .unwrap_err();
    assert!(
        matches!(error, MemoryError::UnknownEntry(_)),
        "revise should fail on a foreign entry, got {error:?}"
    );
    // The revise's append was rolled back: only the original append remains on `a`.
    let effects = block.into_effects();
    let appended: Vec<&EventPayload> = effects
        .events
        .iter()
        .filter(|event| matches!(event, EventPayload::MemoryContentAppended { id, .. } if *id == a))
        .collect();
    assert_eq!(
        appended.len(),
        1,
        "the failed revise's append should have been rolled back, but found {appended:?}"
    );
}

#[test]
fn graph_error_carries_a_memory_context_prefix() {
    // The Graph variant is infrastructure — `route_error` intercepts it and surfaces a generic
    // "internal graph error" to the agent — so its Display follows the error-display convention: a
    // `memory:` layer prefix nesting the graph error's own `materialized graph (…)` prefix, so a
    // propagated error reads as nested context (`memory: materialized graph (malformed): …`).
    let error = MemoryError::Graph(GraphError::Malformed("a corrupt id".to_owned()));
    assert_eq!(
        error.to_string(),
        "memory: materialized graph (malformed): a corrupt id"
    );
}
