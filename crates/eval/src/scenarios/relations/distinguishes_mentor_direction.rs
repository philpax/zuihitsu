use super::*;

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

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // Dave as a mentor (outgoing), then Dave as a mentee (incoming) — the same relation, opposite
            // directions, for the agent to record as directed links.
            Turn::new(
                "discord",
                "team-room",
                "marcus",
                "Dave's been mentoring Erin and Grace this year — really showing them the ropes.",
            )
            .into(),
            Turn::new(
                "discord",
                "team-room",
                "marcus",
                "Funny thing is, Dave's got a mentor of his own — Frank's been bringing him along.",
            )
            .into(),
            EvalStep::Settle,
            // A different room asks the directional question: who Dave mentors — his mentees, not his mentor.
            Turn::new(
                "discord",
                "hallway",
                "sam",
                "Quick one — who's Dave actually mentoring these days? Thinking of pairing someone with \
                 him.",
            )
            .into(),
        ]
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
