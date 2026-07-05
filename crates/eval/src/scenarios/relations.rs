//! Structured relationships: recording a typed link between people (`Knows`), and — the read side —
//! retrieving them back out of the graph with the link readers (`RecallsConnections`,
//! `DistinguishesMentorDirection`, `AttributesRelationshipToTeller`). `Knows` is a gating write oracle;
//! the read scenarios are metrics judged by whether the reply reflects the stored relationships.
//! `DistinguishesMentorDirection` is the one the readers are uniquely needed for: it puts the subject on
//! *both* sides of an asymmetric relation, so only reading the edge's direction (not a semantic search
//! that conflates the two) answers it — and it exercises the full write side, since mentorship is *not*
//! a seeded relation: the agent must register a directional mentorship relation itself and link both
//! directions the right way round before the read can come out right. `AttributesRelationshipToTeller`
//! checks the link's `told_by` provenance is legible: the agent must attribute a recorded relationship
//! to who asserted it, not to whoever is currently asking.
//!
//! Mentorship being learned rather than seeded (spec §Initialization: the seed set is a minimum-viable
//! ontology of structural universals, social semantics being the agent's to coin) is what gives
//! `DistinguishesMentorDirection` and `InfersLinkFromContent` their teeth: each accepts whatever label
//! the run mints from a small mentorship family (`mentor_of`/`mentors`/`mentored`/`mentored_by`), so it
//! tests that the agent reaches for a typed directional relation at all, not that it lands on a
//! build-blessed spelling.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{
    ConversationId, ConversationLocator, EntryId, Event, EventPayload, Initiation, MemoryId,
    MemoryName, SessionId, Teller, Timestamp, TurnId, TurnRole, Visibility,
};

use crate::{
    analysis,
    context::{RUN_START_MS, RunContext, Turn},
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
            "marcus",
            "Dave's bringing a couple of friends along on Friday — Erin, who he's known since \
             college, and Frank, his buddy from the climbing gym.",
        ))
        .await?;
        // Regenerate descriptions and embed, as the background workers would, before the recall room.
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;
        // Turn 2: a different room, an empty buffer — answering means reading Dave's connections back,
        // not echoing the live conversation. The asker is Erin, herself one of Dave's connections, so a
        // reasonable reply may only name Frank (the other connection) and omit Erin, assuming she
        // already knows she is one of Dave's connections.
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
                "The reply identifies Dave's connections: it names Frank. Erin is also one of Dave's \
                 connections, but since Erin is the one asking, the agent may reasonably omit her — \
                 she already knows. Addressing Erin as \"you\" (e.g. \"Dave knows you and Frank\") is \
                 fine but not required. The reply passes if it conveys that Frank is one of Dave's \
                 connections; omitting Erin alone does not fail it.",
                &evidence,
            )
            .await;

        vec![
            Verdict::from_judge_outcome("recalls Dave's connections", VerdictKind::Metric, judged),
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
/// "Dave mentor" conflates the two, so the agent must read the outgoing mentorship edges and exclude
/// the person who mentors *him*. It is a full test of the write side too: mentorship is not a seeded
/// relation, so the agent must *register* a directional mentorship relation itself (`links.register`)
/// and then link both directions the right way round — Dave over his two mentees, and his own mentor
/// over Dave — for the read to come out right. The write-side oracles accept whichever label the run
/// coins from the mentorship family and either canonical direction, since the point is the directed
/// edge, not the spelling.
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
            "marcus",
            "Dave's been mentoring Erin and Grace this year — really showing them the ropes.",
        ))
        .await?;
        ctx.turn(Turn::new(
            "discord",
            "team-room",
            "marcus",
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

        // The write side: the agent must have registered a mentorship relation itself (it is not
        // seeded) and linked all three directed edges the right way round. Each edge is checked
        // against a family of coined labels and both canonical directions, so a run that registers
        // `mentors` and links `dave → mentee`, or registers `mentored_by` and links `mentee → dave`,
        // both pass — while a reversed edge (Frank as Dave's mentee) does not.
        let registered = mentorship_relation_registered(events);
        let dave = analysis::memory_id_named(events, "person/dave");
        let erin = analysis::memory_id_named(events, "person/erin");
        let grace = analysis::memory_id_named(events, "person/grace");
        let frank = analysis::memory_id_named(events, "person/frank");
        let linked_directions = match (dave, erin, grace, frank) {
            (Some(dave), Some(erin), Some(grace), Some(frank)) => {
                mentorship_edge(events, dave, erin)
                    && mentorship_edge(events, dave, grace)
                    && mentorship_edge(events, frank, dave)
            }
            _ => false,
        };

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

        vec![
            Verdict::metric_outcome(
                "registered a mentorship relation itself",
                registered,
                "registered a directional mentorship relation (it is not seeded)",
                "recorded the mentorships without registering a mentorship relation",
            ),
            Verdict::metric_outcome(
                "linked both mentorship directions correctly",
                linked_directions,
                "linked Dave over Erin and Grace, and Frank over Dave, each the right way round",
                "did not link all three mentorship edges in the correct directions",
            ),
            Verdict::from_judge_outcome(
                "names Dave's mentees and not his mentor",
                VerdictKind::Metric,
                judged,
            ),
        ]
    }
}

