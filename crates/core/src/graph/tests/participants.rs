//! The participant-mint plan always produces a qualified name `person/<id>@<platform>`, so two
//! participants who share an id across platforms stay distinct from first contact. The mint is
//! name-only — the caller (`resolve_or_mint_participant`) checks whether the qualified name
//! already exists as a memory (an agent-authored hearsay stub) and binds the platform identity
//! to it, or creates a fresh memory.

use crate::{
    event::EventPayload,
    graph::tests::materialized,
    ids::{MemoryId, Namespace, TEST_PLATFORM, TEST_PLATFORM_ALT},
};

#[test]
fn fresh_handle_mints_the_qualified_name() {
    let (_store, graph) = materialized(vec![]);
    let mint = graph.participant_mint(TEST_PLATFORM, "dave").unwrap();
    assert_eq!(mint.name, Namespace::Person.with_name("dave@chat").into());
}

#[test]
fn handle_bound_to_another_platform_uses_its_own_qualified_name() {
    // `person/dave@chat` is bound to chat's dave — a real platform identity.
    let dave = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(dave, Namespace::Person.with_name("dave@chat")),
        EventPayload::participant_identified(dave, TEST_PLATFORM, "dave"),
    ]);
    // A dave arriving on the other platform gets a different qualified name, so no collision at all.
    let mint = graph.participant_mint(TEST_PLATFORM_ALT, "dave").unwrap();
    assert_eq!(mint.name, Namespace::Person.with_name("dave@forum").into());
    assert_eq!(
        graph.participant_name(TEST_PLATFORM_ALT, "dave").unwrap(),
        Namespace::Person.with_name("dave@forum").into()
    );
}

#[test]
fn existing_qualified_stub_returns_the_qualified_name() {
    // `person/eve@chat` exists as an agent-authored stub — created from conversation, never
    // bound to any platform identity.
    let eve = MemoryId::generate();
    let (_store, graph) = materialized(vec![EventPayload::memory_created(
        eve,
        Namespace::Person.with_name("eve@chat"),
    )]);
    // Eve then arrives on chat: the mint just returns the qualified name. The caller
    // (`resolve_or_mint_participant`) checks whether it already exists as a memory and
    // binds the platform identity to the existing stub.
    let mint = graph.participant_mint(TEST_PLATFORM, "eve").unwrap();
    assert_eq!(mint.name, Namespace::Person.with_name("eve@chat").into());
}
