//! Attestation projection and visibility tests (spec §Visibility → attestations). Every entry
//! materializes with a founding attestation derived from its own append, so a pre-attestation log
//! replays with singleton sets whose verdicts are bit-identical to the pre-change fold; a further
//! teller's `EntryAttested` widens the set, and the visibility predicate takes the widest passing
//! verdict over the live attestations.

use std::collections::BTreeSet;

use crate::{
    event::{EventPayload, Teller, Visibility},
    graph::tests::materialized,
    ids::{EntryId, MemoryId, Namespace},
    time::Timestamp,
    visibility::{visible, visible_attestations},
};

/// A content append with an explicit teller and posture — the founding attestation's source.
fn appended(
    id: MemoryId,
    entry_id: EntryId,
    text: &str,
    told_by: Teller,
    visibility: Visibility,
) -> EventPayload {
    EventPayload::MemoryContentAppended {
        id,
        entry_id,
        asserted_at: Timestamp::from_millis(900),
        occurred_at: None,
        text: text.to_owned(),
        told_by,
        told_in: None,
        visibility,
    }
}

/// A further teller's attestation of an existing entry.
fn attested(
    memory: MemoryId,
    entry: EntryId,
    teller: Teller,
    posture: Visibility,
    phrasing: Option<&str>,
) -> EventPayload {
    EventPayload::EntryAttested {
        memory,
        entry,
        teller,
        told_in: None,
        asserted_at: Timestamp::from_millis(1_500),
        posture,
        phrasing: phrasing.map(str::to_owned),
        source_entry: None,
        produced_by: None,
    }
}

#[test]
fn a_plain_log_replays_with_a_singleton_founding_attestation() {
    // The core migration story: an entry appended before attestations existed materializes with
    // exactly one attestation, derived from its own told_by/visibility, so the fold is unchanged.
    let marcus = MemoryId::generate();
    let erin = MemoryId::generate();
    let (pub_entry, aside) = (EntryId::generate(), EntryId::generate());
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(marcus, Namespace::Person.with_name("marcus")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        appended(
            marcus,
            pub_entry,
            "plays cello",
            Teller::Agent,
            Visibility::Public,
        ),
        appended(
            marcus,
            aside,
            "an aside about marcus",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
    ]);

    let entries = graph.entries_local(marcus).unwrap();
    assert_eq!(entries.len(), 2);
    for entry in &entries {
        assert_eq!(
            entry.attestations.len(),
            1,
            "each entry carries exactly its founding attestation"
        );
    }
    let aside_entry = entries.iter().find(|e| e.entry_id == aside).unwrap();
    let founding = &aside_entry.attestations[0];
    assert_eq!(founding.teller, Teller::Participant(erin));
    assert_eq!(founding.posture, Visibility::PrivateToTeller);

    // The verdicts match the pre-attestation predicate exactly for the representative postures.
    let class_of = |id| graph.class_id(id).map(|class| class.unwrap_or(id));
    let marcus_memory = graph.memory_by_id(marcus).unwrap().unwrap();
    let pub_view = entries.iter().find(|e| e.entry_id == pub_entry).unwrap();
    // Public: surfaces to anyone, including the subject.
    assert!(visible(pub_view, &marcus_memory, &[marcus], &class_of).unwrap());
    // PrivateToTeller: teller present and subject absent → visible.
    assert!(visible(aside_entry, &marcus_memory, &[erin], &class_of).unwrap());
    // Teller absent → hidden.
    assert!(!visible(aside_entry, &marcus_memory, &[], &class_of).unwrap());
    // Subject present → hidden by the subject-guard.
    assert!(!visible(aside_entry, &marcus_memory, &[erin, marcus], &class_of).unwrap());
}

