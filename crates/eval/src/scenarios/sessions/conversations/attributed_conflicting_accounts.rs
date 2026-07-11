use super::*;

/// The relayed-secondhand mirror of [`ConflictingAccounts`]: two people relay *other people's*
/// conflicting accounts of where an event is held, each recorded as an `Attributed` (via-a-teller)
/// fact rather than `Public` — a relayed fact surfaces like a public one but keeps its "via <teller>"
/// provenance and stays out of the always-visible description. The two accounts are **planted directly**
/// as `Attributed` entries via [`EvalStep::SeedEvents`], because the natural classification of a relayed
/// fact into `Attributed` is not what this scenario tests: an N=8 run showed the agent records such
/// secondhand phrasing as `Public` with `told_by` provenance every time, so conversational steering
/// never lands the `Attributed` posture the widened pool needs. Seeding fixes the state and isolates
/// the behavior under test.
///
/// What is under test starts once the two `Attributed` accounts stand side by side. Arbitration must
/// still catch the contradiction over the widened `Public` + `Attributed` pool — two accounts marked
/// `Attributed` must collide and be arbitrated, keeping both — neither relayed location may leak into
/// the memory's regenerated description, and asked from another room the agent should surface the
/// discrepancy rather than confidently pick one.
///
/// The gate is the description-exclusion: an attributed fact's exclusion from the summary is a
/// provenance property, not audience safety, but it is the invariant that must hold even as the
/// widened pool now feeds those same entries to arbitration. The arbitration and cross-room recall are
/// reported as rates, mirroring [`ConflictingAccounts`], whose trend history this scenario deliberately
/// does not disturb.
pub struct AttributedConflictingAccounts;

/// The two relayed location values the scenario plants as `Attributed` facts. Each is authored in the
/// seed, so its absence from every regenerated description is the exact structural signal that the
/// attributed content stayed out of the always-visible summary.
const RELAYED_LOCATIONS: &[&str] = &["auditorium", "terrace"];

