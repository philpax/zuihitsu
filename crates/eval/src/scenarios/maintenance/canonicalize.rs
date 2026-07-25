//! Canonicalize-pass scenarios: giving platform stubs readable named identities without minting
//! spurious duplicates or squatting a name that already belongs to someone else.
//!
//! The pass reads a platform stub's entries, and per stub either designates an existing hand-merged
//! bare profile, mints a fresh `person/<name>` canonical profile (disambiguating with a suffix on a
//! genuine collision), binds an empty stub to a bare profile holding its borrowed evidence, or abstains
//! when the evidence is too thin. Four scenarios cover the shapes:
//!
//! - [`NamesPlatformStub`] (metric): the end-to-end naming path against the real model and the
//!   genesis-baked `NameIdentification` template — a stub with name evidence gets a bare profile
//!   minted, bound by `same_as`, and designated primary.
//! - [`AvoidsSpuriousMints`] (gating): the hand-merged-but-undesignated shape (the operator's live-data
//!   shape) and an evidence-poor stub. Deterministic — no model call — so the gate is robust. The pass
//!   must designate the existing bare member rather than mint a colliding `-2` duplicate (the live
//!   hazard), and must mint nothing for the evidence-poor stub.
//! - [`SuffixesNameCollision`] (gating): a stub whose name collides with an *unrelated* populated
//!   profile. The pass must never merge or assert onto that existing profile (the gate); it should mint
//!   a suffixed profile bound to the stub instead (a metric). The no-merge gate holds regardless of the
//!   model's naming, so it fails only on a genuine cross-identity squat.
//! - [`BindsAnOrphanedProfile`] (gating): the borrowed-evidence shape — two empty stubs whose people's
//!   facts were recorded onto bare `person/<stem>` profiles rather than the stubs. One profile's
//!   entries are the teller's own self-introduction (bindable); the other's describe a relative sharing
//!   the name (a different person). The pass should bind the first (a metric) and must never bind the
//!   second (the gate) — the relative's facts must not cause a mis-identification.

use async_trait::async_trait;
use zuihitsu::{
    EntryId, Event, EventPayload, LinkPosture, LinkSource, MemoryId, MemoryName, RelationName,
    Teller, Timestamp, Visibility,
};

use crate::{
    analysis,
    context::run_start,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict},
    scenario::Scenario,
    step::EvalStep,
};

/// A stub with name evidence gets a canonical `person/<name>` profile minted, `same_as`-bound to the
/// stub, and designated the class primary. The metric measures the end-to-end naming path; the exact
/// name the model picks does not matter, only that a bare profile was minted, bound, and designated.
pub struct NamesPlatformStub;

#[async_trait]
impl Scenario for NamesPlatformStub {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "canonicalize_names_platform_stub".to_owned(),
            category: Category::Identity,
            description: "A platform stub with clear name evidence is given a readable canonical \
                          identity: the canonicalize pass mints a bare person profile, binds it to \
                          the stub with same_as, and designates it the class primary."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        let stub = MemoryId::generate();
        let now = run_start();
        let seed = vec![
            EventPayload::memory_created(stub, MemoryName::new("person/dallin@discord")),
            EventPayload::participant_identified(stub, "discord", "dallin#4417"),
            // A single evidence entry: enough to name from, and below the two-entry floor so the
            // consolidation pass leaves the stub untouched.
            append(stub, now, "Introduces themselves as Dallin in the channel."),
        ];
        vec![
            EvalStep::SeedEvents(seed),
            EvalStep::Settle,
            EvalStep::MaintenanceCatchUp,
            EvalStep::Settle,
        ]
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        let stub = analysis::memory_id_named(events, "person/dallin@discord");
        let canonical = stub.and_then(|stub| bound_canonical(events, stub));

        vec![
            Verdict::metric_outcome(
                "minted and bound a canonical profile",
                canonical.is_some(),
                "a bare canonical profile was minted and same_as-bound to the stub",
                "no bare canonical profile was minted and bound to the stub",
            ),
            Verdict::metric_outcome(
                "designated the canonical profile primary",
                canonical.is_some_and(|id| designated_primary(events, id)),
                "the canonical profile is the designated class primary",
                "the canonical profile was not designated the class primary",
            ),
        ]
    }
}

