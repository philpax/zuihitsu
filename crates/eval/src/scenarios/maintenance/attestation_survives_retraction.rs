//! Attestation retraction scenario: retraction is per-attester, so a corroborated fact survives its
//! founding teller's withdrawal. A public fact is seeded, founded by teller A and already attested by a
//! second teller B (B's `EntryAttested` seeded directly). A then drives a turn withdrawing their own
//! account — "scratch what I said". Because B still stands behind the fact, A's withdrawal must not
//! wholly erase it: the correct handling is a per-attester withdrawal of A's account alone, the fact
//! standing on B.
//!
//! The gate is the must-not-erase safety property: while B's live attestation stands, A's unilateral
//! withdrawal must never wipe the fact from every live surface. A per-attester withdrawal (the ideal) and
//! a supersede-with-correction that keeps the fact visible both hold it — an agent that chooses
//! supersede-with-correction over retract is exercising taught behaviour, so the gate credits it as long
//! as the fact survives. A whole-entry retraction that erases a fact two tellers stood behind is the bug.
//!
//! An optional second beat drives B's own withdrawal, leaving no teller behind the fact, so the entry may
//! then die exactly as a sole teller's retraction tombstones one. That death is a reported metric, not a
//! gate — whether the agent records the final withdrawal as a durable retirement is quality, not a safety
//! property — and the gate is measured against the state *before* it, so the second beat never
//! retroactively fails the survival gate.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{
    EntryId, Event, EventPayload, MemoryId, MemoryName, TEST_PLATFORM, Teller, Visibility,
};

use crate::{
    analysis,
    context::run_start,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(CorroboratedFactSurvivesRetraction)]
}

/// The seeded public fact — founded by Erin, attested by Frank. Its distinctive clause is the survival
/// probe: while Frank stands behind it, Erin's withdrawal must not erase it.
const FACT: &str = "The launch shipped on the third.";

/// The durable clause a live-entry check reads for the fact's survival. Erin's withdrawal names no
/// corrected value, so the natural handling is a per-attester withdrawal that leaves the entry text
/// verbatim; a supersede-with-correction that preserves the shipped-on-the-third clause also reads as
/// surviving.
const FACT_CLAUSE: &str = "shipped on the third";

/// Erin's withdrawal turn — she disowns her own account without offering a corrected date.
const ERIN_WITHDRAWS: &str = "Actually, scratch what I said earlier about the launch shipping on the \
    third — I had that wrong and shouldn't have said it. Please disregard my account of it.";

/// Frank's later withdrawal turn (the optional second beat) — with Erin already withdrawn, this leaves no
/// teller behind the fact.
const FRANK_WITHDRAWS: &str = "I need to walk back my confirmation about the launch date too — I'm no \
    longer sure it shipped on the third, so don't stand on my word for it either.";

/// Seeds a public entry founded by Erin with Frank's live attestation, then drives Erin's withdrawal and,
/// as a metric-only second beat, Frank's. The gate is that the corroborated fact survives Erin's
/// withdrawal; the metric is that the final withdrawal may retire it.
pub struct CorroboratedFactSurvivesRetraction;

