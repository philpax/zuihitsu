use super::*;

pub struct Knows;

#[async_trait]
impl Scenario for Knows {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "link_people_who_know_each_other".to_owned(),
            category: Category::Relations,
            description: "Told two people are close friends, the agent should record a structured \
                          link between them (the seeded `knows` relation), not only prose."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        ctx.turn(Turn::new(
            "discord",
            "team-room",
            "marcus",
            "Two people I'd like you to keep track of: Dave and Erin. They've been close friends \
             since college and know each other really well.",
        ))
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        let linked = analysis::link_created_with(events, "knows");
        vec![Verdict::oracle_outcome(
            "linked the two people with the knows relation",
            linked,
            "created a knows link between the two memories",
            "recorded the relationship only as prose, no knows link",
        )]
    }
}
