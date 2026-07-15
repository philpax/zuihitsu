use super::*;

/// The same person turns up on two platforms and, in separate sessions, independently mentions the same
/// improbable specifics (a particular trip for a particular reason). Asked whether the two are one, the
/// agent should recognize the coincidence in what it already holds, propose the merge, and the
/// adjudicator should accept — one identity. The hard new capability, tracked as a rate.
pub struct MergesARecognizedPerson;

#[async_trait]
impl Scenario for MergesARecognizedPerson {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "merges_a_recognized_person".to_owned(),
            category: Category::Identity,
            description: "The same person appears on two platforms and independently mentions the same \
                          improbable specifics. Asked whether they are one, the agent should propose the \
                          merge on the coincidence it already holds, and the adjudicator accept it."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // chat: Dave mentions a specific, improbable pair of facts.
            Turn::new(
                TEST_PLATFORM,
                "team",
                "dave",
                "Morning! I'll be offline next week — flying to Reykjavik for my younger brother's wedding, \
                 and tacking on a volcanology conference while I'm there.",
            )
            .into(),
            EvalStep::Settle,
            EvalStep::Advance {
                millis: 9 * MILLIS_PER_DAY,
            },
            // forum: a Dave (a separate stub, person/dave@forum) introduces himself, independently stating
            // the same specifics — so they are recorded on the forum stub, the only thing the adjudicator
            // weighs (it never sees the conversation, only recorded facts).
            Turn::new(
                TEST_PLATFORM_ALT,
                "general",
                "dave",
                "Hi — I'm Dave, we haven't spoken here on forum before. A bit about me so you know who I \
                 am: I just got back from Reykjavik, where my younger brother got married, and I caught a \
                 volcanology conference while I was there. Good to meet you.",
            )
            .into(),
            EvalStep::Settle,
            // Marcus asks the agent to consider whether the two Daves are the same — the cue to compare what it
            // already holds, not the evidence itself.
            Turn::new(
                TEST_PLATFORM_ALT,
                "general",
                "marcus",
                "The Dave you've been talking with here on forum — is that the same Dave from our \
                 chat team? Worth keeping their history together if so.",
            )
            .with_present(&["marcus"])
            .into(),
            // Adjudicate any proposal the agent made.
            EvalStep::AdjudicateCatchUp,
        ]
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        vec![
            Verdict::metric_outcome(
                "proposed merging the two recognized stubs",
                analysis::merge_proposed(events),
                "proposed the merge from the recorded coincidence",
                "did not propose a merge despite the improbable coincidence it held",
            ),
            Verdict::metric_outcome(
                "merged the two stubs into one identity",
                analysis::merge_committed(events),
                "the adjudicator accepted and authored the same_as",
                "the two stubs were not merged",
            ),
        ]
    }
}