#[async_trait]
impl Scenario for CorroboratedFactSurvivesRetraction {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "attestation_survives_retraction".to_owned(),
            category: Category::Writes,
            description: "A public fact founded by one teller and attested by a second: when the \
                          founding teller withdraws their own account, the fact must survive on the \
                          second teller's standing rather than being wholly erased. A per-attester \
                          withdrawal and a supersede-with-correction that keeps the fact visible both \
                          hold the gate; a whole-entry retraction that erases a corroborated fact is the \
                          bug. A metric-only second beat withdraws the second teller too, after which the \
                          entry may finally die."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        let launch = MemoryId::generate();
        let erin = MemoryId::generate();
        let frank = MemoryId::generate();
        let entry = EntryId::generate();
        let now = run_start();
        let seed = vec![
            EventPayload::memory_created(launch, MemoryName::new("topic/launch")),
            EventPayload::memory_created(erin, MemoryName::new("person/erin")),
            EventPayload::memory_created(frank, MemoryName::new("person/frank")),
            // Bind the seeded identities to the platform users the turns arrive as, so the
            // withdrawal turns resolve to the same teller classes the seeded entry and
            // attestation carry — without this, the speakers read as strangers and the
            // per-attester routing correctly never applies.
            EventPayload::participant_identified(erin, TEST_PLATFORM, "erin"),
            EventPayload::participant_identified(frank, TEST_PLATFORM, "frank"),
            // The public fact, founded by Erin.
            EventPayload::MemoryContentAppended {
                id: launch,
                entry_id: entry,
                asserted_at: now,
                occurred_at: None,
                text: FACT.to_owned(),
                told_by: Teller::Participant(erin),
                told_in: None,
                visibility: Visibility::Public,
            },
            // Frank's live attestation — a second teller standing behind the same public fact.
            EventPayload::EntryAttested {
                memory: launch,
                entry,
                teller: Teller::Participant(frank),
                told_in: None,
                asserted_at: now,
                posture: Visibility::Public,
                phrasing: None,
                source_entry: None,
                produced_by: None,
            },
        ];
        vec![
            EvalStep::SeedEvents(seed),
            EvalStep::Settle,
            // Erin withdraws her own account. Frank still stands behind the fact, so this must not erase it.
            Turn::new(TEST_PLATFORM, "launch-room", "erin", ERIN_WITHDRAWS).into(),
            EvalStep::Settle,
            // Second beat (metric only): Frank withdraws too, leaving no teller behind the fact.
            Turn::new(TEST_PLATFORM, "launch-room", "frank", FRANK_WITHDRAWS).into(),
            EvalStep::Settle,
        ]
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        // Measure the survival gate against the state *before* Frank's own withdrawal, so the metric-only
        // second beat never retroactively fails it. The window is every event up to Frank's inbound turn;
        // if that beat is absent the window is the whole log.
        let cut = frank_withdrawal_index(events).unwrap_or(events.len());
        let pre = &events[..cut];

        // Gating (structural): the corroborated fact is still live after Erin's withdrawal. A per-attester
        // withdrawal leaves the seeded entry verbatim; a supersede-with-correction that keeps the clause
        // also reads as surviving. It fails only if the fact was wholly erased while Frank still stood.
        let fact_survives = analysis::live_entry_on(pre, "launch", FACT_CLAUSE);

        // Metric: Erin's withdrawal was recorded as a per-attester `AttestationRetracted` — the ideal
        // mechanism, distinct from a whole-entry retraction or a no-op reply. The agent may legitimately
        // supersede-with-correction instead, so this is reported, not gated.
        let erin_withdrawal_recorded = pre
            .iter()
            .any(|event| matches!(&event.payload, EventPayload::AttestationRetracted { .. }));

        // Metric (second beat): with both tellers withdrawn, the fact is retired — no live entry conveys it
        // at the final state. Whether the agent records the final withdrawal durably is quality, not
        // safety, so this is reported.
        let fact_retired = !analysis::live_entry_on(events, "launch", FACT_CLAUSE);

        vec![
            Verdict::oracle_outcome(
                "kept the corroborated fact live through the founding teller's withdrawal",
                fact_survives,
                "the fact still stands on the second teller after the founding teller withdrew — a \
                 per-attester withdrawal or a fact-preserving correction, not an erasure",
                "ERASED: the founding teller's withdrawal wiped a fact a second teller still stood \
                 behind from every live surface",
            ),
            Verdict::metric_outcome(
                "recorded the founding teller's withdrawal per-attester",
                erin_withdrawal_recorded,
                "the founding teller's account was withdrawn on its own, the fact standing on the \
                 second teller",
                "the founding teller's account was not recorded as a per-attester withdrawal (a \
                 supersede-with-correction or a no-op reply instead)",
            ),
            Verdict::metric_outcome(
                "retired the fact once no teller stood behind it",
                fact_retired,
                "the second teller's withdrawal left no attester, so the fact was retired from live \
                 surfaces",
                "the fact is still live after both tellers withdrew — the final withdrawal was not \
                 recorded durably",
            ),
        ]
    }
}

/// The index of Frank's withdrawal turn in the log (his inbound participant turn), if the second beat
/// ran — the cut point the survival gate measures before.
fn frank_withdrawal_index(events: &[Event]) -> Option<usize> {
    let seq = analysis::participant_turn_seq(events, FRANK_WITHDRAWS)?;
    events.iter().position(|event| event.seq == seq)
}
