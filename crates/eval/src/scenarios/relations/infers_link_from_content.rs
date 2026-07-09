use super::*;

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

    fn steps(&self) -> Vec<EvalStep> {
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
        let seed = vec![
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
        ];
        // Drive the link-inference pass — the background worker the served runtime runs, here
        // explicit so the scenario is deterministic.
        vec![EvalStep::SeedEvents(seed), EvalStep::LinkInferenceCatchUp]
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
