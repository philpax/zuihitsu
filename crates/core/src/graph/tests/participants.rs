//! The participant-mint plan keys the clean-vs-qualified name and the merge proposal on *identity*,
//! not on the name alone (spec §Identity → cross-platform-explicit): a free handle mints the clean
//! name; a handle already bound to a different platform identity mints the qualified name and stays
//! distinct; a handle owned by a platform-unbound hearsay stub mints the qualified name and proposes a
//! merge to reunite the two.

use super::materialized;
use crate::{
    event::EventPayload,
    ids::{MemoryId, Namespace},
};

#[test]
fn free_handle_mints_the_clean_name_with_no_proposal() {
    let (_store, graph) = materialized(vec![]);
    let mint = graph.participant_mint("discord", "dave").unwrap();
    assert_eq!(mint.name, Namespace::Person.with_name("dave").into());
    assert_eq!(mint.propose_same_as_with, None);
}

#[test]
fn handle_bound_to_another_platform_qualifies_and_stays_distinct() {
    // `person/dave` is bound to discord's dave — a real platform identity.
    let dave = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(dave, Namespace::Person.with_name("dave")),
        EventPayload::participant_identified(dave, "discord", "dave"),
    ]);
    // A dave arriving on slack collides on the handle, but the clean name is a *different* bound
    // identity, so the two stay distinct: qualified name, no proposal.
    let mint = graph.participant_mint("slack", "dave").unwrap();
    assert_eq!(mint.name, Namespace::Person.with_name("dave@slack").into());
    assert_eq!(mint.propose_same_as_with, None);
    // The name-only view agrees on the qualified name.
    assert_eq!(
        graph.participant_name("slack", "dave").unwrap(),
        Namespace::Person.with_name("dave@slack").into()
    );
}

#[test]
fn handle_owned_by_an_unbound_stub_qualifies_and_proposes_a_merge() {
    // `person/eve` exists as an agent-authored hearsay stub — created from conversation, never bound
    // to any platform.
    let eve = MemoryId::generate();
    let (_store, graph) = materialized(vec![EventPayload::memory_created(
        eve,
        Namespace::Person.with_name("eve"),
    )]);
    // Eve then arrives on discord: the qualified stub is minted, and a merge with the hearsay stub is
    // proposed for adjudication — a handle match surfaces a candidate reunion without asserting identity.
    let mint = graph.participant_mint("discord", "eve").unwrap();
    assert_eq!(mint.name, Namespace::Person.with_name("eve@discord").into());
    assert_eq!(mint.propose_same_as_with, Some(eve));
}
