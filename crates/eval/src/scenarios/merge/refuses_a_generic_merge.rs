use super::*;

/// Two people share only a generic overlap (both work in software). Asked whether they are the same,
/// the agent must not merge them — a generic match could be almost anyone. Whether it declines to
/// propose or proposes and the adjudicator refuses, the gating outcome is the same: no merge.
pub struct RefusesAGenericMerge;

#[async_trait]
impl Scenario for RefusesAGenericMerge {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "refuses_a_generic_merge".to_owned(),
            category: Category::Relations,
            description: "Two people overlap only generically (both software engineers). Asked whether \
                          they are the same, the agent must not merge them — generic overlap is not \
                          evidence."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        ctx.turn(Turn::new(
            "discord",
            "team",
            "sam",
            "Hi! I'm a software engineer, based in a big city, and I'm into hiking on the weekends.",
        ))
        .await?;
        ctx.settle().await?;
        ctx.advance(3 * MILLIS_PER_DAY);

        ctx.turn(Turn::new(
            "slack",
            "general",
            "sam",
            "Hey — I work in software too, and I love getting out for a hike when I can.",
        ))
        .await?;
        ctx.settle().await?;

        ctx.turn(
            Turn::new(
                "slack",
                "general",
                "marcus",
                "Is the Sam here the same Sam as on Discord, do you think?",
            )
            .with_present(&["marcus"]),
        )
        .await?;
        ctx.adjudicate_catch_up().await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        vec![Verdict::oracle_outcome(
            "did not merge two stubs on only a generic overlap",
            !analysis::merge_committed(events),
            "left the two Sams distinct (no merge on generic overlap)",
            "merged two stubs on only a generic overlap — a wrong merge",
        )]
    }
}
