use super::*;
/// Day-noon millis for a `YYYY-MM-DD`, the `occurred_sort` a `Day` occurrence denormalizes to.
pub(super) fn day_noon(date: &str) -> Timestamp {
    let midnight = CivilDate(date.into()).midnight_millis().unwrap();
    Timestamp::from_millis(midnight + 86_400_000 / 2)
}

/// The post-turn synthesis is now a `response_format`-constrained call: the model returns the
/// `SynthesizeArgs` JSON as its reply (the schema may arrive fenced; the parser locates the object), so
/// a scripted synthesis is a `Reply` carrying that JSON rather than a forced tool call.
pub(super) fn synthesize_call(reply: SynthesizeReply) -> Completion {
    Completion::Reply(serde_json::to_string(&reply).unwrap())
}

/// The description-synthesis reply shape the test scripts as JSON, now typed so a call site reads
/// as what it is rather than a raw string. Mirrors the `SynthesizeArgs` the describe pass sends to
/// the model (see `src/agent/turn/describe.rs`).
#[derive(Debug, Clone, Serialize)]
pub(super) struct SynthesizeReply {
    description: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    occurrences: Vec<SynthesizeOccurrence>,
}

impl SynthesizeReply {
    pub(super) fn description(text: impl Into<String>) -> Self {
        SynthesizeReply {
            description: text.into(),
            occurrences: Vec::new(),
        }
    }

