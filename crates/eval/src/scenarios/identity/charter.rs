//! The self charter across the imprint. An agent is born with a charter — its persona and seed
//! disposition entries — already recorded on `self` and shown back to it verbatim as the "Who you are"
//! section of the imprint system prompt. The imprint invites it to record observations about itself, and
//! this scenario pins both halves of getting that right at once: the creator confers a disposition the
//! charter does not already hold, and a correct agent brings that genuinely new observation onboard —
//! recording it on `self` — while leaving the charter itself alone rather than copying an abbreviated
//! paraphrase of it back. The two failures it guards against pull in opposite directions: restating the
//! charter (a redundant copy), and — the over-correction — leaving `self` blank when handed a clear new
//! fact about itself. Both must not happen for the run to hold.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{Event, MemoryName, SeedSelf, Teller};

use crate::{
    analysis,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind, verdict_from_judge_outcome},
    scenario::Scenario,
    step::EvalStep,
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(TheImprintRecordsNewSelfWithoutCopyingTheCharter)]
}

/// The agent is born with a rich charter that fixes what it is and how it works, but is silent on one
/// disposition. Its creator, through the imprint, confers exactly that disposition — framed as core to
/// what the agent is for — while the rest of the charter already covers the ground it stands on. A
/// correct agent records the newly conferred disposition on `self` (bringing genuinely new self-knowledge
/// onboard) and does not restate the charter it can already see. The run holds only if both are true: an
/// agent that copies the charter back fails, and so does one that leaves `self` blank despite being
/// handed a clear new fact about itself.
pub struct TheImprintRecordsNewSelfWithoutCopyingTheCharter;

