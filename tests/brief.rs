//! Contextual-brief composition tests (spec appendix scenarios 2, 14, 21 — the deterministic
//! `[brief]`/`[predicate]` surface). Each builds a materialized graph, composes a brief for a present
//! set, and asserts a fact is present or absent — model-free, because composition is deterministic.

#![cfg(feature = "sqlite")]

use zuihitsu::{
    BriefRequest, BriefSettings, CivilDate, Graph, MemoryId, MemoryName, MemoryStore, Settings,
    Store, TagName, Teller, TemporalRef, Timestamp, Visibility, brief, event::EventPayload,
    ids::EntryId,
};

/// Compose a brief at the epoch (these deterministic tests don't exercise the time-relative
/// `<upcoming/>` window unless they plant a future occurrence, so a fixed `now` keeps them stable).
fn compose_at_epoch(
    graph: &Graph,
    settings: &BriefSettings,
    present_set: &[MemoryId],
    current_context: Option<MemoryId>,
    working_set: &[MemoryId],
) -> String {
    brief::compose(
        graph,
        settings,
        &BriefRequest {
            present_set,
            current_context,
            working_set,
            now: Timestamp::from_millis(0),
        },
    )
    .unwrap()
}

/// A content append carrying an `occurred_at` (the `appended` helper above leaves it `None`).
fn appended_at(
    id: MemoryId,
    occurred_at: TemporalRef,
    text: &str,
    told_by: Teller,
    visibility: Visibility,
) -> EventPayload {
    EventPayload::MemoryContentAppended {
        id,
        entry_id: EntryId::generate(),
        asserted_at: Timestamp::from_millis(0),
        occurred_at: Some(occurred_at),
        text: text.to_owned(),
        told_by,
        told_in: None,
        visibility,
    }
}

/// Build a store, append `payloads`, and materialize a fresh in-memory graph from them.
fn materialized(payloads: Vec<EventPayload>) -> (MemoryStore, Graph) {
    let mut store = MemoryStore::new();
    store
        .append(Timestamp::from_millis(1_000), payloads)
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    (store, graph)
}

fn created(id: MemoryId, name: &str) -> EventPayload {
    EventPayload::MemoryCreated {
        id,
        name: MemoryName::new(name),
    }
}

fn appended(
    id: MemoryId,
    at_ms: i64,
    text: &str,
    told_by: Teller,
    visibility: Visibility,
) -> EventPayload {
    EventPayload::MemoryContentAppended {
        id,
        entry_id: EntryId::generate(),
        asserted_at: Timestamp::from_millis(at_ms),
        occurred_at: None,
        text: text.to_owned(),
        told_by,
        told_in: None,
        visibility,
    }
}

#[test]
fn current_room_brief_shows_confidential_regardless_of_present_set() {
    // Scenario 14: #leads is #confidential. A later session has Phil and Dave but not the teller;
    // the current-context brief still shows confidential — it's a memory-level tag, not teller-gated.
    let leads = MemoryId::generate();
    let phil = MemoryId::generate();
    let dave = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(leads, "context/leads"),
        EventPayload::TagCreated {
            name: TagName::new("confidential"),
            description: "confidential room".to_owned(),
        },
        EventPayload::TagAppliedToMemory {
            memory: leads,
            tag: TagName::new("confidential"),
        },
        created(phil, "person/phil"),
        created(dave, "person/dave"),
    ]);

    let out = compose_at_epoch(
        &graph,
        &Settings::default().brief,
        &[phil, dave],
        Some(leads),
        &[],
    );
    assert!(out.contains("Current room: #leads (confidential)"));
}

#[test]
fn an_aside_about_a_present_subject_is_suppressed_in_the_brief() {
    // Scenario 2 (composition half): Erin's private aside about Phil. With Phil present, his brief
    // block renders his public fact but the subject-guard suppresses the aside. (The surfaces-while-
    // absent half and the join injection complete at the join increment.)
    let phil = MemoryId::generate();
    let erin = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(phil, "person/phil"),
        created(erin, "person/erin"),
        appended(
            phil,
            1_000,
            "on the platform team",
            Teller::Agent,
            Visibility::Public,
        ),
        appended(
            phil,
            1_100,
            "is being managed out",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
    ]);

    let out = compose_at_epoch(&graph, &Settings::default().brief, &[erin, phil], None, &[]);
    assert!(out.contains("on the platform team")); // Phil's block renders
    assert!(!out.contains("is being managed out")); // ...but the aside is suppressed
}

#[test]
fn a_subject_joining_suppresses_asides_about_them() {
    // Scenario 2 (join half): Erin's private aside about Phil. While only Erin is present it is
    // visible (it would surface to her). Phil's join-brief is built against the now-present set
    // {Erin, Phil}, where the subject-guard suppresses it — the dangerous direction is closed.
    let phil = MemoryId::generate();
    let erin = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(phil, "person/phil"),
        created(erin, "person/erin"),
        appended(
            phil,
            1_000,
            "is being managed out",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
    ]);
    let settings = Settings::default().brief;

    // Before Phil joins (only Erin present): the aside is visible.
    let before = brief::compose_participant(&graph, phil, &[erin], &settings).unwrap();
    assert!(before.contains("is being managed out"));

    // Phil's join-brief, built against {Erin, Phil}: the subject-guard suppresses it.
    let join_brief = brief::compose_participant(&graph, phil, &[erin, phil], &settings).unwrap();
    assert!(!join_brief.contains("is being managed out"));
}

