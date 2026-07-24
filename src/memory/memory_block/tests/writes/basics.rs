//! Create, rename, and link basics: the duplicate-name and unregistered-relation guards, the
//! person-seed visibility gates keyed on teller and authority, and the platform-namespace rename
//! guards.

use super::{AppendOptions, Authority, MemoryError, VisibilityChoice, block};
use crate::{
    clock::ManualClock,
    event::Teller,
    graph::Graph,
    ids::{MemoryId, Namespace},
    time::Timestamp,
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
fn an_inline_seed_about_a_person_requires_explicit_visibility_from_any_teller() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let speaker = MemoryId::generate();
    let mut block = block(
        graph,
        clock,
        Teller::Participant(speaker),
        Authority::Platform,
    );

    // A participant-told inline seed about a third party is refused rather than silently landing at
    // the PrivateToTeller default — the fact would vanish for every other audience.
    let rowan = Namespace::Person.with_name("rowan");
    assert!(matches!(
        block.create(&rowan, Some("backend lead")).unwrap_err(),
        MemoryError::VisibilityRequiredOnCreate
    ));

    // An explicit classification is honored, and a non-person memory has no subject to guard, so its
    // unclassified seed keeps the write-time default.
    block
        .create_with_opts(
            &rowan,
            Some("backend lead"),
            Some(AppendOptions {
                visibility: Some(VisibilityChoice::Attributed),
                ..AppendOptions::default()
            }),
        )
        .unwrap();
    block
        .create(
            Namespace::Topic.with_name("roadmap"),
            Some("ship on Friday"),
        )
        .unwrap();
}

#[test]
fn an_operator_seed_about_a_person_takes_the_default_unclassified() {
    // The operator is explicitly asserting from the console and may take the default, matching the
    // link gate's authority scoping — the create gate keys on platform authority, not the teller.
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let speaker = MemoryId::generate();
    let mut block = block(
        graph,
        clock,
        Teller::Participant(speaker),
        Authority::Operator,
    );
    block
        .create(Namespace::Person.with_name("rowan"), Some("backend lead"))
        .unwrap();
}

#[test]
fn agent_renames_stay_out_of_the_platform_namespace() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let stub = block
        .create(Namespace::Person.with_name("rowan@chat"), None)
        .unwrap();
    let profile = block
        .create(Namespace::Person.with_name("rowan"), None)
        .unwrap();

    // Moving a stub's name is refused: it mirrors the platform's view and follows the platform.
    assert!(matches!(
        block.rename(stub, "person/wren").unwrap_err(),
        MemoryError::RenameOfPlatformHandle { .. }
    ));
    // Claiming the qualified shape for another memory is refused: first contact binds a platform
    // identity by name, so the rename would squat a future participant's binding.
    assert!(matches!(
        block.rename(profile, "person/wren@chat").unwrap_err(),
        MemoryError::RenameOntoPlatformHandle { .. }
    ));
    // The bare profile renames freely — the agent's own namespace.
    block.rename(profile, "person/wren").unwrap();
}

#[test]
fn operator_renames_may_touch_platform_handles() {
    // The operator asserts from the console with connector-level authority, so the platform-namespace
    // rename guards do not apply.
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Operator);
    let stub = block
        .create(Namespace::Person.with_name("rowan@chat"), None)
        .unwrap();
    block.rename(stub, "person/wren@chat").unwrap();
}