#[test]
fn a_public_founding_renders_to_all_while_a_private_attestation_stays_a_chip() {
    // A Public founding entry with a further teller's PrivateToTeller attestation (narrower, so the
    // audience-widening invariant holds): the fact renders to everyone (widest verdict Public), but
    // the private attestation is a hidden chip — visible only where its own posture passes.
    let marcus = MemoryId::generate();
    let erin = MemoryId::generate();
    let entry = EntryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(marcus, Namespace::Person.with_name("marcus")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        appended(
            marcus,
            entry,
            "plays cello",
            Teller::Agent,
            Visibility::Public,
        ),
        attested(
            marcus,
            entry,
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
            Some("erin heard the same"),
        ),
    ]);

    let entries = graph.entries_local(marcus).unwrap();
    let view = &entries[0];
    assert_eq!(
        view.attestations.len(),
        2,
        "founding plus erin's attestation"
    );

    let class_of = |id| graph.class_id(id).map(|class| class.unwrap_or(id));
    let memory = graph.memory_by_id(marcus).unwrap().unwrap();
    // The fact renders regardless of who is present — the Public founding is the widest verdict.
    assert!(visible(view, &memory, &[], &class_of).unwrap());
    assert!(visible(view, &memory, &[marcus], &class_of).unwrap());

    // Chip rule: with no one present, only the founding (agent, public) attestation shows.
    let none_present = visible_attestations(view, &memory, &[], &class_of).unwrap();
    assert_eq!(none_present.len(), 1);
    assert_eq!(none_present[0].teller, Teller::Agent);
    // With erin present, her private attestation joins the visible subset.
    let erin_present = visible_attestations(view, &memory, &[erin], &class_of).unwrap();
    assert_eq!(erin_present.len(), 2);
    // With the subject present, erin's aside about marcus is suppressed; the founding still shows.
    let subject_present = visible_attestations(view, &memory, &[erin, marcus], &class_of).unwrap();
    assert_eq!(subject_present.len(), 1);
    assert_eq!(subject_present[0].teller, Teller::Agent);
}

#[test]
fn a_private_founding_with_narrower_attestations_stays_private() {
    // A PrivateToTeller founding entry told by erin, plus a second PrivateToTeller attestation by
    // frank. The union surfaces when *either* teller is present, but it is never public and never
    // reaches the subject — a private founding stays private no matter how many tellers stand behind it.
    let marcus = MemoryId::generate();
    let (erin, frank) = (MemoryId::generate(), MemoryId::generate());
    let entry = EntryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(marcus, Namespace::Person.with_name("marcus")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        EventPayload::memory_created(frank, Namespace::Person.with_name("frank")),
        appended(
            marcus,
            entry,
            "an aside",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
        attested(
            marcus,
            entry,
            Teller::Participant(frank),
            Visibility::PrivateToTeller,
            None,
        ),
    ]);

    let entries = graph.entries_local(marcus).unwrap();
    let view = &entries[0];
    let class_of = |id| graph.class_id(id).map(|class| class.unwrap_or(id));
    let memory = graph.memory_by_id(marcus).unwrap().unwrap();

    // No teller present → hidden.
    assert!(!visible(view, &memory, &[], &class_of).unwrap());
    // Either teller present (subject absent) → visible.
    assert!(visible(view, &memory, &[erin], &class_of).unwrap());
    assert!(visible(view, &memory, &[frank], &class_of).unwrap());
    // A teller present but the subject too → hidden by the subject-guard.
    assert!(!visible(view, &memory, &[erin, marcus], &class_of).unwrap());
    assert!(!visible(view, &memory, &[frank, marcus], &class_of).unwrap());
}

#[test]
fn a_re_attestation_by_the_same_teller_is_last_writer_wins() {
    let marcus = MemoryId::generate();
    let erin = MemoryId::generate();
    let entry = EntryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(marcus, Namespace::Person.with_name("marcus")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        appended(
            marcus,
            entry,
            "plays cello",
            Teller::Agent,
            Visibility::Public,
        ),
        attested(
            marcus,
            entry,
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
            Some("first"),
        ),
        attested(
            marcus,
            entry,
            Teller::Participant(erin),
            Visibility::Attributed,
            Some("second"),
        ),
    ]);

    let entries = graph.entries_local(marcus).unwrap();
    let view = &entries[0];
    // The same teller attesting twice leaves one row, updated to the latest posture and phrasing.
    assert_eq!(view.attestations.len(), 2);
    let erin_attestation = view
        .attestations
        .iter()
        .find(|a| a.teller == Teller::Participant(erin))
        .unwrap();
    assert_eq!(erin_attestation.posture, Visibility::Attributed);
    assert_eq!(erin_attestation.phrasing.as_deref(), Some("second"));
}

#[test]
fn retracting_one_attestation_leaves_the_entry_live() {
    // Two tellers stand behind the fact; withdrawing one leaves the other, so the entry stays live
    // and carries the remaining attestation.
    let project = MemoryId::generate();
    let (erin, frank) = (MemoryId::generate(), MemoryId::generate());
    let entry = EntryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(project, Namespace::Topic.with_name("hooli")),
        appended(
            project,
            entry,
            "the launch slipped",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
        attested(
            project,
            entry,
            Teller::Participant(frank),
            Visibility::PrivateToTeller,
            None,
        ),
        EventPayload::attestation_retracted(
            project,
            entry,
            Teller::Participant(frank),
            "frank withdrew",
            None,
        ),
    ]);

    let entries = graph.entries_local(project).unwrap();
    assert_eq!(entries.len(), 1, "the entry stays live");
    let view = &entries[0];
    assert_eq!(
        view.attestations.len(),
        1,
        "only the live attestation remains"
    );
    assert_eq!(view.attestations[0].teller, Teller::Participant(erin));
}