    pub(super) fn with_occurrence(mut self, occurrence: SynthesizeOccurrence) -> Self {
        self.occurrences.push(occurrence);
        self
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SynthesizeOccurrence {
    pub(super) entry: usize,
    occurred_at: SynthesizeTime,
}

impl SynthesizeOccurrence {
    /// An occurrence on a specific day (the common case in tests).
    pub(super) fn day(entry: usize, day: impl Into<String>) -> Self {
        SynthesizeOccurrence {
            entry,
            occurred_at: SynthesizeTime::day(day),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub(super) enum SynthesizeTime {
    Day { day: String },
}

impl SynthesizeTime {
    pub(super) fn day(day: impl Into<String>) -> Self {
        SynthesizeTime::Day { day: day.into() }
    }
}

/// The focused arbitration call's reply: the describe pass now poses the pairwise-contradiction check
/// as its own model call, separate from the description rewrite, so a memory with two or more public
/// entries drives two synthesis calls — a description `synthesize_call`, then this `arbitrate_call`.
/// The reply is the bare `ExtractedArbitration`-shaped object (see `src/agent/turn/describe.rs`).
pub(super) fn arbitrate_call(arbitration: SynthesizeArbitration) -> Completion {
    Completion::Reply(serde_json::to_string(&arbitration).unwrap())
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SynthesizeArbitration {
    pub(super) competing: Vec<usize>,
    pub(super) credited: Vec<usize>,
    pub(super) statement: String,
}

pub(super) fn temporal_resolutions(events: &[Event]) -> Vec<EventPayload> {
    events
        .iter()
        .map(|e| &e.payload)
        .filter(|p| matches!(p, EventPayload::EntryTemporalResolved { .. }))
        .cloned()
        .collect()
}

pub(super) fn temporal_resolve_failures(events: &[Event]) -> Vec<EventPayload> {
    events
        .iter()
        .map(|e| &e.payload)
        .filter(|p| matches!(p, EventPayload::EntryTemporalResolveFailed { .. }))
        .cloned()
        .collect()
}

/// A no-conflict arbitration reply — the describe pass poses a focused arbitration call whenever a
/// memory has two or more public entries (here the seed mirror plus the appended back-pointing entry), so these
/// current-day guard scenarios script it to find nothing.
fn no_conflict() -> Completion {
    arbitrate_call(SynthesizeArbitration {
        competing: Vec::new(),
        credited: Vec::new(),
        statement: String::new(),
    })
}

#[tokio::test]
async fn an_authored_occurrence_survives_a_current_day_extraction() {
    let mut h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.baseline_descriptions();

    // A memory created with an authored October occurrence, then an untimed back-pointing entry ("this date") that
    // the extraction mis-resolves to the conversation's now (2026-06-08, the suite's TEST_NOW).
    let model = ScriptedModel::new([
        run_lua_call(
            r#"local demo = memory.create("event/demo", "Vendor demo", { occurred_at = "2026-10-03", visibility = "public" })
               demo:append("The demo is locked for this date.", { visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        synthesize_call(
            SynthesizeReply::description("Vendor demo, locked.")
                .with_occurrence(SynthesizeOccurrence::day(2, "2026-06-08")),
        ),
        no_conflict(),
    ]);
    run_turn(h.as_turn(&model, "Lock the demo", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    let demo = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Event.with_name("demo"))
        .unwrap()
        .unwrap();
    let entries = h.engine.graph.lock().entries_local(demo.id).unwrap();
    // The authored October occurrence still stands on the seed entry — recall reads October 3rd.
    assert_eq!(entries[0].occurred_sort, Some(day_noon("2026-10-03")));
    assert_eq!(
        entries[0].occurred_at,
        Some(TemporalRef::Day(CivilDate("2026-10-03".into())))
    );
    assert!(entries[0].occurred_authored);
    // The back-pointing entry stays untimed: the current-day resolution was suppressed, not applied.
    assert_eq!(entries[1].occurred_sort, None);
    assert!(temporal_resolutions(&h.events()).is_empty());
    assert_eq!(temporal_resolve_failures(&h.events()).len(), 1);
}

#[tokio::test]
async fn a_differently_dated_extraction_still_applies() {
    let mut h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.baseline_descriptions();

    // Same shape, but the extraction resolves the new entry to a genuinely different day — the demo
    // moved to October 12th — which is not the current day, so the guard leaves it alone.
    let model = ScriptedModel::new([
        run_lua_call(
            r#"local demo = memory.create("event/demo", "Vendor demo", { occurred_at = "2026-10-03", visibility = "public" })
               demo:append("The demo moved to the 12th.", { visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        synthesize_call(
            SynthesizeReply::description("Vendor demo, moved.")
                .with_occurrence(SynthesizeOccurrence::day(2, "2026-10-12")),
        ),
        no_conflict(),
    ]);
    run_turn(h.as_turn(&model, "The demo moved", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    let demo = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Event.with_name("demo"))
        .unwrap()
        .unwrap();
    let entries = h.engine.graph.lock().entries_local(demo.id).unwrap();
    assert_eq!(entries[1].occurred_sort, Some(day_noon("2026-10-12")));
    assert_eq!(temporal_resolutions(&h.events()).len(), 1);
    assert!(temporal_resolve_failures(&h.events()).is_empty());
}

#[tokio::test]
async fn a_current_day_extraction_applies_without_a_dated_sibling() {
    let mut h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.baseline_descriptions();

    // No occurrence anywhere on the memory, so the guard has no differently-dated sibling to fire
    // against: a current-day resolution ("today") applies as an ordinary same-day fact.
    let model = ScriptedModel::new([
        run_lua_call(
            r#"local demo = memory.create("event/demo", "Vendor demo", { visibility = "public" })
               demo:append("It kicked off today.", { visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        synthesize_call(
            SynthesizeReply::description("Vendor demo, underway.")
                .with_occurrence(SynthesizeOccurrence::day(2, "2026-06-08")),
        ),
        no_conflict(),
    ]);
    run_turn(h.as_turn(&model, "The demo started", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    let demo = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Event.with_name("demo"))
        .unwrap()
        .unwrap();
    let entries = h.engine.graph.lock().entries_local(demo.id).unwrap();
    assert_eq!(entries[1].occurred_sort, Some(day_noon("2026-06-08")));
    assert_eq!(temporal_resolutions(&h.events()).len(), 1);
    assert!(temporal_resolve_failures(&h.events()).is_empty());
}

#[tokio::test]
async fn temporal_extraction_resolves_an_untimed_entry() {
    let mut h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.baseline_descriptions();

    let model = ScriptedModel::new([
        // The dated fact is a real appended entry, not the create description — a create's description
        // mirror is exempt from temporal extraction, so a statement to resolve must be an actual entry.
        run_lua_call(
            r#"local dave = memory.create(PERSON_DAVE)
               dave:append("Met Dave last Tuesday", { visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        // The synthesis call resolves statement 1's "last Tuesday" to a concrete day.
        synthesize_call(
            SynthesizeReply::description("Dave, met recently.")
                .with_occurrence(SynthesizeOccurrence::day(1, "2026-06-02")),
        ),
    ]);
    run_turn(h.as_turn(&model, "Remember Dave", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    // The untimed entry gained an occurrence, and an EntryTemporalResolved records it.
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    let entries = h.engine.graph.lock().entries_local(dave.id).unwrap();
    assert_eq!(entries[0].occurred_sort, Some(day_noon("2026-06-02")));
    assert_eq!(temporal_resolutions(&h.events()).len(), 1);
}

#[tokio::test]
async fn temporal_extraction_does_not_override_an_explicit_occurred_at() {
    let mut h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();

    let model = ScriptedModel::new([
        run_lua_call(
            r#"local d = memory.create(PERSON_DAVE)
               d:append("Met Dave", { occurred_at = { day = "2020-01-01" }, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        // The model tries to time statement 1, but the agent already set it explicitly.
        synthesize_call(
            SynthesizeReply::description("Dave.")
                .with_occurrence(SynthesizeOccurrence::day(1, "2026-06-02")),
        ),
    ]);
    run_turn(h.as_turn(&model, "Remember Dave", 8))
        .await
        .unwrap();

    // The explicit occurrence stands; extraction emitted nothing for the already-timed entry.
    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    let entries = h.engine.graph.lock().entries_local(dave.id).unwrap();
    assert_eq!(entries[0].occurred_sort, Some(day_noon("2020-01-01")));
    assert!(temporal_resolutions(&h.events()).is_empty());
}

pub(super) fn belief_arbitrations(events: &[Event]) -> Vec<EventPayload> {
    events
        .iter()
        .map(|e| &e.payload)
        .filter(|p| matches!(p, EventPayload::BeliefArbitrated { .. }))
        .cloned()
        .collect()
}

#[tokio::test]
async fn a_regen_conflict_emits_belief_arbitrated() {
    let mut h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();
    h.baseline_descriptions();

    let model = ScriptedModel::new([
        run_lua_call(
            r#"local d = memory.create(PERSON_DAVE)
               d:append("Dave works at Acme", { by_agent = true, visibility = "public" })
               d:append("Dave works at Hooli", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        // The description call, then the focused arbitration call: statements 1 and 2 conflict, and the
        // arbitration credits the second.
        synthesize_call(SynthesizeReply::description("Dave works at Hooli.")),
        arbitrate_call(SynthesizeArbitration {
            competing: vec![1, 2],
            credited: vec![2],
            statement: "Credited the more recent: Dave works at Hooli.".to_owned(),
        }),
    ]);
    run_turn(h.as_turn(&model, "Where does Dave work?", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    let dave = h
        .engine
        .graph
        .lock()
        .memory_by_name(Namespace::Person.with_name("dave"))
        .unwrap()
        .unwrap();
    let entries = h.engine.graph.lock().entries_local(dave.id).unwrap();
    let arbitrations = belief_arbitrations(&h.events());
    assert_eq!(arbitrations.len(), 1);
    let EventPayload::BeliefArbitrated {
        memory,
        competing_entries,
        resolution,
        produced_by,
    } = &arbitrations[0]
    else {
        unreachable!();
    };
    assert_eq!(*memory, dave.id);
    // The 1-based statement numbers resolved to the two entries' ids, in order.
    assert_eq!(
        *competing_entries,
        vec![entries[0].entry_id, entries[1].entry_id]
    );
    assert_eq!(resolution.credited, vec![entries[1].entry_id]);
    assert!(resolution.statement.contains("Hooli"));
    assert!(produced_by.is_some());
}

#[tokio::test]
async fn a_single_sided_arbitration_is_dropped() {
    let mut h = Harness::new();
    genesis::rollout(
        h.engine.store.lock().as_mut(),
        &h.clock,
        &seed(),
        None,
        &InstanceFeatures::default(),
    )
    .unwrap();
    h.engine
        .graph
        .lock()
        .materialize_from(h.engine.store.lock().as_ref())
        .unwrap();

    h.baseline_descriptions();

    // Two public entries put the arbitration call in play, but the arbitration names only one competing
    // statement — not a real conflict, so [`arbitration_event`]'s >= 2 validation drops it.
    let model = ScriptedModel::new([
        run_lua_call(
            r#"local d = memory.create(PERSON_DAVE)
               d:append("Dave works at Acme", { by_agent = true, visibility = "public" })
               d:append("Dave is a climber", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("Noted.".to_owned()),
        synthesize_call(SynthesizeReply::description("Dave.")),
        arbitrate_call(SynthesizeArbitration {
            competing: vec![1],
            credited: vec![1],
            statement: "only one side".to_owned(),
        }),
    ]);
    run_turn(h.as_turn(&model, "Remember Dave", 8))
        .await
        .unwrap();
    h.describe(&model).await;

    assert!(belief_arbitrations(&h.events()).is_empty());
}