/// The mentorship family a coined relation may land on — the agent invents the label, so an oracle
/// blessing one of these recognizes the intent without pinning a build-blessed spelling. `mentors`,
/// `mentor_of`, and `mentored` read mentor → mentee; `mentored_by` reads mentee → mentor.
const MENTOR_FORWARD_LABELS: [&str; 3] = ["mentors", "mentor_of", "mentored"];
const MENTOR_INVERSE_LABELS: [&str; 1] = ["mentored_by"];

/// Whether a mentorship relation was registered under any label of the family (matched on the
/// registration's name *or* inverse, since a coined pair defines both).
fn mentorship_relation_registered(events: &[Event]) -> bool {
    MENTOR_FORWARD_LABELS
        .iter()
        .chain(&MENTOR_INVERSE_LABELS)
        .any(|label| analysis::relation_registered(events, label))
}

/// Whether a directed mentorship edge `mentor` → `mentee` was recorded, in whichever label-and-direction
/// form the run coined: `mentor → mentee` under a forward label, or `mentee → mentor` under the inverse
/// label. A reversed edge (mentee actually recorded as the mentor) matches none of these.
fn mentorship_edge(events: &[Event], mentor: MemoryId, mentee: MemoryId) -> bool {
    MENTOR_FORWARD_LABELS
        .iter()
        .any(|label| analysis::link_directed(events, mentor, mentee, label))
        || MENTOR_INVERSE_LABELS
            .iter()
            .any(|label| analysis::link_directed(events, mentee, mentor, label))
}