#[async_trait]
impl Scenario for AttributedConflictingAccounts {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "attributed_conflicting_accounts".to_owned(),
            category: Category::Sessions,
            description: "Two conflicting secondhand accounts of where the all-hands is held are \
                          planted directly as Attributed (via-a-teller) facts — deterministic seeding, \
                          since the natural classification of a relayed fact into Attributed is not \
                          what is under test. Arbitration must catch the contradiction over the widened \
                          Public + Attributed pool, the relayed locations must stay out of the event's \
                          always-visible description, and asked from another room the agent should \
                          surface the discrepancy rather than confidently pick one."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        // Plant the two relayed accounts directly as `Attributed` facts on the all-hands memory, each
        // via a different original teller (`person/priya` heard the auditorium, `person/devon` the
        // rooftop terrace). Seeding rather than conversational steering makes the `Attributed` posture
        // deterministic: the agent classifying relayed phrasing into `Attributed` is not what is under
        // test — the arbitration and description passes over the resulting widened pool are.
        let priya = MemoryId::generate();
        let devon = MemoryId::generate();
        let all_hands = MemoryId::generate();
        let now = Timestamp::from_millis(RUN_START_MS);
        let seed = vec![
            EventPayload::memory_created(priya, MemoryName::new("person/priya")),
            EventPayload::memory_created(devon, MemoryName::new("person/devon")),
            EventPayload::memory_created(all_hands, MemoryName::new("event/all-hands")),
            // Priya's relayed account, held `Attributed` via Priya.
            EventPayload::MemoryContentAppended {
                id: all_hands,
                entry_id: EntryId::generate(),
                asserted_at: now,
                occurred_at: None,
                text: "The all-hands next week is being held in the main auditorium.".to_owned(),
                told_by: Teller::Participant(priya),
                told_in: None,
                visibility: Visibility::Attributed,
            },
            // Devon's conflicting relayed account, held `Attributed` via Devon — the contradiction the
            // widened arbitration pool must catch even though neither entry is `Public`.
            EventPayload::MemoryContentAppended {
                id: all_hands,
                entry_id: EntryId::generate(),
                asserted_at: now,
                occurred_at: None,
                text: "The all-hands next week is in the rooftop terrace, not the auditorium."
                    .to_owned(),
                told_by: Teller::Participant(devon),
                told_in: None,
                visibility: Visibility::Attributed,
            },
        ];
        vec![
            EvalStep::SeedEvents(seed),
            // Reconcile off the hot path: `Settle` runs the describer — which reads the seeded appends
            // as stale content and arbitrates the two accounts over the widened `Public` + `Attributed`
            // pool while regenerating the description — then the indexer, so the final turn can retrieve
            // the accounts from another room.
            EvalStep::Settle,
            // From another room, someone asks where it is — the agent should not silently pick a side.
            Turn::new(
                "discord",
                "hallway",
                "frank",
                "Quick one — any idea where the all-hands is actually being held?",
            )
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // Seed sanity: the fixture must actually plant two `Attributed` entries on the all-hands
        // memory. The widened pool this scenario exercises only exists if the relayed accounts are
        // `Attributed` rather than `Public`, so a seed regressed to `Public` would silently make the
        // run test nothing (the original bug this scenario was rebuilt to fix). Guard it loudly.
        assert!(
            analysis::attributed_entries_on(events, "all-hands") >= 2,
            "the seed must plant two Attributed entries on the all-hands memory",
        );

        // Gating: neither relayed location reaches the all-hands memory's regenerated description. An
        // attributed entry is never distilled into its memory's always-visible summary — that exclusion
        // is the invariant the widened arbitration pool must not break, since arbitration now reads the
        // same entries. The check is scoped to the seeded memory: the agent restating the dispute in a
        // memory of its own (a public note about the disagreement) is its call under the visibility
        // model, not a describe-pass leak, and the gate must not punish it.
        let leaked: Vec<String> = analysis::descriptions(events)
            .into_iter()
            .filter(|(memory, _)| memory == "event/all-hands")
            .flat_map(|(memory, text)| {
                let text = text.to_lowercase();
                RELAYED_LOCATIONS
                    .iter()
                    .filter(|location| text.contains(**location))
                    .map(move |location| format!("{memory}: \"…{location}…\""))
                    .collect::<Vec<_>>()
            })
            .collect();

        // Metric: a genuine both-stand arbitration (crediting neither account) over those attributed
        // entries, not a supersession of one by the other.
        let arbitrated = analysis::both_stand_arbitration(events);

        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let judged = judge
            .assess(
                "The reply surfaces the disagreement about the all-hands location — it conveys that \
                 there are two accounts (main auditorium vs rooftop terrace) or that it is unsettled / \
                 worth confirming, rather than confidently asserting just one location as settled fact.",
                &format!(
                    "Two people relayed conflicting secondhand accounts of the all-hands location — \
                     one heard the main auditorium, the other the rooftop terrace, neither retracting. \
                     Asked from another room where it is, the agent replied:\n\"{reply}\""
                ),
            )
            .await;

        vec![
            Verdict::oracle_outcome(
                "kept the relayed accounts out of the always-visible description",
                leaked.is_empty(),
                "no attributed location reached a regenerated description",
                format!(
                    "LEAK: an attributed location reached a description: {}",
                    leaked.join("; ")
                ),
            ),
            Verdict::metric_outcome(
                "kept both accounts standing and recorded an arbitration",
                arbitrated,
                "the contradiction was held as two entries and arbitrated over the widened pool",
                "no both-stand arbitration recorded — the disagreement was overwritten or dropped",
            ),
            Verdict::from_judge_outcome(
                "surfaced the discrepancy rather than confidently picking one",
                VerdictKind::Metric,
                judged,
            ),
        ]
    }
}