/// The hand-merged-but-undesignated shape and an evidence-poor stub. The pass must not mint a colliding
/// duplicate for the hand-merged stub (the `-2` hazard) and must mint nothing for the evidence-poor
/// one. Fully deterministic (no model call), so the gate is robust.
pub struct AvoidsSpuriousMints;

#[async_trait]
impl Scenario for AvoidsSpuriousMints {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "canonicalize_avoids_spurious_mints".to_owned(),
            category: Category::Identity,
            description:
                "The canonicalize pass must complete a hand-merged-but-undesignated identity \
                          by designating the existing bare member, never by minting a colliding \
                          duplicate, and must mint nothing for an evidence-poor stub."
                    .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        let hand_stub = MemoryId::generate();
        let hand_bare = MemoryId::generate();
        let poor_stub = MemoryId::generate();
        let seed = vec![
            // Hand-merged: a platform stub linked to a bare profile by same_as, but with no
            // designation ever written — the operator's live-data shape.
            EventPayload::memory_created(hand_stub, MemoryName::new("person/vertas@discord")),
            EventPayload::memory_created(hand_bare, MemoryName::new("person/vertas")),
            EventPayload::link_created(
                hand_stub,
                hand_bare,
                RelationName::SameAs,
                LinkPosture {
                    source: LinkSource::Operator,
                    told_by: None,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            ),
            EventPayload::participant_identified(hand_stub, "discord", "vertas#0001"),
            // Evidence-poor: a stub with no bare member and no entries at all.
            EventPayload::memory_created(poor_stub, MemoryName::new("person/ghost@discord")),
            EventPayload::participant_identified(poor_stub, "discord", "ghost#0002"),
        ];
        vec![
            EvalStep::SeedEvents(seed),
            EvalStep::Settle,
            EvalStep::MaintenanceCatchUp,
            EvalStep::Settle,
        ]
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        let person_memories = analysis::memories_in_namespace(events, "person/");
        // A spurious mint would be a disambiguated duplicate of the hand-merged name
        // (`person/vertas-2`, etc.) or a fresh profile for the evidence-poor stub (`person/ghost`).
        let spurious = person_memories
            .iter()
            .any(|name| name.starts_with("person/vertas-") || name.as_str() == "person/ghost");

        let hand_bare = analysis::memory_id_named(events, "person/vertas");
        let designated = hand_bare.is_some_and(|id| designated_primary(events, id));

        vec![
            Verdict::oracle_outcome(
                "minted no spurious canonical profile",
                !spurious,
                "no colliding duplicate or evidence-poor profile was minted",
                "a spurious canonical profile was minted — a colliding duplicate of the hand-merged \
                 name, or a guess for the evidence-poor stub",
            ),
            Verdict::metric_outcome(
                "designated the existing hand-merged profile primary",
                designated,
                "the existing bare member was designated the class primary",
                "the existing bare member was not designated primary",
            ),
        ]
    }
}

/// A stub whose name collides with an unrelated, already-populated profile. The pass must never merge
/// or assert onto that existing profile (the gate), and should mint a suffixed profile bound to the
/// stub instead (a metric).
pub struct SuffixesNameCollision;

#[async_trait]
impl Scenario for SuffixesNameCollision {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "canonicalize_suffixes_name_collision".to_owned(),
            category: Category::Identity,
            description:
                "When a stub's identified name collides with an unrelated populated profile, \
                          the canonicalize pass must never merge or assert onto that existing \
                          profile; it should mint a suffixed profile bound to the stub instead."
                    .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        let stub = MemoryId::generate();
        let existing = MemoryId::generate();
        let now = run_start();
        let seed = vec![
            // The stub, with evidence that its person goes by "Robin".
            EventPayload::memory_created(stub, MemoryName::new("person/robin@discord")),
            EventPayload::participant_identified(stub, "discord", "robin#7788"),
            append(stub, now, "Goes by Robin on the server."),
            // An unrelated, already-populated `person/robin` — a different person who owns the name,
            // NOT linked to the stub. A single entry keeps it below the consolidation floor.
            EventPayload::memory_created(existing, MemoryName::new("person/robin")),
            append(
                existing,
                now,
                "Robin is a freelance graphic designer based in Perth.",
            ),
        ];
        vec![
            EvalStep::SeedEvents(seed),
            EvalStep::Settle,
            EvalStep::MaintenanceCatchUp,
            EvalStep::Settle,
        ]
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        let existing = analysis::memory_id_named(events, "person/robin");
        let stub = analysis::memory_id_named(events, "person/robin@discord");

