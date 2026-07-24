//! The exclude opt across append, create-with-content, and link: it records a `Visibility::Exclude`
//! over the named parties, doubles as an explicit classification, and teaches at its boundaries — the
//! open-seed leak, the conflicting posture, and the empty list.

use std::collections::BTreeSet;

use super::{AppendOptions, Authority, MemoryError, VisibilityChoice, block};
use crate::{
    clock::ManualClock,
    event::{EventPayload, EventSource, Teller, Visibility},
    graph::Graph,
    ids::{EntryId, MemoryId, Namespace},
    memory::memory_block::{LinkOptions, RelationSpec},
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::RelationName,
};

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
                exclude: Some(BTreeSet::from([dave, frank])),
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
    assert_eq!(
        visibility,
        Visibility::Exclude(BTreeSet::from([dave, frank]))
    );
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
                exclude: Some(BTreeSet::from([dave])),
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
    assert_eq!(visibility, Visibility::Exclude(BTreeSet::from([dave])));
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
                exclude: Some(BTreeSet::from([dave])),
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
        exclude: Some(BTreeSet::from([dave])),
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
    // A seed that landed private — here classified explicitly, since an unclassified person seed is
    // refused by the create gate — is not open, so a same-block exclude append is not a leak and
    // passes.
    let speaker = MemoryId::generate();
    let dave = MemoryId::generate();
    let mut block = block(
        Graph::open_in_memory().unwrap(),
        ManualClock::new(Timestamp::from_millis(1_000)),
        Teller::Participant(speaker),
        Authority::Platform,
    );
    let marcus = block
        .create_with_opts(
            Namespace::Person.with_name("marcus"),
            Some("mentioned in passing"),
            Some(AppendOptions {
                visibility: Some(VisibilityChoice::Private),
                ..AppendOptions::default()
            }),
        )
        .unwrap();
    block
        .append(
            marcus,
            "keep this from the named party",
            AppendOptions {
                exclude: Some(BTreeSet::from([dave])),
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
                exclude: Some(BTreeSet::from([MemoryId::generate()])),
                ..AppendOptions::default()
            },
        )
        .unwrap();
}

#[test]
fn a_redundant_private_beside_exclude_folds_into_the_exclude() {
    // An exclude is already a private posture, so a visibility = "private" set beside it is redundant
    // but consistent: the write records as the exclude it means, costing no recovery round-trip. The
    // pairing is the model's reflex for anything sensitive, so it is accepted, not corrected.
    let mut block = block(
        Graph::open_in_memory().unwrap(),
        ManualClock::new(Timestamp::from_millis(1_000)),
        Teller::Agent,
        Authority::Platform,
    );
    let plan = block
        .create(Namespace::Event.with_name("plan"), None)
        .unwrap();
    let dave = MemoryId::generate();
    block
        .append(
            plan,
            "logistics",
            AppendOptions {
                visibility: Some(VisibilityChoice::Private),
                exclude: Some(BTreeSet::from([dave])),
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
    assert_eq!(visibility, Visibility::Exclude(BTreeSet::from([dave])));
}

#[test]
fn a_public_or_attributed_beside_exclude_is_a_conflict() {
    // Unlike a redundant private, a public or attributed beside an exclude contradicts the private
    // posture the exclude fixes — a teachable error, never a silent precedence.
    for contradictory in [VisibilityChoice::Public, VisibilityChoice::Attributed] {
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
                    visibility: Some(contradictory),
                    exclude: Some(BTreeSet::from([MemoryId::generate()])),
                    ..AppendOptions::default()
                },
            )
            .unwrap_err();
        assert!(
            matches!(error, MemoryError::VisibilityConflict),
            "{contradictory:?}: {error:?}"
        );
    }
}

#[test]
fn an_empty_exclude_beside_private_is_still_a_teachable_error() {
    // The empty-exclude teaching survives the redundant-private tolerance: an exclude naming no one
    // signals a malformed list whatever visibility rides beside it.
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
                exclude: Some(BTreeSet::new()),
                ..AppendOptions::default()
            },
        )
        .unwrap_err();
    assert!(matches!(error, MemoryError::ExcludeEmpty), "{error:?}");
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
                exclude: Some(BTreeSet::new()),
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
                exclude: Some(BTreeSet::from([dave])),
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
    assert_eq!(visibility, Visibility::Exclude(BTreeSet::from([dave])));
}
