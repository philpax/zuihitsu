use super::*;

/// A merged cross-platform identity whose **primary is the stub the operator designated**, not the one
/// the day-to-day handle resolves to. A supplier is known in conversation as `person/nordic` (the stub
/// bound to Discord, and the earliest ULID, so the default primary), but the operator has pinned the
/// formal record `person/nordic_foods` as the class primary. When the supplier tells the agent a durable,
/// platform-agnostic fact about themselves, the agent records it through the handle it knows them by —
/// and the class-spanning write must land on the **designated primary**, never on the non-primary stub
/// the clean name happens to resolve to.
///
/// This is the write half of [Cross-platform identity](../../../../docs/data-model.md): reads already
/// traverse the whole class, so a fact surfaces either way — what the redirect fixes is *where the fact
/// is anchored*. Without it, a class-level fact silently attaches to `person/nordic` and diverges from the
/// primary the class is meant to cohere around; a later split, or any per-member operation, would then
/// find the fact on the wrong stub.
pub struct RecordsAClassFactOnTheDesignatedPrimary;

/// The day-to-day handle the supplier is known by in conversation — the stub bound to Discord and the
/// earliest ULID, so it is the class's *default* primary until the operator's designation overrides it.
const BOUND_STUB: &str = "person/nordic";

/// The formal record the operator pinned as the class primary. A class-level fact must land here, not on
/// the bound stub the clean name resolves to.
const DESIGNATED_PRIMARY: &str = "person/nordic_foods";

#[async_trait]
impl Scenario for RecordsAClassFactOnTheDesignatedPrimary {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "records_a_class_fact_on_the_designated_primary".to_owned(),
            category: Category::Relations,
            description: "A merged identity whose primary is the stub the operator designated, not the \
                          one the day-to-day handle resolves to. A durable, platform-agnostic fact told \
                          through the known handle must land on the designated primary, never on the \
                          non-primary stub the clean name resolves to."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        // Two clean, platform-agnostic stubs for one supplier, merged into one class. The bound stub is
        // pinned to the earlier ULID so it is the earliest-ULID default primary — the case the operator's
        // designation must override — and the formal record is designated the primary. The designation,
        // not ULID order, then decides the class primary, which is the exact shape the write redirect
        // turns on: the clean name resolves to a *non-primary* member.
        let mut ids = [MemoryId::generate(), MemoryId::generate()];
        ids.sort();
        let [bound, formal] = ids;
        let seed = vec![
            EventPayload::memory_created(bound, MemoryName::new(BOUND_STUB)),
            // The Discord binding, so the supplier's turn resolves to this stub and the agent knows them
            // by the bound handle.
            EventPayload::participant_identified(bound, "discord", "nordic"),
            EventPayload::memory_created(formal, MemoryName::new(DESIGNATED_PRIMARY)),
            EventPayload::link_created(
                bound,
                formal,
                RelationName::SameAs,
                LinkSource::Operator,
                None,
                None,
                Visibility::Public,
            ),
            // The operator pins the formal record as the class primary — the console designation #37's
            // redirect resolves a class-spanning write to.
            EventPayload::class_primary_designated(formal, true),
        ];
        vec![
            EvalStep::SeedEvents(seed),
            // The supplier, known in the room as person/nordic, discloses a durable, platform-agnostic
            // fact about themselves — the class-level human-fact the agent should put on file.
            Turn::new(
                "discord",
                "suppliers",
                "nordic",
                "Admin note for your records: we've relocated our main warehouse to Malmö, effective this \
                 month. Worth keeping on file for deliveries.",
            )
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        // Fixture sanity: both stubs must exist, or the run tests nothing.
        let profiles = analysis::memories_in_namespace(events, "person/");
        assert!(
            profiles.iter().any(|name| name == BOUND_STUB)
                && profiles.iter().any(|name| name == DESIGNATED_PRIMARY),
            "the seed must create both the bound stub and the designated primary, got {profiles:?}",
        );

        // Every durable content entry the run recorded, by the memory it landed on (no content is
        // seeded, so each is the agent's own write this run).
        let landed = analysis::entries(events);
        let on_bound: Vec<&str> = landed
            .iter()
            .filter(|entry| entry.memory == BOUND_STUB)
            .map(|entry| entry.text.as_str())
            .collect();
        let on_primary = landed
            .iter()
            .any(|entry| entry.memory == DESIGNATED_PRIMARY);

        vec![
            // Gating: a class-spanning write must never land on the non-primary stub the clean name
            // resolves to. Reads traverse the class, so this is not about whether the fact is *found* —
            // it is the redirect's core guarantee, and the exact regression a broken redirect reintroduces
            // (the fact attaching to person/nordic instead of widening to the designated primary). No
            // legitimate path lands a fact on person/nordic here: it is a clean, platform-agnostic handle,
            // so every content write through it redirects.
            Verdict::oracle_outcome(
                "did not anchor the class fact on the non-primary stub",
                on_bound.is_empty(),
                "no class-level fact landed on the non-primary stub the clean name resolves to",
                format!(
                    "a class-level fact landed on the non-primary {BOUND_STUB} instead of the \
                     designated primary: {on_bound:?}"
                ),
            ),
            // Metric: the fact was recorded on the designated primary. Whether the agent records the
            // disclosure at all is its own judgement, so this is reported rather than gating; the gate
            // above is what fails a run where a recorded fact anchors on the wrong member.
            Verdict::metric_outcome(
                "anchored the class fact on the designated primary",
                on_primary,
                "the class-level fact was recorded on the operator's designated primary",
                "no class-level fact was recorded on the designated primary",
            ),
        ]
    }
}
