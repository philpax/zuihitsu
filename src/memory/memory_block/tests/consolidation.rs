//! The consolidation write methods: tier-1 synthesis (a fresh replacement inheriting the sources'
//! posture) and tier-2 dedup (retiring sources into an existing, more-public entry).

use super::{Authority, MemoryError, VisibilityChoice, block, told};
use crate::{
    clock::ManualClock,
    event::{EventPayload, Teller, Visibility},
    graph::Graph,
    ids::{EntryId, MemoryId, Namespace},
    time::Timestamp,
};

/// The `told_by`, `visibility`, and `text` a `MemoryContentAppended` for `entry` carries.
fn appended(events: &[EventPayload], entry: EntryId) -> Option<(Teller, Visibility, String)> {
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

/// One `EntryAttested` on `entry`, reduced to the fields these tests assert over.
struct Attested {
    teller: Teller,
    posture: Visibility,
    phrasing: Option<String>,
    source_entry: Option<EntryId>,
    asserted_at: Timestamp,
}

/// Every `EntryAttested` buffered against `entry`, in buffer order.
fn attestations(events: &[EventPayload], entry: EntryId) -> Vec<Attested> {
    events
        .iter()
        .filter_map(|event| match event {
            EventPayload::EntryAttested {
                entry: attested,
                teller,
                posture,
                phrasing,
                source_entry,
                asserted_at,
                ..
            } if *attested == entry => Some(Attested {
                teller: teller.clone(),
                posture: posture.clone(),
                phrasing: phrasing.clone(),
                source_entry: *source_entry,
                asserted_at: *asserted_at,
            }),
            _ => None,
        })
        .collect()
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

#[test]
fn consolidate_attests_each_cross_teller_source_on_an_agent_replacement() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let alice = MemoryId::generate();
    let bob = MemoryId::generate();
    let mut block = block(graph, clock, Teller::Agent, Authority::Agent);
    let topic = block
        .create(Namespace::Topic.with_name("aside"), None)
        .unwrap();
    // Two relayed (attributed) accounts by different tellers — both all-audience, so they merge across
    // tellers into one agent-synthesized replacement.
    let first = block
        .append(
            topic,
            "alice's account",
            told(Teller::Participant(alice), VisibilityChoice::Attributed),
        )
        .unwrap();
    let second = block
        .append(
            topic,
            "bob's account",
            told(Teller::Participant(bob), VisibilityChoice::Attributed),
        )
        .unwrap();

    let replacement = block
        .consolidate(topic, &[first, second], "merged".to_owned(), None)
        .unwrap();
    let events = block.into_effects().events;

    // The synthesized text is nobody's verbatim account, so its founding attribution is the agent's,
    // at the sources' attributed level.
    assert_eq!(
        appended(&events, replacement),
        Some((Teller::Agent, Visibility::Attributed, "merged".to_owned()))
    );
    // Each source teller survives as an attestation on the replacement, carrying its source entry id,
    // asserted_at, and the group's posture — and never a self-phrasing (their text lives in the
    // tombstoned sources' history).
    let atts = attestations(&events, replacement);
    assert_eq!(atts.len(), 2, "one attestation per distinct source teller");
    for (teller, source) in [
        (Teller::Participant(alice), first),
        (Teller::Participant(bob), second),
    ] {
        let att = atts
            .iter()
            .find(|att| att.teller == teller)
            .unwrap_or_else(|| panic!("an attestation for {teller:?}"));
        assert_eq!(att.posture, Visibility::Attributed);
        assert_eq!(att.phrasing, None);
        assert_eq!(att.source_entry, Some(source));
        assert_eq!(att.asserted_at, Timestamp::from_millis(1_000));
    }
}

#[test]
fn consolidate_leaves_no_self_attestation_on_a_uniform_teller_merge() {
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
            told(Teller::Participant(alice), VisibilityChoice::Attributed),
        )
        .unwrap();
    let second = block
        .append(
            topic,
            "second",
            told(Teller::Participant(alice), VisibilityChoice::Attributed),
        )
        .unwrap();

    let replacement = block
        .consolidate(topic, &[first, second], "merged".to_owned(), None)
        .unwrap();
    let events = block.into_effects().events;

    // A single-teller merge keeps that teller as the replacement's founding attribution, so no separate
    // attestation is emitted — it would only duplicate the founding one.
    assert_eq!(
        appended(&events, replacement),
        Some((
            Teller::Participant(alice),
            Visibility::Attributed,
            "merged".to_owned()
        ))
    );
    assert!(
        attestations(&events, replacement).is_empty(),
        "the founding teller is not re-attested"
    );
}