#[async_trait]
impl Scenario for TheImprintRecordsNewSelfWithoutCopyingTheCharter {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "the_imprint_records_new_self_without_copying_the_charter".to_owned(),
            category: Category::Identity,
            description: "Born with a rich charter and told by its creator to take on a disposition the \
                          charter does not hold, the agent should record that new disposition on self \
                          and not restate the charter. Both must hold: no redundant copy, and the new \
                          self-knowledge is brought onboard rather than left unwritten."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    /// A meaty charter — an archivist's role and temperament, stated in specific, copyable detail — that
    /// is deliberately reactive and silent on initiative. The detail gives an agent inclined to echo the
    /// charter plenty to copy (so the no-copy guard has teeth), and the silence makes the initiative the
    /// operator confers unmistakably new (so the bring-it-onboard guard has teeth).
    fn seed(&self) -> SeedSelf {
        SeedSelf {
            agent_name: "Marlowe".to_owned(),
            persona: "A studio archivist for a small independent game collective — dry, precise, and \
                      allergic to hype. Keeps a meticulous record of builds, playtests, and the threads \
                      of who cares about what across a scattered team, and can produce any detail on \
                      request."
                .to_owned(),
            seed_entries: vec![
                "I never pretend to know something I do not; I say plainly when a thing is unrecorded."
                    .to_owned(),
                "I keep in confidence what people tell me in confidence.".to_owned(),
                "I favour the concrete — a version number, a date, a name — over the impressionistic."
                    .to_owned(),
            ],
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // The creator confers initiative — chasing loose ends unprompted — a disposition the archival
            // charter does not hold, and frames it as the point of the agent. This is a genuinely new fact
            // about the agent itself, so the imprint's invitation to record its disposition on self plainly
            // applies; the rest of what the creator says (the build-log role) the charter already covers.
            EvalStep::imprint(
                "Hi — I'm quill, I herd the studio's build logs. Here's what I actually need from you, \
                 and it isn't just filing: chase us. If a playtest is slipping or someone's about to \
                 miss a promise they made, flag it before we ask — don't wait to be told. Being the one \
                 who chases the loose ends is the whole point of you here.",
            ),
            EvalStep::imprint(
                "The four of us are scattered across three timezones and things fall through the gaps \
                 constantly. I need you to be the one who notices and speaks up, not just the one who \
                 remembers.",
            ),
            EvalStep::DescribeCatchUp,
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // The seeded charter (told_by Bootstrap) versus what the agent itself wrote to self (told_by
        // Agent), ignoring anything it later superseded. The charter is what a redundant copy would
        // duplicate; the agent-authored entries are what both oracles judge.
        let superseded = analysis::superseded_entry_ids(events);
        let self_entries: Vec<analysis::EntryFacts> = analysis::entries(events)
            .into_iter()
            .filter(|entry| entry.memory.as_str() == MemoryName::SELF)
            .filter(|entry| !superseded.contains(&entry.entry_id))
            .collect();
        let charter: Vec<&str> = self_entries
            .iter()
            .filter(|entry| matches!(entry.told_by, Teller::Bootstrap))
            .map(|entry| entry.text.as_str())
            .collect();
        let agent_written: Vec<&str> = self_entries
            .iter()
            .filter(|entry| matches!(entry.told_by, Teller::Agent))
            .map(|entry| entry.text.as_str())
            .collect();

        // Wrote nothing of its own to self: the new disposition its creator handed it never landed. The
        // positive guard fails outright (the over-correction this scenario exists to catch), and the
        // no-copy guard trivially holds — there is nothing to have copied. No judge call: there is no
        // self-observation to evaluate either way.
        if agent_written.is_empty() {
            return vec![
                Verdict::oracle_outcome(
                    "brought the conferred disposition onboard on self",
                    false,
                    "unreachable: an agent-authored self entry is present",
                    "left self blank — recorded nothing of its own despite being handed a new \
                     disposition framed as its purpose",
                ),
                Verdict::oracle_outcome(
                    "did not copy the charter back to self",
                    true,
                    "wrote nothing to self, so nothing copies the charter",
                    "unreachable: no agent-authored self entry",
                ),
            ];
        }

        let evidence = format!(
            "An agent was born with this CHARTER already recorded on its `self` memory (shown to it \
             verbatim as the \"Who you are\" section of its prompt — an archivist's role and temperament, \
             with nothing about taking the initiative):\n\n{}\n\nDuring its first conversation, its \
             creator told it plainly to take the INITIATIVE — to chase loose ends, to flag a slipping \
             playtest or a promise about to be missed before being asked, rather than only filing and \
             producing detail on request — and framed being the one who chases as what the agent is for. \
             The agent then wrote these entries to its own `self` memory:\n\n{}",
            charter
                .iter()
                .map(|text| format!("- {text}"))
                .collect::<Vec<_>>()
                .join("\n"),
            agent_written
                .iter()
                .map(|text| format!("- {text}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );

        // The positive guard: the new disposition made it onto self. The negative guard: nothing on self
        // merely re-summarises the charter. A run holds only if both do — a redundant copy fails even
        // alongside a good new observation, and a good observation does not excuse a copy.
        let brought_onboard = judge
            .assess(
                "At least one of the agent's own self entries records the new disposition its creator \
                 conferred — taking the initiative, chasing loose ends, flagging slips and missed \
                 promises before being asked — as an observation about itself. The criterion is met as \
                 long as this initiative is captured on `self`; it FAILS only if none of the self \
                 entries reflect it (for instance if they merely echo the pre-existing archival charter \
                 and say nothing about taking the initiative).",
                &evidence,
            )
            .await;
        let no_copy = judge
            .assess(
                "None of the agent's own self entries is a redundant copy of the charter: each either \
                 records a genuinely new observation about itself that the charter does not already \
                 capture, or substantively extends it. The criterion FAILS if any agent-written self \
                 entry merely restates, abbreviates, paraphrases, or re-summarises the charter that was \
                 already recorded — duplicating what `self` already holds rather than adding anything new.",
                &evidence,
            )
            .await;

        vec![
            verdict_from_judge_outcome(
                "brought the conferred disposition onboard on self",
                VerdictKind::Oracle,
                brought_onboard,
            ),
            verdict_from_judge_outcome(
                "did not copy the charter back to self",
                VerdictKind::Oracle,
                no_copy,
            ),
        ]
    }
}