#[test]
fn withdrawing_the_last_attestation_tombstones_the_entry() {
    // When no teller still stands behind the fact, the entry is tombstoned exactly as EntryRetracted
    // does: it drops from live surfaces and history shows the reason.
    let project = MemoryId::generate();
    let erin = MemoryId::generate();
    let entry = EntryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(project, Namespace::Topic.with_name("hooli")),
        appended(
            project,
            entry,
            "the launch slipped",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
        EventPayload::attestation_retracted(
            project,
            entry,
            Teller::Participant(erin),
            "erin retracted it",
            None,
        ),
    ]);

    // Gone from the live read.
    assert!(graph.entries_local(project).unwrap().is_empty());
    // Present in history, tombstoned by its own id with the reason recorded.
    let history = graph.entries_local_history(project).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].superseded_by, Some(entry));
    assert_eq!(
        history[0].retracted_reason.as_deref(),
        Some("erin retracted it")
    );
}

#[test]
fn a_whole_entry_retraction_withdraws_every_attestation() {
    // EntryRetracted retires the fact outright, so every teller's attestation is withdrawn — the
    // entry tombstones and no attestation stays live.
    let project = MemoryId::generate();
    let (erin, frank) = (MemoryId::generate(), MemoryId::generate());
    let entry = EntryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(project, Namespace::Topic.with_name("hooli")),
        appended(
            project,
            entry,
            "the launch slipped",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
        attested(
            project,
            entry,
            Teller::Participant(frank),
            Visibility::PrivateToTeller,
            None,
        ),
        EventPayload::entry_retracted(project, entry, "filed on the wrong memory", None),
    ]);

    assert!(graph.entries_local(project).unwrap().is_empty());
    let history = graph.entries_local_history(project).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(
        history[0].retracted_reason.as_deref(),
        Some("filed on the wrong memory")
    );
    // The live attestation fetch returns none — both tellers' attestations were withdrawn.
    assert!(
        history[0].attestations.is_empty(),
        "no attestation stays live after a whole-entry retraction"
    );
}

#[test]
fn the_provenance_marker_names_visible_attesters_and_leaks_no_hidden_one() {
    // A public fact with a visible attributed corroborator (dave) and a hidden private confidence
    // (frank, absent): the surfaced marker names dave and carries no residue of frank — the
    // load-bearing privacy property of the chip rule at the render seam.
    let project = MemoryId::generate();
    let (dave, frank) = (MemoryId::generate(), MemoryId::generate());
    let entry = EntryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(project, Namespace::Topic.with_name("hooli")),
        EventPayload::memory_created(dave, Namespace::Person.with_name("dave")),
        EventPayload::memory_created(frank, Namespace::Person.with_name("frank")),
        appended(
            project,
            entry,
            "the launch slipped",
            Teller::Agent,
            Visibility::Public,
        ),
        attested(
            project,
            entry,
            Teller::Participant(dave),
            Visibility::Attributed,
            None,
        ),
        attested(
            project,
            entry,
            Teller::Participant(frank),
            Visibility::PrivateToTeller,
            Some("frank said so in confidence"),
        ),
    ]);

    let view = &graph.entries_local(project).unwrap()[0];
    let memory = graph.memory_by_id(project).unwrap().unwrap();
    // No one present: dave's attributed corroboration is public-safe and shows; frank's confidence
    // (teller absent) is hidden and leaves no residue.
    let marker = graph
        .entry_provenance_marker(view, &memory, &[])
        .unwrap()
        .expect("a corroborated public entry carries a marker");
    assert_eq!(marker, "[also told by person/dave]");
    assert!(
        !marker.contains("frank"),
        "the hidden attester never surfaces: {marker}"
    );

    // With frank present, his confidence joins the visible subset — but the fact is public, so it
    // rides the corroboration register (still no confidential wording leaked into a shareable line).
    let marker = graph
        .entry_provenance_marker(view, &memory, &[frank])
        .unwrap()
        .expect("still corroborated");
    assert!(!marker.contains("confidence"), "{marker}");
}