#[test]
fn the_working_set_is_re_filtered_against_the_new_present_set() {
    // The working set carried across a compaction is re-filtered through `visible` against the *new*
    // present set, never trusted from the old session: Erin's private aside about Phil surfaces in
    // active threads while only Erin is present, but is suppressed once Phil is present at the new
    // segment boundary (the safety property fixture 22 guards, at the deterministic level).
    let phil = MemoryId::generate();
    let erin = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(phil, "person/phil"),
        created(erin, "person/erin"),
        appended(
            phil,
            1_000,
            "is being managed out",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
    ]);
    let settings = Settings::default().brief;

    // Phil is in the working set. With only Erin present, the aside is visible in active threads.
    let only_erin = compose_at_epoch(&graph, &settings, &[erin], None, &[phil]);
    assert!(only_erin.contains("# Active threads"));
    assert!(only_erin.contains("is being managed out"));

    // With Phil present at the new boundary, the aside is suppressed — the working-set copy is
    // re-filtered against {Erin, Phil} just like any other block.
    let with_phil = compose_at_epoch(&graph, &settings, &[erin, phil], None, &[phil]);
    assert!(!with_phil.contains("is being managed out"));
}

#[test]
fn the_present_set_cap_does_not_narrow_the_predicate() {
    // Scenario 21: with the present-set cap set to 1, Dave is present but ranks below the cap (only a
    // name-only entry, no full block). A fact on Phil (in the cap, rendered) excludes Dave; the
    // exclude must still fire, because the predicate resolves against the full present set — not the
    // capped one. Told by Phil himself, so the subject-guard does not also suppress it: the exclude
    // is the only thing gating it, isolating the cap-vs-predicate separation.
    let phil = MemoryId::generate();
    let dave = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(phil, "person/phil"),
        created(dave, "person/dave"),
        // Phil has the more recent activity, so he ranks into the cap of 1; Dave falls below it.
        appended(
            phil,
            2_000,
            "joined the climbing gym",
            Teller::Participant(phil),
            Visibility::Public,
        ),
        EventPayload::MemoryContentAppended {
            id: phil,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(2_100),
            occurred_at: None,
            text: "thinking of leaving, keep it from Dave".to_owned(),
            told_by: Teller::Participant(phil),
            told_in: None,
            visibility: Visibility::Exclude(vec![dave]),
        },
    ]);

    let mut settings = Settings::default().brief;
    settings.present_set_cap = 1;
    let out = compose_at_epoch(&graph, &settings, &[phil, dave], None, &[]);

    assert!(out.contains("joined the climbing gym")); // Phil's block renders (in the cap)
    assert!(out.contains("person/dave (present)")); // Dave is present but below the cap (name-only)
    // The exclude fires because Dave is in the full present set, despite ranking below the cap.
    assert!(!out.contains("keep it from Dave"));
}

#[test]
fn upcoming_block_lists_near_future_items_within_the_window() {
    // now = epoch (day 0). The dentist on day 3 falls in the default 7-day window; the far review on
    // day 30 does not.
    let dentist = MemoryId::generate();
    let far = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(dentist, "event/dentist"),
        appended_at(
            dentist,
            TemporalRef::Day(CivilDate("1970-01-04".into())),
            "cleaning",
            Teller::Agent,
            Visibility::Public,
        ),
        created(far, "event/far"),
        appended_at(
            far,
            TemporalRef::Day(CivilDate("1970-01-31".into())),
            "annual review",
            Teller::Agent,
            Visibility::Public,
        ),
    ]);
    let out = compose_at_epoch(&graph, &Settings::default().brief, &[], None, &[]);
    assert!(out.contains("# Upcoming"));
    assert!(out.contains("cleaning"));
    assert!(!out.contains("annual review")); // beyond the 7-day window
}

#[test]
fn upcoming_respects_the_subject_guard() {
    // A private aside about Phil with a near-future occurrence, told by Erin: visible in <upcoming/>
    // while only Erin is present, suppressed once Phil (its subject) is present.
    let phil = MemoryId::generate();
    let erin = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(phil, "person/phil"),
        created(erin, "person/erin"),
        appended_at(
            phil,
            TemporalRef::Day(CivilDate("1970-01-04".into())),
            "farewell lunch",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
    ]);
    let settings = Settings::default().brief;
    let only_erin = compose_at_epoch(&graph, &settings, &[erin], None, &[]);
    assert!(only_erin.contains("farewell lunch"));
    let with_phil = compose_at_epoch(&graph, &settings, &[erin, phil], None, &[]);
    assert!(!with_phil.contains("farewell lunch"));
}