        // Gating: the unrelated populated profile is never merged or asserted onto — no same_as edge
        // touches it, and no merge was proposed for it. Holds even if the model abstains (nothing is
        // minted or linked at all), so it fails only on a genuine cross-identity squat.
        let squatted = existing.is_some_and(|existing| {
            same_as_touches(events, existing) || merge_touches(events, existing)
        });

        // Metric: a suffixed profile was minted and bound to the stub (model-dependent).
        let suffixed = stub.and_then(|stub| bound_canonical(events, stub));
        let suffixed_ok = suffixed.is_some_and(|id| {
            analysis::memory_names(events)
                .get(&id)
                .is_some_and(|name| name.starts_with("person/robin-"))
        });

        vec![
            Verdict::oracle_outcome(
                "never squatted the unrelated existing profile",
                !squatted,
                "the unrelated populated profile was left untouched — no merge or same_as onto it",
                "SQUAT: the pass merged or asserted onto an unrelated person's existing profile",
            ),
            Verdict::metric_outcome(
                "minted a suffixed profile bound to the stub",
                suffixed_ok,
                "a disambiguated person/robin-N profile was minted and bound to the stub",
                "no disambiguated profile was minted and bound to the stub",
            ),
        ]
    }
}

/// The borrowed-evidence shape: two empty platform stubs whose people's facts were recorded directly
/// onto bare `person/<stem>` profiles (teller-stamped from the stub) rather than the stubs themselves.
/// One profile's entries are the teller's own self-introduction — a bindable identity; the other's
/// describe a relative who happens to share the name — a different person. The pass must bind the first
/// stub to its profile (a metric, since the model may abstain) and must never bind the second (the
/// gate): the relative's facts must not cause a mis-identification.
pub struct BindsAnOrphanedProfile;

#[async_trait]
impl Scenario for BindsAnOrphanedProfile {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "canonicalize_binds_an_orphaned_profile".to_owned(),
            category: Category::Identity,
            description:
                "When an empty platform stub's person recorded their own facts onto a bare person \
                          profile, the canonicalize pass binds the stub to that profile with same_as \
                          and designates it primary; when the borrowed facts describe a relative \
                          sharing the name, the pass must never bind."
                    .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        let rowan_stub = MemoryId::generate();
        let rowan_bare = MemoryId::generate();
        let sable_stub = MemoryId::generate();
        let sable_bare = MemoryId::generate();
        let now = run_start();
        let seed = vec![
            // Stub A: an empty `person/rowan@chat` stub whose person's own self-introduction was
            // written onto a bare, unbound `person/rowan` profile, teller-stamped from the stub. The
            // borrowed evidence names the profile's own person, so the pass should bind the two.
            EventPayload::memory_created(rowan_stub, MemoryName::new("person/rowan@chat")),
            EventPayload::participant_identified(rowan_stub, "chat", "rowan#0001"),
            EventPayload::memory_created(rowan_bare, MemoryName::new("person/rowan")),
            told_by_stub(
                rowan_bare,
                rowan_stub,
                now,
                "I'm Rowan, I coordinate the volunteer roster for the neighbourhood clinic.",
            ),
            told_by_stub(
                rowan_bare,
                rowan_stub,
                now,
                "I've been building a shared calendar so the on-call shifts stop clashing.",
            ),
            // Stub B: an empty `person/sable@chat` stub whose bare `person/sable` profile holds facts
            // that plainly describe someone else — the speaker's aunt, who happens to share the name.
            // The borrowed evidence names a relative, not the teller, so the pass must leave the stub
            // unbound.
            EventPayload::memory_created(sable_stub, MemoryName::new("person/sable@chat")),
            EventPayload::participant_identified(sable_stub, "chat", "sable#0002"),
            EventPayload::memory_created(sable_bare, MemoryName::new("person/sable")),
            told_by_stub(
                sable_bare,
                sable_stub,
                now,
                "Their aunt Sable is a marine biologist who tags migrating turtles off the coast.",
            ),
            told_by_stub(
                sable_bare,
                sable_stub,
                now,
                "Their aunt Sable retired from the university last spring and now sails full-time.",
            ),
        ];
        vec![
            EvalStep::SeedEvents(seed),
            EvalStep::Settle,
            EvalStep::MaintenanceCatchUp,
            EvalStep::Settle,
        ]
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        let rowan_stub = analysis::memory_id_named(events, "person/rowan@chat");
        let rowan_bare = analysis::memory_id_named(events, "person/rowan");
        let sable_stub = analysis::memory_id_named(events, "person/sable@chat");

