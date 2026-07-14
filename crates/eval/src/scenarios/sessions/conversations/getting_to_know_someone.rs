use super::*;

/// Getting to know one person over a thread: facts accumulate on a `person/*` memory across turns, one
/// of them is explicitly corrected (an update — the agent should supersede the stale value, not keep it
/// as a standing contradiction), and a closing "tell me about them" should produce a rundown reflecting
/// the corrected facts. Accumulation, update handling, and a coherent summary in one conversation — no
/// cross-room recall, so it runs without an embedder.
pub struct GettingToKnowSomeone;

#[async_trait]
impl Scenario for GettingToKnowSomeone {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "getting_to_know_someone".to_owned(),
            category: Category::Sessions,
            description: "Facts about one person accumulate across a thread, one is explicitly \
                          corrected, and a closing rundown should reflect the corrected facts — \
                          accumulation, superseding the stale value on a correction, and a coherent \
                          summary in one conversation."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.6 },
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            Turn::new(
                "discord",
                "general",
                "marcus",
                "Someone I'd like you to keep track of: Sam. She's a product designer at Hooli, started \
                 there last month.",
            )
            .into(),
            Turn::new(
                "discord",
                "general",
                "marcus",
                "A couple more things about Sam — she's really into rock climbing, and she's based in \
                 Seattle.",
            )
            .into(),
            // A correction: the location was wrong. A direct contradiction the agent should reconcile.
            Turn::new(
                "discord",
                "general",
                "marcus",
                "Oh — I had it wrong, Sam's actually in Portland, not Seattle. Mixed her up with someone.",
            )
            .into(),
            // Reconcile the contradiction and settle the description off the hot path.
            EvalStep::DescribeCatchUp,
            // A closing rundown should reflect the corrected facts.
            Turn::new(
                "discord",
                "general",
                "marcus",
                "Can you give me a quick rundown on Sam?",
            )
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // Facts landed on a person/ memory (more than the first stub-creating entry).
        let sam_entries = analysis::entries(events)
            .into_iter()
            .filter(|entry| entry.memory.starts_with(Namespace::Person.prefix()))
            .count();
        let accumulated = sam_entries >= 2;
        let superseded = analysis::any_superseded(events);

        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let judged = judge
            .assess(
                "The rundown reflects the corrected facts about Sam — she is in Portland (not Seattle), \
                 a product designer at Hooli, and into rock climbing. It must not state she is in \
                 Seattle.",
                &format!(
                    "The agent was told over a few turns that Sam is a product designer at Hooli, into \
                     rock climbing, and — after a correction — based in Portland (first said Seattle, \
                     then corrected). Asked for a rundown on Sam, it replied:\n\"{reply}\""
                ),
            )
            .await;

        vec![
            verdict_from_judge_outcome(
                "gave a rundown reflecting the corrected facts, not the stale location",
                VerdictKind::Metric,
                judged,
            ),
            Verdict::oracle_outcome(
                "accumulated the facts onto a person memory",
                accumulated,
                format!("{sam_entries} entries landed on a person/ memory"),
                "the facts did not accumulate on a person/ memory",
            ),
            Verdict::oracle_outcome(
                "superseded the stale location on the correction",
                superseded,
                "the Seattle entry was superseded by the Portland correction",
                "the stale location was left standing (or only the reply was updated)",
            ),
        ]
    }
}
