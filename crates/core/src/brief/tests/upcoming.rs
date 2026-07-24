//! The `<upcoming/>` window: a near-future occurrence within the default window surfaces while a far
//! one does not, and the subject-guard suppresses an upcoming aside once its subject is present.
use super::{appended_at, compose_at_epoch, created, materialized};
use crate::{
    event::{Teller, Visibility},
    ids::MemoryId,
    settings::Settings,
    time::{CivilDate, TemporalRef},
};

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
    // A private aside about Marcus with a near-future occurrence, told by Erin: visible in <upcoming/>
    // while only Erin is present, suppressed once Marcus (its subject) is present.
    let marcus = MemoryId::generate();
    let erin = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(marcus, "person/marcus"),
        created(erin, "person/erin"),
        appended_at(
            marcus,
            TemporalRef::Day(CivilDate("1970-01-04".into())),
            "farewell lunch",
            Teller::Participant(erin),
            Visibility::PrivateToTeller,
        ),
    ]);
    let settings = Settings::default().brief;
    let only_erin = compose_at_epoch(&graph, &settings, &[erin], None, &[]);
    assert!(only_erin.contains("farewell lunch"));
    let with_marcus = compose_at_epoch(&graph, &settings, &[erin, marcus], None, &[]);
    assert!(!with_marcus.contains("farewell lunch"));
}