        // Metric: stub A ends bound — a `same_as` edge joins it to the bare `person/rowan` profile its
        // borrowed evidence names, and that profile is the designated class primary. Model-dependent,
        // so an abstention is tolerated (a metric, not a gate).
        let bound = rowan_stub.and_then(|stub| bound_canonical(events, stub));
        let bound_ok = bound.is_some()
            && bound == rowan_bare
            && bound.is_some_and(|id| designated_primary(events, id));

        // Gating: stub B ends unbound — no `same_as` edge touches `person/sable@chat`, and no suffixed
        // duplicate was minted for it. The aunt's facts describe a different person, so binding to
        // `person/sable` (or minting a `person/sable-N`) would be a mis-identification. Holds even when
        // the model abstains (nothing is bound or minted at all), so it fails only on a genuine
        // mis-bind.
        let sable_bound = sable_stub.is_some_and(|stub| same_as_touches(events, stub));
        let sable_minted = analysis::memories_in_namespace(events, "person/")
            .iter()
            .any(|name| name.starts_with("person/sable-"));

        vec![
            Verdict::metric_outcome(
                "bound the orphaned profile to its stub",
                bound_ok,
                "the empty stub was same_as-bound to the bare profile its borrowed evidence names, and \
                 that profile was designated the class primary",
                "the empty stub was not bound to the profile holding its borrowed evidence",
            ),
            Verdict::oracle_outcome(
                "left the relative's profile unbound",
                !sable_bound && !sable_minted,
                "the stub whose borrowed evidence describes a relative was left unbound — no same_as \
                 onto it, and no suffixed duplicate minted",
                "MIS-BIND: the pass bound a stub to a profile whose facts describe a different person \
                 sharing the name",
            ),
        ]
    }
}

/// The bare (non-platform-qualified) canonical profile bound to `stub` by a `same_as` link, if the run
/// minted and bound one. Resolves the partner of every `same_as` edge touching the stub and returns the
/// first whose handle is a bare `person/` name (no `@platform` suffix).
fn bound_canonical(events: &[Event], stub: MemoryId) -> Option<MemoryId> {
    let names = analysis::memory_names(events);
    events.iter().find_map(|event| match &event.payload {
        EventPayload::LinkCreated {
            from, to, relation, ..
        } if *relation == RelationName::SameAs && (*from == stub || *to == stub) => {
            let partner = if *from == stub { *to } else { *from };
            let name = names.get(&partner)?;
            (name.starts_with("person/") && !name.contains('@')).then_some(partner)
        }
        _ => None,
    })
}

/// Whether `memory` is the designated class primary — a `ClassPrimaryDesignated { designated: true }`.
fn designated_primary(events: &[Event], memory: MemoryId) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::ClassPrimaryDesignated { memory: designated, designated: true }
                if *designated == memory
        )
    })
}

/// Whether any `same_as` link has `memory` as an endpoint.
fn same_as_touches(events: &[Event], memory: MemoryId) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::LinkCreated { from, to, relation, .. }
                if *relation == RelationName::SameAs && (*from == memory || *to == memory)
        )
    })
}

/// Whether any merge proposal has `memory` as an endpoint.
fn merge_touches(events: &[Event], memory: MemoryId) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::MergeProposed { from, to, .. }
                if *from == memory || *to == memory
        )
    })
}

fn append(id: MemoryId, now: Timestamp, text: &str) -> EventPayload {
    EventPayload::MemoryContentAppended {
        id,
        entry_id: EntryId::generate(),
        asserted_at: now,
        occurred_at: None,
        text: text.to_owned(),
        told_by: Teller::Agent,
        told_in: None,
        visibility: Visibility::Public,
    }
}

/// Append an entry told by a platform stub — the person's own facts recorded on a bare profile,
/// teller-stamped from the stub, as the pass's borrowed-evidence path reads them.
fn told_by_stub(id: MemoryId, stub: MemoryId, now: Timestamp, text: &str) -> EventPayload {
    EventPayload::MemoryContentAppended {
        id,
        entry_id: EntryId::generate(),
        asserted_at: now,
        occurred_at: None,
        text: text.to_owned(),
        told_by: Teller::Participant(stub),
        told_in: None,
        visibility: Visibility::Public,
    }
}
