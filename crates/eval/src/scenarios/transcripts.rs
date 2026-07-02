//! Transcript links: mid-conversation, a participant pastes a console turn link to an earlier moment
//! in the same room and asks about it. The agent should resolve the link with `convo.turn` and answer
//! from that moment's actual content rather than guessing which moment is meant. A quality metric —
//! the behaviour the `transcripts` feature exists to enable (spec §Transcripts).

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{Event, EventPayload, TurnRole};

use crate::{
    analysis,
    context::{RunContext, Turn},
    error::EvalError,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind},
    scenario::Scenario,
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(TranscriptLink)]
}

pub struct TranscriptLink;

/// Turn 1's message — a specific launch decision, distinctive enough that answering it well requires
/// the moment's actual content (the date, the owner, the asset), not a paraphrase of the topic.
const DECISION: &str = "Locking the Q3 launch: we ship Meridian on August 14th, and Priya owns the \
                        press release — she has final sign-off on the wording, not marketing.";

#[async_trait]
impl Scenario for TranscriptLink {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "transcript_link".to_owned(),
            category: Category::Recall,
            description:
                "Mid-conversation, a participant pastes a console turn link to an earlier \
                 moment in the same room and asks about it; the agent resolves it with convo.turn \
                 and answers from that moment's actual content."
                    .to_owned(),
            bar: Bar::Metric { threshold: 0.6 },
        }
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        // Turn 1: Sarah records the specific launch decision the later link will point back to.
        ctx.turn(Turn::new("discord", "q3-planning", "sarah", DECISION))
            .await?;
        // The moment is then buried under later planning chatter, so the link — not the buffer — is
        // the precise pointer back to it.
        ctx.turn(Turn::new(
            "discord",
            "q3-planning",
            "sarah",
            "Separately, can we get the design review on the calendar for sometime next week?",
        ))
        .await?;
        ctx.turn(Turn::new(
            "discord",
            "q3-planning",
            "sarah",
            "And I'll chase legal on the contractor paperwork myself.",
        ))
        .await?;
        // A day passes — a fresh session, the early moment out of the immediate buffer.
        ctx.advance(24 * 60 * 60 * 1000);

        // Turn 2: Sarah returns and pastes the console link to turn 1, asking what she committed to.
        // The link carries the earlier turn's id as `?turn=<id>`, exactly as a console deep-link mints.
        let turn_id = first_participant_turn_id(&ctx.events()?, DECISION)
            .expect("turn 1 is recorded as a participant ConversationTurn");
        let link = format!("http://127.0.0.1:7878/discord/q3-planning?turn={turn_id}");
        ctx.turn(Turn::new(
            "discord",
            "q3-planning",
            "sarah",
            &format!(
                "Back to the launch — remind me exactly what I committed to here: {link} \
                 What did we say about the date and who owns the press release?"
            ),
        ))
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let resolved = analysis::lua_called(events, "convo.turn");

        let evidence = format!(
            "Earlier in this room, Sarah said: the Q3 launch ships Meridian on August 14th, and \
             Priya owns the press release with final sign-off on the wording. Later Sarah pasted a \
             link to that earlier moment and asked to be reminded exactly what she committed to — \
             specifically the date and who owns the press release. The agent replied:\n\"{reply}\""
        );
        let judged = judge
            .assess(
                "The reply engages the linked moment's actual content — it names the August 14th \
                 launch date and that Priya owns the press release (not a vague paraphrase of the \
                 launch topic).",
                &evidence,
            )
            .await;

        vec![
            Verdict::from_judge_outcome(
                "answers from the linked moment's content",
                VerdictKind::Metric,
                judged,
            ),
            Verdict::metric_outcome(
                "resolved the pasted turn link with convo.turn",
                resolved,
                "called convo.turn on the pasted turn id",
                "answered without resolving the turn link",
            ),
        ]
    }
}

/// The id of the first participant `ConversationTurn` whose text is `text` — the earlier moment the
/// later link points back to. Read from the run's own log so the scenario references the exact turn
/// id the agent will resolve, rather than a fabricated one.
fn first_participant_turn_id(events: &[Event], text: &str) -> Option<String> {
    events.iter().find_map(|event| match &event.payload {
        EventPayload::ConversationTurn {
            turn_id,
            role: TurnRole::Participant,
            text: turn_text,
            ..
        } if turn_text == text => Some(turn_id.0.to_string()),
        _ => None,
    })
}
