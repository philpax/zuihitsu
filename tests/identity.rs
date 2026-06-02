//! Identity-resolution tests: a platform participant and a room locator resolve to a stable id,
//! minting exactly once on first contact and resolving to the same id thereafter (spec §Identity).

#![cfg(feature = "sqlite")]

use zuihitsu::{
    ConversationLocator, Graph, ManualClock, MemoryStore, Seq, Store, Timestamp,
    resolve_or_mint_conversation, resolve_or_mint_participant,
};

#[test]
fn participant_resolves_and_mints_once() {
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut graph = Graph::open_in_memory().unwrap();

    // First contact mints a provisional stub bound to the platform key.
    let id = resolve_or_mint_participant(&mut store, &clock, &graph, "discord", "12345").unwrap();
    graph.materialize_from(&store).unwrap();
    assert_eq!(store.head().unwrap(), Seq(2)); // MemoryCreated + ParticipantIdentified
    assert_eq!(graph.participant_for("discord", "12345").unwrap(), Some(id));
    assert_eq!(
        graph.memory_by_id(id).unwrap().unwrap().name.as_str(),
        "person/12345@discord"
    );

    // Second contact resolves to the same stub and mints nothing.
    let again =
        resolve_or_mint_participant(&mut store, &clock, &graph, "discord", "12345").unwrap();
    assert_eq!(again, id);
    assert_eq!(store.head().unwrap(), Seq(2));

    // A different platform user, and the same user_id on another platform, are distinct stubs.
    let other =
        resolve_or_mint_participant(&mut store, &clock, &graph, "discord", "67890").unwrap();
    let elsewhere =
        resolve_or_mint_participant(&mut store, &clock, &graph, "slack", "12345").unwrap();
    assert_ne!(other, id);
    assert_ne!(elsewhere, id);
    assert_ne!(elsewhere, other);
    assert_eq!(store.head().unwrap(), Seq(6)); // two more mints, two events each
}

#[test]
fn conversation_resolves_and_mints_once() {
    let mut store = MemoryStore::new();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut graph = Graph::open_in_memory().unwrap();
    let leads = ConversationLocator::new("discord", "guild/42/chan/leads");

    // First contact opens the room and eagerly mints its context memory.
    let id = resolve_or_mint_conversation(&mut store, &clock, &graph, &leads).unwrap();
    graph.materialize_from(&store).unwrap();
    assert_eq!(store.head().unwrap(), Seq(2)); // MemoryCreated(context) + ConversationStarted
    assert_eq!(graph.conversation_for_locator(&leads).unwrap(), Some(id));
    // The locator resolves to a real, non-person context memory (defaults Public, no subject-guard).
    let context = graph.context_for_conversation(id).unwrap().unwrap();
    assert_eq!(
        graph.memory_by_id(context).unwrap().unwrap().name.as_str(),
        "context/discord:guild/42/chan/leads"
    );

    // The same locator resolves to the same room and opens nothing new.
    let again = resolve_or_mint_conversation(&mut store, &clock, &graph, &leads).unwrap();
    assert_eq!(again, id);
    assert_eq!(store.head().unwrap(), Seq(2));

    // A different room is a distinct conversation with its own context.
    let dms = ConversationLocator::new("discord", "dm/dave");
    let other = resolve_or_mint_conversation(&mut store, &clock, &graph, &dms).unwrap();
    assert_ne!(other, id);
    assert_eq!(store.head().unwrap(), Seq(4));
}