#[test]
fn exclude_semantics_hold_through_the_attestation_path() {
    // An Exclude founding on a non-person memory (no subject-guard): visible while the teller is
    // present and no named excludee is, mirroring the pre-attestation Exclude semantics.
    let project = MemoryId::generate();
    let (erin, dave) = (MemoryId::generate(), MemoryId::generate());
    let entry = EntryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(project, Namespace::Topic.with_name("hooli")),
        appended(
            project,
            entry,
            "an aside implicating dave",
            Teller::Participant(erin),
            Visibility::Exclude(BTreeSet::from([dave])),
        ),
    ]);

    let entries = graph.entries_local(project).unwrap();
    let view = &entries[0];
    let class_of = |id| graph.class_id(id).map(|class| class.unwrap_or(id));
    let memory = graph.memory_by_id(project).unwrap().unwrap();
    // Teller present, excludee absent → visible.
    assert!(visible(view, &memory, &[erin], &class_of).unwrap());
    // Excludee present → hidden.
    assert!(!visible(view, &memory, &[erin, dave], &class_of).unwrap());
}

#[test]
fn a_founding_tellers_re_attest_keeps_the_founding_attestation_first() {
    // The founding teller narrows their own attestation (a later private re-affirmation of a fact
    // they founded publicly). The LWW upsert updates the posture but keeps the row's original seq,
    // so the founding attestation still leads the read — the marker assembly and the visibility
    // fallback both key on founding-first.
    let project = MemoryId::generate();
    let erin = MemoryId::generate();
    let dave = MemoryId::generate();
    let entry = EntryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(project, Namespace::Topic.with_name("hooli")),
        appended(
            project,
            entry,
            "the launch slipped",
            Teller::Participant(erin),
            Visibility::Public,
        ),
        attested(
            project,
            entry,
            Teller::Participant(dave),
            Visibility::Attributed,
            None,
        ),
        attested(
            project,
            entry,
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
            None,
        ),
    ]);
    let (_, view) = graph.entry_by_id(entry).unwrap().expect("the entry exists");
    assert_eq!(
        view.attestations[0].teller,
        Teller::Participant(erin),
        "the founding attestation leads the read after its own re-attest"
    );
    assert_eq!(view.attestations[0].posture, Visibility::PrivateToTeller);
    assert_eq!(view.attestations[1].teller, Teller::Participant(dave));
}

#[test]
fn a_withdrawn_attestation_reaches_history_but_no_live_surface() {
    // Frank's withdrawn account: absent from the live read's attestation set, present on the
    // history read with its reason (the console renders the withdrawal struck-through), and — even
    // on a history view — contributing neither a chip nor a widening verdict.
    let project = MemoryId::generate();
    let (erin, frank) = (MemoryId::generate(), MemoryId::generate());
    let entry = EntryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(project, Namespace::Topic.with_name("hooli")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        EventPayload::memory_created(frank, Namespace::Person.with_name("frank")),
        appended(
            project,
            entry,
            "the launch slipped",
            Teller::Participant(erin),
            Visibility::Public,
        ),
        attested(
            project,
            entry,
            Teller::Participant(frank),
            Visibility::Public,
            None,
        ),
        EventPayload::AttestationRetracted {
            memory: project,
            entry,
            teller: Teller::Participant(frank),
            reason: "walked it back".to_owned(),
            produced_by: None,
        },
    ]);

    let live = graph.class_entries(project).unwrap();
    assert_eq!(
        live[0].attestations.len(),
        1,
        "the live read carries only erin's founding attestation"
    );

    let history = graph.class_history(project).unwrap();
    let withdrawn: Vec<_> = history[0]
        .attestations
        .iter()
        .filter(|attestation| attestation.retracted_reason.is_some())
        .collect();
    assert_eq!(
        withdrawn.len(),
        1,
        "the history read carries the withdrawal"
    );
    assert_eq!(
        withdrawn[0].retracted_reason.as_deref(),
        Some("walked it back")
    );

    // Even on the history view, the withdrawn account is invisible to the chip engine.
    let memory = graph.memory_by_id(project).unwrap().unwrap();
    let class_of = |id: MemoryId| -> Result<MemoryId, crate::graph::GraphError> { Ok(id) };
    let visible =
        crate::visibility::visible_attestations(&history[0], &memory, &[erin], &class_of).unwrap();
    assert!(
        visible
            .iter()
            .all(|attestation| attestation.teller != Teller::Participant(frank)),
        "a withdrawn account never renders as a chip"
    );
}
