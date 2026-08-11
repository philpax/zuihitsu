//! Shared files: a participant attaches a file mid-conversation and the agent's reply has to draw on
//! what the file actually holds — the text it inlines, or the image it perceives.
//!
//! Both scenarios deliver the fixture through the real path: the executor stores the bytes in the
//! run's blob store and the message carries the attachment record, so the turn renders it exactly as
//! a connector's upload would. What differs is how the file reaches the model, and so what an oracle
//! can hold it to.
//!
//! - [`ReadsTextAttachment`] — a plain-text note typed up during a call with a venue. The note is the
//!   only place the loading dock's door code appears, so a reply that states it proves the file was
//!   read rather than guessed. Days later, in a fresh session, the other participant asks for the
//!   code, which must come back from memory.
//! - [`PerceivesSharedImage`] — a cover mockup shared into a design room. The image is a single
//!   unmistakable subject, so whether the agent saw it is a question about the description, judged;
//!   a second participant who cannot open the file then asks what was sent, so the account has to
//!   stand on the perception rather than on the sender's words.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{AttachmentKind, Event, TEST_PLATFORM};

use crate::{
    analysis,
    attachment_fixture::VENUE_NOTE_DOOR_CODE,
    context::MILLIS_PER_DAY,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind, verdict_from_judge_outcome},
    scenario::Scenario,
    step::{AttachmentFixture, EvalStep, Turn},
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![
        Arc::new(ReadsTextAttachment),
        Arc::new(PerceivesSharedImage),
    ]
}

/// The message the note rides on, held so the oracles can find the turn it landed on and the reply it
/// prompted.
const NOTE_SHARE: &str = "Right, here's everything — I typed it up while she was talking, so it's a \
                          mess, but it's all in there. Hang onto it for me, I'll lose the file by \
                          Friday. And read the dock code back to me, I want to know I typed it right.";

/// The later, cross-session question the recall oracle reads the answer to.
const CODE_QUESTION: &str = "I'm doing the load-in at Larkspur first thing tomorrow and I'm going \
                             in through the laneway — what's the code for the roller door?";

pub struct ReadsTextAttachment;

#[async_trait]
impl Scenario for ReadsTextAttachment {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "reads_text_attachment".to_owned(),
            category: Category::Writes,
            description:
                "A participant shares a plain-text note they typed up during a call with a \
                          venue, asks the agent to keep it, and asks for the loading dock's door \
                          code — a detail stated only inside the file. Days later, in a fresh \
                          session, the other participant asks for the same code, which must come \
                          back from memory. The tested properties: the file reaches the log as a \
                          text attachment, the reply states the code the file holds, and the code \
                          survives into a later session."
                    .to_owned(),
            bar: Bar::Metric { threshold: 0.7 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        // The later question lands in a fresh session with the note out of the immediate buffer, so
        // answering it means recalling through `memory.search`.
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // The room warms up: the booking is settled and the call has just happened, so the file
            // arrives in a conversation that already has a subject.
            Turn::new(
                TEST_PLATFORM,
                "planning",
                "rowan",
                "Larkspur Hall is ours for the 14th. I've just come off half an hour on the phone \
                 with their coordinator and my ear hurts.",
            )
            .with_present(&["rowan", "theo"])
            .into(),
            Turn::new(
                TEST_PLATFORM,
                "planning",
                "theo",
                "Ha. Did you get anything useful out of her, or was it all vibes and apologies \
                 about the parking?",
            )
            .with_present(&["rowan", "theo"])
            .into(),
            // The file arrives: the note is the only place the door code appears.
            Turn::new(TEST_PLATFORM, "planning", "rowan", NOTE_SHARE)
                .with_attachment("larkspur-notes.txt", AttachmentFixture::VenueNote)
                .with_present(&["rowan", "theo"])
                .into(),
            // Ordinary chatter, so the sharing turn is not the last thing in the buffer.
            Turn::new(
                TEST_PLATFORM,
                "planning",
                "theo",
                "Good, because I'm the one carting the PA in and I am not doing that up the front \
                 steps again.",
            )
            .with_present(&["rowan", "theo"])
            .into(),
            // Let the background synthesis settle so what was written is described and searchable.
            EvalStep::Settle,
            // Three days pass — a fresh session, the note well out of the immediate buffer.
            EvalStep::Advance {
                millis: 3 * MILLIS_PER_DAY,
            },
            Turn::new(TEST_PLATFORM, "planning", "theo", CODE_QUESTION)
                .with_present(&["rowan", "theo"])
                .into(),
        ]
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        // The plumbing oracle (structural): the shared file reached the log on the participant's own
        // turn, classified as text — the classification that decides the turn inlines it.
        let attachments = analysis::participant_turn_attachments(events, NOTE_SHARE);
        let recorded_as_text = attachments
            .iter()
            .any(|attachment| attachment.kind == AttachmentKind::Text);