#[test]
fn consolidate_into_leaves_a_hidden_attestation_with_phrasing() {
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

    // The retired private source leaves a hidden (private-posture) attestation on the public entry,
    // preserving its teller and exact wording, before the tombstoning consolidation.
    let atts = attestations(&events, public);
    assert_eq!(atts.len(), 1);
    assert_eq!(atts[0].teller, Teller::Participant(alice));
    assert_eq!(atts[0].posture, Visibility::PrivateToTeller);
    assert_eq!(
        atts[0].phrasing.as_deref(),
        Some("the same fact, privately")
    );
    assert_eq!(atts[0].source_entry, Some(private));
    // The attestation is buffered before the EntriesConsolidated that tombstones the source.
    let attest_pos = events
        .iter()
        .position(|event| matches!(event, EventPayload::EntryAttested { .. }))
        .unwrap();
    let consolidated_pos = events
        .iter()
        .position(|event| matches!(event, EventPayload::EntriesConsolidated { .. }))
        .unwrap();
    assert!(
        attest_pos < consolidated_pos,
        "the attestation precedes the consolidation"
    );
}

#[test]
fn consolidate_into_leaves_an_attributed_attestation_for_an_attributed_source() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let alice = MemoryId::generate();
    let bob = MemoryId::generate();
    let mut block = block(graph, clock, Teller::Agent, Authority::Agent);
    let topic = block
        .create(Namespace::Topic.with_name("aside"), None)
        .unwrap();
    let attributed = block
        .append(
            topic,
            "the same fact, attributed",
            told(Teller::Participant(alice), VisibilityChoice::Attributed),
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
        .consolidate_into(topic, &[attributed], public, None)
        .unwrap();
    let events = block.into_effects().events;

    // Folding an attributed source into a public entry preserves the attribution as an Attributed
    // attestation on the public entry — no audience is widened (both are all-audience).
    let atts = attestations(&events, public);
    assert_eq!(atts.len(), 1);
    assert_eq!(atts[0].teller, Teller::Participant(alice));
    assert_eq!(atts[0].posture, Visibility::Attributed);
    assert_eq!(
        atts[0].phrasing.as_deref(),
        Some("the same fact, attributed")
    );
}

#[test]
fn consolidate_into_suppresses_a_duplicate_attestation() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let alice = MemoryId::generate();
    let mut block = block(graph, clock, Teller::Agent, Authority::Agent);
    let topic = block
        .create(Namespace::Topic.with_name("aside"), None)
        .unwrap();
    // The public entry is founded by alice, so alice already attests it at the public posture. A private
    // source told by alice folded into it would re-attest at a *different* posture (private), which still
    // emits; but a public source told by alice… is never a source. To exercise suppression, the source
    // shares the target's teller *and* posture: two public entries by alice, folding one into the other.
    // (Public is not an eligible source in the pass, but consolidate_into does not re-check eligibility —
    // the clustering layer does — so this drives the suppression path directly.)
    let target = block
        .append(
            topic,
            "the fact",
            told(Teller::Participant(alice), VisibilityChoice::Public),
        )
        .unwrap();
    let source = block
        .append(
            topic,
            "the fact again",
            told(Teller::Participant(alice), VisibilityChoice::Public),
        )
        .unwrap();

    block
        .consolidate_into(topic, &[source], target, None)
        .unwrap();
    let events = block.into_effects().events;

    // alice already attests the target at the public posture (as its founding teller), so the identical
    // attestation the fold would add is suppressed to keep the log lean — the source is still tombstoned.
    assert!(
        attestations(&events, target).is_empty(),
        "an identical (teller, posture) attestation is not re-emitted"
    );
    let retired = events.iter().any(|event| {
        matches!(
            event,
            EventPayload::EntriesConsolidated { sources, replacement, .. }
                if *replacement == target && sources == &[source]
        )
    });
    assert!(
        retired,
        "the source is still retired despite the suppression"
    );
}