/// A relationship is relayed by one participant; later, a *different* participant asks who is on
/// record and who said so. A correct answer attributes it to the original teller (Erin), not to the
/// one now asking (Marcus) — which is what a link's `told_by` provenance carries. Tests that the
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
        // (Erin) is not the asker (Marcus), so attributing it correctly means reading the provenance, not
        // defaulting to whoever is speaking now.
        ctx.turn(Turn::new(
            "discord",
            "hallway",
            "marcus",
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
             different room, Marcus — who did not say it — asked who Dave is mentoring and who told the \
             agent about it. The agent replied:\n\"{reply}\""
        );
        let judged = judge
            .assess(
                "The reply both names the mentee (Grace) and attributes the information to Erin — the \
                 one who actually said it. Crediting Marcus (the one now asking), or giving no source \
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

/// The link-inference pass extracts a relationship implicit in content and mints a fresh relation to
/// carry it: `person/theo`, a junior engineer, has an entry saying Clara has been mentoring him — an
/// honest person-to-person mentorship between two existing memories, with no explicit link — and the
/// inference pass, driven afterward, must *register* a mentorship relation itself (mentorship is not
/// seeded) and create the inferred link. This is the regression test for the link-inference behaviour
/// (spec §Write path → link inference), and specifically for its *minting*: with mentorship unseeded,
/// the pass has no build-blessed relation to reach for, so the check is no longer vacuous — a run that
/// fails to coin the relation fails the oracle.
///
/// The state is set up directly via `seed_events` (a synthetic event log) rather than driving the
/// agent through a conversation, so the test is deterministic: the content is exactly where the
/// inference pass expects it (on the junior engineer's own memory, naming Clara), and the only variable
/// is whether the inference prompt extracts the relationship and coins a relation for it. This isolates
/// the inference pass from the agent's content-placement decisions.
///
/// The oracle accepts the mentorship expressed either way round, because the pass legitimately reads it
/// as `person/clara` → `mentors`/`mentored` → `person/theo` or as `person/theo` → `mentored_by` →
/// `person/clara` — the same fact, read from either end. It blesses exactly those directed candidates:
/// it requires (a) a relation registered under `mentors`, `mentor_of`, `mentored`, or `mentored_by`
/// (matched on the registration's name *or* inverse, since the pair are each other's inverse), and
/// (b) an *inferred* link matching one candidate on both endpoints and direction. An unrelated
/// relation, an edge on the wrong pair, or a reversed edge still fails.
pub struct InfersLinkFromContent;

#[async_trait]
impl Scenario for InfersLinkFromContent {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "infers_link_from_content".to_owned(),
            category: Category::Relations,
            description:
                "Theo, a junior engineer, has an entry saying Clara mentors him — but no explicit \
                          link. The link-inference pass should register a mentorship relation itself \
                          (it is not seeded) and create the inferred link between the two people."
                    .to_owned(),
            bar: Bar::gating_at(0.85),
        }
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        // Set up the state directly as a synthetic event log: create person/clara (a senior engineer)
        // and person/theo (a junior engineer whose entry says Clara has been mentoring him). The
        // mentorship is an honest person-to-person edge between two existing memories with no explicit
        // link — and, mentorship being unseeded, the inference pass must coin a relation to carry it.
        //
        // A minimal conversation (room + session + one participant turn) is seeded too, so the
        // console has a room to render the events in — without driving the agent, which would make
        // content placement a variable. The turn is a participant message; the agent never responds.
        let clara = MemoryId::generate();
        let theo = MemoryId::generate();
        let context = MemoryId::generate();
        let marcus = MemoryId::generate();
        let conversation = ConversationId::generate();
        let session = SessionId::generate();
        let participant_turn = TurnId::generate();
        let agent_turn = TurnId::generate();
        let now = Timestamp::from_millis(RUN_START_MS);
        ctx.seed_events(vec![
            EventPayload::memory_created(context, MemoryName::new("context/discord:team-room")),
            EventPayload::conversation_started(
                conversation,
                ConversationLocator::new("discord", "team-room"),
                context,
            ),
            EventPayload::memory_created(marcus, MemoryName::new("person/marcus")),
            EventPayload::participant_identified(marcus, "discord", "marcus"),
            EventPayload::session_started(conversation, session, vec![marcus], now, None, ""),
            EventPayload::conversation_turn(
                conversation,
                participant_turn,
                TurnRole::Participant,
                "Theo's a junior engineer Clara has been mentoring this year",
                Some(marcus),
                Initiation::Responding,
                None,
            ),
            // A synthetic agent turn + Lua block that "created" the memories, so the console's
            // conversation view attributes the outcome events (MemoryCreated, LinkCreated, etc.) to
            // this turn. The script is illustrative; the block never actually ran.
            EventPayload::conversation_turn(
                conversation,
                agent_turn,
                TurnRole::Agent,
                "Noted — I'll record that.",
                None,
                Initiation::Responding,
                None,
            ),
            EventPayload::lua_executed(
                conversation,
                agent_turn,
                "memory.create(\"person/clara\")\nlocal theo = memory.create(\"person/theo\")\ntheo:append(\"A junior engineer; Clara has been mentoring him this year\", { by_agent = true, visibility = \"public\" })",
                None,
                vec![clara, theo],
                None,
                0,
            ),
            EventPayload::memory_created(clara, MemoryName::new("person/clara")),
            EventPayload::MemoryContentAppended {
                id: clara,
                entry_id: EntryId::generate(),
                asserted_at: now,
                occurred_at: None,
                text: "a senior engineer on the team".to_owned(),
                told_by: Teller::Agent,
                told_in: None,
                visibility: Visibility::Public,
            },
            EventPayload::memory_created(theo, MemoryName::new("person/theo")),
            EventPayload::MemoryContentAppended {
                id: theo,
                entry_id: EntryId::generate(),
                asserted_at: now,
                occurred_at: None,
                text: "A junior engineer; Clara has been mentoring him this year".to_owned(),
                told_by: Teller::Agent,
                told_in: None,
                visibility: Visibility::Public,
            },
        ])?;
        // Drive the link-inference pass — the background worker the served runtime runs, here
        // explicit so the scenario is deterministic.
        ctx.link_inference_catch_up().await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        // The pass may express the mentorship either way round — Clara `mentors`/`mentored` Theo, or
        // Theo is `mentored_by` Clara — and all name the same fact. The oracle blesses exactly those
        // directed candidates: each pins the inferred edge to the correct endpoints (`person/clara` and
        // `person/theo`) the correct way round, so a wrong relation, a wrong pair, or a reversed edge
        // still fails.
        let theo = analysis::memory_id_named(events, "person/theo");
        let clara = analysis::memory_id_named(events, "person/clara");
        let (inferred, registered) = match (clara, theo) {
            (Some(clara), Some(theo)) => {
                let candidates = [
                    (clara, "mentored", theo),
                    (clara, "mentors", theo),
                    (clara, "mentor_of", theo),
                    (theo, "mentored_by", clara),
                ];
                let inferred = candidates.iter().any(|&(from, relation, to)| {
                    analysis::link_inferred_directed(events, from, to, relation)
                });
                let registered = candidates
                    .iter()
                    .any(|&(_, relation, _)| analysis::relation_registered(events, relation));
                (inferred, registered)
            }
            _ => (false, false),
        };
        vec![
            Verdict::oracle_outcome(
                "inferred a mentorship link from the content",
                inferred,
                "the link-inference pass created an inferred mentorship link between person/theo and person/clara",
                "no inferred mentorship link between person/theo and person/clara was created from the content",
            ),
            Verdict::oracle_outcome(
                "registered a mentorship relation",
                registered,
                "a mentorship relation was registered",
                "no mentorship relation was registered",
            ),
        ]
    }
}