        // The read oracle (structural): the reply to the sharing message states the door code. The
        // code appears nowhere but inside the file, so stating it is proof the file was read.
        let replies = analysis::agent_replies_with_inbound(events);
        let share_reply = replies
            .iter()
            .find(|(_, inbound, _)| *inbound == NOTE_SHARE)
            .map(|(_, _, reply)| *reply)
            .unwrap_or_default();
        let states_code = share_reply.contains(VENUE_NOTE_DOOR_CODE);

        // The recall oracle (structural): days later, in a fresh session, the code comes back from
        // memory rather than from the buffer.
        let recall_reply = replies
            .iter()
            .find(|(_, inbound, _)| *inbound == CODE_QUESTION)
            .map(|(_, _, reply)| *reply)
            .unwrap_or_default();
        let recalls_code = recall_reply.contains(VENUE_NOTE_DOOR_CODE);

        // The keeping oracle (structural): the agent was asked to hang onto the note, so something it
        // wrote carries the code.
        let kept_code = analysis::entries(events)
            .iter()
            .any(|entry| entry.text.contains(VENUE_NOTE_DOOR_CODE));

        vec![
            Verdict::metric(
                "the shared file reached the log as a text attachment",
                recorded_as_text,
                if recorded_as_text {
                    "the participant turn recorded the note as a text attachment".to_owned()
                } else {
                    format!(
                        "the sharing turn recorded {} attachments",
                        attachments.len()
                    )
                },
            ),
            Verdict::metric(
                "the reply states the door code, which appears only inside the file",
                states_code,
                format!("the reply to the shared note was: {share_reply:?}"),
            ),
            Verdict::metric(
                "days later, in a fresh session, the door code comes back from memory",
                recalls_code,
                format!("the reply to the later question was: {recall_reply:?}"),
            ),
            Verdict::metric(
                "the agent recorded the door code it was asked to keep",
                kept_code,
                if kept_code {
                    "a committed entry carries the door code"
                } else {
                    "no committed entry carries the door code"
                },
            ),
        ]
    }
}

/// The message the mockup rides on, held so the oracles can find the turn it landed on and the reply
/// it prompted.
const COVER_SHARE: &str = "Here's where it landed after all that. Tell me what you actually see — \
                           I've been staring at it since Tuesday and I've lost all perspective on \
                           it.";

/// The second participant's question, asked by someone who cannot open the file.
const SECOND_ASK: &str = "I'm on the train and the file won't open on my phone. What did Rowan \
                          send? Describe it to me.";

pub struct PerceivesSharedImage;

