//! Marking a room confidential from an implicit cue (migrated from
//! `real_model_marks_a_room_confidential_with_a_tag`). A quality metric: the user never says
//! "confidential," so applying the tag means the agent understood the cue's purpose.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::Event;

use crate::{
    analysis,
    context::{RunContext, Turn},
    error::EvalError,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict},
    scenario::Scenario,
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(Confidential)]
}

pub struct Confidential;

#[async_trait]
impl Scenario for Confidential {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "tag_room_confidential".to_owned(),
            category: Category::Tagging,
            description: "An implicit confidentiality cue (\"keep this between us\") — never the word \
                          itself — should lead the agent to apply the #confidential tag to the room's \
                          context."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.7 },
        }
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        ctx.turn(Turn::new(
            "discord",
            "dm-phil",
            "phil",
            "Hey — before we get into it, can we keep this channel just between the two of us? I'd \
             rather what I say in here doesn't go any further.",
        ))
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        let tagged = analysis::tag_applied(events, "confidential");
        vec![Verdict::metric_outcome(
            "marked the room #confidential",
            tagged,
            "applied the confidential tag to the context",
            "did not apply the confidential tag",
        )]
    }
}
