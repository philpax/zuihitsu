//! Visibility defaults for writes about a person: an aside about a third party defaults private, while
//! an agent-authored write about a person has no protective default and must be classified explicitly.

use super::{AppendOptions, Authority, MemoryError, VisibilityChoice, block};
use crate::{
    clock::ManualClock,
    event::{EventPayload, Teller, Visibility},
    graph::Graph,
    ids::{MemoryId, Namespace},
    time::Timestamp,
};

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
    // The create path fails with the create-specific error, whose message teaches the seed shape.
    let erin = Namespace::Person.with_name("erin");
    assert!(matches!(
        block
            .create(&erin, Some("may be leaving the team"))
            .unwrap_err(),
        MemoryError::VisibilityRequiredOnCreate
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
