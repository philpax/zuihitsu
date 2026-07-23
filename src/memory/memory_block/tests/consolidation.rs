//! The consolidation write methods: tier-1 synthesis (a fresh replacement inheriting the sources'
//! posture) and tier-2 dedup (retiring sources into an existing, more-public entry).

use super::{Authority, MemoryError, VisibilityChoice, block, told};
use crate::{
    clock::ManualClock,
    event::{EventPayload, Teller, Visibility},
    graph::Graph,
    ids::{MemoryId, Namespace},
    time::Timestamp,
};

/// The `told_by`, `visibility`, and `text` a `MemoryContentAppended` for `entry` carries.
fn appended(
    events: &[EventPayload],
    entry: crate::ids::EntryId,
) -> Option<(Teller, Visibility, String)> {
    events.iter().find_map(|event| match event {
        EventPayload::MemoryContentAppended {
            entry_id,
            told_by,
            visibility,
            text,
            ..
        } if *entry_id == entry => Some((told_by.clone(), visibility.clone(), text.clone())),
        _ => None,
    })
}

#[test]
fn consolidate_inherits_a_uniform_teller_and_visibility() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let alice = MemoryId::generate();
    let mut block = block(graph, clock, Teller::Agent, Authority::Agent);
    let topic = block
        .create(Namespace::Topic.with_name("aside"), None)
        .unwrap();
    let first = block
        .append(
            topic,
            "first",
            told(Teller::Participant(alice), VisibilityChoice::Private),
        )
        .unwrap();
    let second = block
        .append(
            topic,
            "second",
            told(Teller::Participant(alice), VisibilityChoice::Private),
        )
        .unwrap();

    let replacement = block
        .consolidate(topic, &[first, second], "merged".to_owned(), None)
        .unwrap();

    let events = block.into_effects().events;
    // The synthesized replacement carries the sources' exact teller and visibility.
    assert_eq!(
        appended(&events, replacement),
        Some((
            Teller::Participant(alice),
            Visibility::PrivateToTeller,
            "merged".to_owned()
        ))
    );
    // And an EntriesConsolidated tombstones both sources against the replacement.
    let consolidated = events.iter().any(|event| {
        matches!(
            event,
            EventPayload::EntriesConsolidated { sources, replacement: target, .. }
                if *target == replacement && sources.contains(&first) && sources.contains(&second)
        )
    });
    assert!(
        consolidated,
        "the sources are tombstoned by an EntriesConsolidated"
    );
}

#[test]
fn consolidate_preserves_an_exact_exclude_set() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let alice = MemoryId::generate();
    let excluded = MemoryId::generate();
    let mut block = block(graph, clock, Teller::Agent, Authority::Agent);
    let topic = block
        .create(Namespace::Topic.with_name("aside"), None)
        .unwrap();
    // Two entries told by the same teller, excluded from the same party.
    let mut exclude_opts_a = told(Teller::Participant(alice), VisibilityChoice::Private);
    exclude_opts_a.exclude = Some([excluded].into_iter().collect());
    let first = block.append(topic, "first", exclude_opts_a).unwrap();
    let mut exclude_opts_b = told(Teller::Participant(alice), VisibilityChoice::Private);
    exclude_opts_b.exclude = Some([excluded].into_iter().collect());
    let second = block.append(topic, "second", exclude_opts_b).unwrap();

    let replacement = block
        .consolidate(topic, &[first, second], "merged".to_owned(), None)
        .unwrap();

    let events = block.into_effects().events;
    let (told_by, visibility, _) = appended(&events, replacement).unwrap();
    assert_eq!(told_by, Teller::Participant(alice));
    assert_eq!(
        visibility,
        Visibility::Exclude([excluded].into_iter().collect())
    );
}

#[test]
fn consolidate_collapses_a_cross_teller_public_merge_to_agent() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let alice = MemoryId::generate();
    let bob = MemoryId::generate();
    let mut block = block(graph, clock, Teller::Agent, Authority::Agent);
    let topic = block
        .create(Namespace::Topic.with_name("aside"), None)
        .unwrap();
    let first = block
        .append(
            topic,
            "first",
            told(Teller::Participant(alice), VisibilityChoice::Public),
        )
        .unwrap();
    let second = block
        .append(
            topic,
            "second",
            told(Teller::Participant(bob), VisibilityChoice::Public),
        )
        .unwrap();

    let replacement = block
        .consolidate(topic, &[first, second], "merged".to_owned(), None)
        .unwrap();

    let events = block.into_effects().events;
    // A cross-teller public merge collapses attribution to the agent, keeping the public level.
    assert_eq!(
        appended(&events, replacement),
        Some((Teller::Agent, Visibility::Public, "merged".to_owned()))
    );
}

#[test]
fn consolidate_rejects_mixed_visibility_sources() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let alice = MemoryId::generate();
    let mut block = block(graph, clock, Teller::Agent, Authority::Agent);
    let topic = block
        .create(Namespace::Topic.with_name("aside"), None)
        .unwrap();
    let public = block
        .append(
            topic,
            "public",
            told(Teller::Participant(alice), VisibilityChoice::Public),
        )
        .unwrap();
    let private = block
        .append(
            topic,
            "private",
            told(Teller::Participant(alice), VisibilityChoice::Private),
        )
        .unwrap();

    assert!(matches!(
        block
            .consolidate(topic, &[public, private], "merged".to_owned(), None)
            .unwrap_err(),
        MemoryError::ConsolidationInvariant(_)
    ));
}

#[test]
fn consolidate_into_retires_sources_into_an_existing_replacement() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let alice = MemoryId::generate();
    let bob = MemoryId::generate();
    let mut block = block(graph, clock, Teller::Agent, Authority::Agent);
    let topic = block
        .create(Namespace::Topic.with_name("aside"), None)
        .unwrap();
    let private = block
        .append(
            topic,
            "the same fact, privately",
            told(Teller::Participant(alice), VisibilityChoice::Private),
        )
        .unwrap();
    let public = block
        .append(
            topic,
            "the same fact, publicly",
            told(Teller::Participant(bob), VisibilityChoice::Public),
        )
        .unwrap();

    block
        .consolidate_into(topic, &[private], public, None)
        .unwrap();

    let events = block.into_effects().events;
    // The private source is retired into the existing public entry — no new content is appended.
    let appends = events
        .iter()
        .filter(|event| matches!(event, EventPayload::MemoryContentAppended { .. }))
        .count();
    assert_eq!(
        appends, 2,
        "no replacement entry is appended in a tier-2 dedup"
    );
    let retired = events.iter().any(|event| {
        matches!(
            event,
            EventPayload::EntriesConsolidated { sources, replacement, .. }
                if *replacement == public && sources == &[private]
        )
    });
    assert!(
        retired,
        "the private source is folded into the public entry"
    );
}
