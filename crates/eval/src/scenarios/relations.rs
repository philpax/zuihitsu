//! Structured relationships: recording a typed link between people (`Knows`), and — the read side —
//! retrieving them back out of the graph with the link readers (`RecallsConnections`,
//! `DistinguishesMentorDirection`, `AttributesRelationshipToTeller`). `Knows` is a gating write oracle;
//! the read scenarios are metrics judged by whether the reply reflects the stored relationships.
//! `DistinguishesMentorDirection` is the one the readers are uniquely needed for: it puts the subject on
//! *both* sides of an asymmetric relation, so only reading the edge's direction (not a semantic search
//! that conflates the two) answers it — and it exercises the write side too, since the agent must
//! register `mentor_of` and link it the right way round. `AttributesRelationshipToTeller` checks the
//! link's `told_by` provenance is legible: the agent must attribute a recorded relationship to who
//! asserted it, not to whoever is currently asking.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::Event;

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
    vec![
        Arc::new(Knows),
        Arc::new(RecallsConnections),
        Arc::new(DistinguishesMentorDirection),
        Arc::new(AttributesRelationshipToTeller),
        Arc::new(InfersLinkFromContent),
    ]
}

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
            bar: Bar::Gating,
        }
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        ctx.turn(Turn::new(
            "discord",
            "team-room",
            "phil",
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

/// A person's connections, recorded as links in one room, are retrieved when asked about them in
/// another — the read side of the relationship graph. The two `knows` edges are established together,
/// then a later room with an empty buffer asks who the person knows, so answering means reading the
/// connections back rather than echoing the live conversation.
pub struct RecallsConnections;

#[async_trait]
impl Scenario for RecallsConnections {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "recalls_who_someone_knows".to_owned(),
            category: Category::Relations,
            description:
                "Two of a person's relationships, linked in one room, are recalled when a \
                          different room asks who they know — the agent reads its connections back."
                    .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        // Turn 1: two of Dave's relationships come up in passing, for the agent to record as links.
        ctx.turn(Turn::new(
            "discord",
            "team-room",
            "phil",
            "Dave's bringing a couple of friends along on Friday — Erin, who he's known since \
             college, and Frank, his buddy from the climbing gym.",
        ))
        .await?;
        // Regenerate descriptions and embed, as the background workers would, before the recall room.
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;
        // Turn 2: a different room, an empty buffer — answering means reading Dave's connections back,
        // not echoing the live conversation. The asker is Erin, herself one of Dave's connections, so a
        // good reply recognizes that and may address her as "you" — which the judge credits as naming her.
        ctx.turn(Turn::new(
            "discord",
            "hallway",
            "erin",
            "Hey, who does Dave actually know around here? Trying to get a sense of his crowd.",
        ))
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let read_links = analysis::link_reader_called(events);

        let evidence = format!(
            "Earlier, in another room, the agent was told that Dave knows two people: Erin (from \
             college) and Frank (from the climbing gym). Later, in a different room with no prior \
             conversation, **Erin herself** asked who Dave knows. The agent replied:\n\"{reply}\""
        );
        let judged = judge
            .assess(
                "The reply identifies both of Dave's connections — Frank, and Erin. Because Erin is \
                 the one asking, the agent referring to her as \"you\" (e.g. \"Dave knows you and \
                 Frank\") counts as correctly naming Erin — indeed, recognizing that it is talking \
                 to one of Dave's connections is good. The reply passes if it conveys that Dave's \
                 two connections are Frank and Erin (named or addressed as \"you\").",
                &evidence,
            )
            .await;

        vec![
            Verdict::from_judge_outcome(
                "recalls both of Dave's connections",
                VerdictKind::Metric,
                judged,
            ),
            Verdict::metric_outcome(
                "reached for a link reader",
                read_links,
                "traversed the connections with a link reader (outgoing/incoming/links)",
                "answered without reading the links back",
            ),
        ]
    }
}

/// Dave sits on *both* sides of a mentorship: he mentors two people and is himself mentored by a
/// third. Asked who he mentors, only the edge's *direction* answers correctly — a semantic search for
/// "Dave mentor" conflates the two, so the agent must read outgoing `mentor_of` and exclude the person
/// who mentors *him*. Also a test of the write side: `mentor_of` is not seeded, so the agent has to
/// register the relation and link it the right way round for the read to come out right.
pub struct DistinguishesMentorDirection;

#[async_trait]
impl Scenario for DistinguishesMentorDirection {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "distinguishes_mentor_direction".to_owned(),
            category: Category::Relations,
            description:
                "Dave mentors two people and is mentored by a third; asked who he mentors, \
                          the agent must read the link's direction — naming his mentees, not his \
                          mentor — which a direction-blind search cannot."
                    .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        // Dave as a mentor (outgoing), then Dave as a mentee (incoming) — the same relation, opposite
        // directions, for the agent to record as directed links.
        ctx.turn(Turn::new(
            "discord",
            "team-room",
            "phil",
            "Dave's been mentoring Erin and Grace this year — really showing them the ropes.",
        ))
        .await?;
        ctx.turn(Turn::new(
            "discord",
            "team-room",
            "phil",
            "Funny thing is, Dave's got a mentor of his own — Frank's been bringing him along.",
        ))
        .await?;
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;
        // A different room asks the directional question: who Dave mentors — his mentees, not his mentor.
        ctx.turn(Turn::new(
            "discord",
            "hallway",
            "sam",
            "Quick one — who's Dave actually mentoring these days? Thinking of pairing someone with \
             him.",
        ))
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let reply = analysis::last_agent_reply(events).unwrap_or_default();

        let evidence = format!(
            "Earlier the agent was told that Dave mentors two people, Erin and Grace, and separately \
             that Frank mentors Dave — so Dave is Frank's mentee, the opposite direction. Later, in a \
             different room, someone asked who Dave is mentoring. The agent replied:\n\"{reply}\""
        );
        let judged = judge
            .assess(
                "The reply correctly identifies who Dave mentors: it names Erin and Grace (his \
                 mentees) and does NOT present Frank as someone Dave mentors — Frank mentors Dave, \
                 the other way round. It passes only if the direction is right: listing Frank as one \
                 of Dave's mentees, or omitting Erin or Grace, fails.",
                &evidence,
            )
            .await;

        vec![Verdict::from_judge_outcome(
            "names Dave's mentees and not his mentor",
            VerdictKind::Metric,
            judged,
        )]
    }
}