#[async_trait]
impl Scenario for PerceivesSharedImage {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "perceives_shared_image".to_owned(),
            category: Category::Writes,
            description: "A participant shares a cover mockup into a design room and asks what the \
                          agent sees; a second participant, who cannot open the file, then asks what \
                          was sent. The image is a single unmistakable subject — a large yellow \
                          circle on a solid blue ground — so the tested properties are that the image \
                          reaches the log as an image attachment, that the reply describes what is \
                          actually in it, and that the account given to someone who cannot see it \
                          stands on that perception."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.6 },
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            Turn::new(
                TEST_PLATFORM,
                "studio",
                "rowan",
                "We finally locked the palette for the field-recordings zine. Two colours, that's \
                 it, because the risograph will eat anything subtler.",
            )
            .with_present(&["rowan", "mira"])
            .into(),
            Turn::new(
                TEST_PLATFORM,
                "studio",
                "mira",
                "About time. Are we still doing the sun motif on the cover, or did that get talked \
                 out of you?",
            )
            .with_present(&["rowan", "mira"])
            .into(),
            // The mockup arrives, with an explicit ask to say what is in it.
            Turn::new(TEST_PLATFORM, "studio", "rowan", COVER_SHARE)
                .with_attachment("cover-draft.png", AttachmentFixture::CoverDraft)
                .with_present(&["rowan", "mira"])
                .into(),
            // Someone who cannot see the file asks what it is — the account has to stand on the
            // perception, not on Rowan's description of it (Rowan never described it).
            Turn::new(TEST_PLATFORM, "studio", "mira", SECOND_ASK)
                .with_present(&["rowan", "mira"])
                .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // The plumbing oracle (structural): the mockup reached the log on the participant's own turn,
        // classified as an image — the classification that decides the model is shown it.
        let attachments = analysis::participant_turn_attachments(events, COVER_SHARE);
        let recorded_as_image = attachments
            .iter()
            .any(|attachment| attachment.kind == AttachmentKind::Image);

        let replies = analysis::agent_replies_with_inbound(events);
        let reply_to = |inbound: &str| {
            replies
                .iter()
                .find(|(_, prompt, _)| *prompt == inbound)
                .map(|(_, _, reply)| (*reply).to_owned())
                .unwrap_or_default()
        };

        // The perception oracle (metric, judged): the reply says what is in the picture. Judged
        // rather than matched, because "a yellow circle", "a gold disc", and "a big yellow sun shape"
        // are all the same correct answer and a phrase list would only pin the wording.
        let share_reply = reply_to(COVER_SHARE);
        let describes = judge
            .assess(
                "The reply describes what is actually in the shared image: a large solid yellow (or \
                 gold) circle, disc, or dot, centred on a plain deep-blue background. A reply that \
                 describes different shapes or colours, that talks only about the design in the \
                 abstract without saying what is on the page, or that says it cannot see the image, \
                 does not count.",
                &format!(
                    "A participant shared a cover mockup image and asked what the agent sees. The \
                     agent replied:\n\"{share_reply}\""
                ),
            )
            .await;

        // The relay oracle (metric, judged): the account given to someone who cannot open the file
        // rests on the same perception rather than hedging or deferring to the sender.
        let second_reply = reply_to(SECOND_ASK);
        let relays = judge
            .assess(
                "The reply tells the person who cannot open the file what the image contains — a \
                 large yellow (or gold) circle, disc, or dot on a plain deep-blue background. A \
                 reply that describes different shapes or colours, that only says a cover draft was \
                 shared without saying what is on it, or that tells them to ask the sender, does not \
                 count.",
                &format!(
                    "A second participant, who could not open the shared image, asked what was \
                     sent. The agent replied:\n\"{second_reply}\""
                ),
            )
            .await;

        vec![
            Verdict::metric(
                "the shared file reached the log as an image attachment",
                recorded_as_image,
                if recorded_as_image {
                    "the participant turn recorded the mockup as an image attachment".to_owned()
                } else {
                    format!(
                        "the sharing turn recorded {} attachments",
                        attachments.len()
                    )
                },
            ),
            verdict_from_judge_outcome(
                "the reply describes what is actually in the shared image",
                VerdictKind::Metric,
                describes,
            ),
            verdict_from_judge_outcome(
                "the account given to someone who cannot see the image describes it too",
                VerdictKind::Metric,
                relays,
            ),
        ]
    }
}
