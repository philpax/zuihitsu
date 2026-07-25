//! The maintenance-sweep completion record: each pass driver appends a `MaintenancePassCompleted`
//! observation under [`EventSource::Orchestration`], so the operator's maintenance history shows when
//! a pass ran, over what window, and how much it did. Exercised over the in-memory backends by driving
//! the on-demand pass facade (`MaintenanceStart::FromStart`), which records the same event the timer
//! driver does.

use crate::{
    Instance,
    clock::ManualClock,
    event::{EventPayload, MaintenancePass},
    ids::{EntryId, MemoryId, Namespace, Seq},
    model::{Completion, ModelClient, ScriptedModel},
    time::Timestamp,
};

/// A born in-memory instance. Genesis seeds the maintenance templates and the `same_as` relation, so a
/// canonicalize sweep runs rather than no-opping on a missing template.
fn born_server() -> Instance {
    let server =
        Instance::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    server
        .control()
        .create_agent(&crate::SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "A discreet companion with a long memory.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    server
}

/// Every `MaintenancePassCompleted` on the log, as `(seq, payload)` — the record the operator's
/// maintenance history folds.
fn completions(server: &Instance) -> Vec<(Seq, MaintenancePass, Seq, Seq, u32)> {
    server
        .control()
        .events()
        .unwrap()
        .into_iter()
        .filter_map(|event| match event.payload {
            EventPayload::MaintenancePassCompleted {
                pass,
                from,
                to,
                actions,
            } => Some((event.seq, pass, from, to, actions)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_canonicalize_sweep_records_its_completion_with_the_window_and_count() {
    let server = born_server();
    // A platform stub with a name-bearing entry: the canonicalize pass reads it, the model identifies
    // the name, and the pass mints a `person/dave` profile and designates it primary — one committed
    // effect.
    let stub = MemoryId::generate();
    server
        .control()
        .seed_events(vec![
            EventPayload::memory_created(stub, Namespace::Person.with_name("dave@discord")),
            EventPayload::MemoryContentAppended {
                id: stub,
                entry_id: EntryId::generate(),
                asserted_at: Timestamp::from_millis(1_000),
                occurred_at: None,
                text: "Goes by Dave on the server.".to_owned(),
                told_by: crate::event::Teller::Agent,
                told_in: None,
                visibility: crate::event::Visibility::Public,
            },
            EventPayload::participant_identified(stub, "discord", "dave#0001"),
        ])
        .unwrap();
    let head_before = server.engine.store.lock().head().unwrap();

    let model = ScriptedModel::new([Completion::Reply(r#"{"name": "dave"}"#.to_owned())]);
    server
        .canonicalize_catch_up(&model as &dyn ModelClient)
        .await
        .unwrap();

    // Exactly one sweep record, naming the pass, the swept window, and the one committed effect.
    let records = completions(&server);
    assert_eq!(records.len(), 1, "one canonicalize sweep is recorded");
    let (record_seq, pass, from, to, actions) = records[0];
    assert_eq!(pass, MaintenancePass::Canonicalize);
    assert_eq!(
        from,
        Seq::ZERO,
        "the on-demand sweep starts from the log start"
    );
    assert_eq!(
        to, head_before,
        "the window ends at the head the sweep swept to"
    );
    assert_eq!(actions, 1, "one canonical profile was designated");
    // The record is appended after the pass's own effects, which sit past the head it read.
    assert!(
        record_seq > to,
        "the record lands after the window it swept"
    );

    // Sanity: the sweep actually designated a primary (the effect the count reflects).
    let designated = server.control().events().unwrap().into_iter().any(|event| {
        matches!(
            event.payload,
            EventPayload::ClassPrimaryDesignated { designated, .. } if designated
        )
    });
    assert!(designated, "the sweep designated a canonical primary");
}

#[tokio::test]
async fn a_no_op_sweep_still_records_with_zero_actions() {
    let server = born_server();
    // No platform stubs exist, so the sweep finds nothing to canonicalize — but it still records that
    // it ran, with a zero action count, so the operator sees the machinery is alive.
    let head_before = server.engine.store.lock().head().unwrap();
    let model = ScriptedModel::new([]);
    server
        .canonicalize_catch_up(&model as &dyn ModelClient)
        .await
        .unwrap();

    let records = completions(&server);
    assert_eq!(records.len(), 1, "the quiet sweep is still recorded");
    let (_, pass, from, to, actions) = records[0];
    assert_eq!(pass, MaintenancePass::Canonicalize);
    assert_eq!(from, Seq::ZERO);
    assert_eq!(to, head_before);
    assert_eq!(
        actions, 0,
        "a sweep with nothing to do records zero actions"
    );
}