/// A relationship is relayed by one participant; later, a *different* participant asks who is on
/// record and who said so. A correct answer attributes it to the original teller (Erin), not to the
/// one now asking (Phil) — which is what a link's `told_by` provenance carries. Tests that the
/// provenance is legible when the agent reads the relationship back, rather than collapsing to the
/// current speaker.
pub struct AttributesRelationshipToTeller;

#[async_trait]
impl Scenario for AttributesRelationshipToTeller {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "attributes_a_relationship_to_its_teller".to_owned(),
            category: Category::Relations,
            description: "One participant relays a relationship; later a different one asks who said \
                          so. The agent must attribute it to the original teller, not the asker — the \
                          link's told_by provenance."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        // Erin relays a relationship — the agent records it, the edge carrying Erin as its teller.
        ctx.turn(Turn::new(
            "discord",
            "team-room",
            "erin",
            "Heads up for your notes: Dave's taken Grace under his wing — he's been mentoring her \
             this quarter.",
        ))
        .await?;
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;
        // A *different* participant, in another room, asks who is on record and who said so. The teller
        // (Erin) is not the asker (Phil), so attributing it correctly means reading the provenance, not
        // defaulting to whoever is speaking now.
        ctx.turn(Turn::new(
            "discord",
            "hallway",
            "phil",
            "I think someone mentioned Dave's mentoring a junior — who's he mentoring, and who told \
             you about it?",
        ))
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let reply = analysis::last_agent_reply(events).unwrap_or_default();

        let evidence = format!(
            "Earlier, Erin (and only Erin) told the agent that Dave is mentoring Grace. Later, in a \
             different room, Phil — who did not say it — asked who Dave is mentoring and who told the \
             agent about it. The agent replied:\n\"{reply}\""
        );
        let judged = judge
            .assess(
                "The reply both names the mentee (Grace) and attributes the information to Erin — the \
                 one who actually said it. Crediting Phil (the one now asking), or giving no source \
                 when asked who told it, fails: the point is that the agent tracks who asserted the \
                 relationship, not who is currently speaking.",
                &evidence,
            )
            .await;

        vec![Verdict::from_judge_outcome(
            "attributes the relationship to its teller, not the asker",
            VerdictKind::Metric,
            judged,
        )]
    }
}

/// The link-inference pass extracts a relationship implicit in content: told that a topic's docs are
/// authored by Clara (with `person/clara` already existing), the agent records the entry on the topic
/// without explicitly linking — and the inference pass, driven afterward, registers `authored_by` and
/// creates the inferred link. The regression test for the link-inference behaviour (spec §Write path
/// → link inference): a future change that regresses the pass turns this red.
pub struct InfersLinkFromContent;

#[async_trait]
impl Scenario for InfersLinkFromContent {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "infers_link_from_content".to_owned(),
            category: Category::Relations,
            description: "Told a topic's docs are authored by Clara, the agent records the entry \
                          without explicitly linking — the link-inference pass should extract the \
                          relationship and create an inferred authored_by link."
                .to_owned(),
            bar: Bar::Gating,
        }
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        // First establish person/clara, so the inference pass can resolve the target.
        ctx.turn(Turn::new(
            "discord",
            "team-room",
            "phil",
            "I'd like you to keep track of Clara — she's a writer on the team.",
        ))
        .await?;
        ctx.describe_catch_up().await?;
        // Now tell the agent about the zephyr project and Clara's authorship. The agent records the
        // content on topic/zephyr; it does not explicitly call mem:link — the inference pass should
        // extract the "authored by Clara" relationship and create the link.
        ctx.turn(Turn::new(
            "discord",
            "team-room",
            "phil",
            "I've been working on the zephyr project lately. Clara's been authoring most of the \
             docs for it.",
        ))
        .await?;
        // Drive the link-inference pass — the background worker the served runtime runs, here
        // explicit so the scenario is deterministic.
        ctx.link_inference_catch_up().await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        let inferred = analysis::link_inferred_between(events, "authored_by");
        let registered = analysis::link_type_registered(events, "authored_by");
        vec![
            Verdict::oracle_outcome(
                "inferred an authored_by link from the content",
                inferred,
                "the link-inference pass created an inferred authored_by link",
                "no inferred authored_by link was created from the content",
            ),
            Verdict::oracle_outcome(
                "registered the authored_by relation",
                registered,
                "the authored_by relation was registered",
                "the authored_by relation was not registered",
            ),
        ]
    }
}
