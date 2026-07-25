//! Re-home step tests: links stranded on a `same_as` class's non-primary members migrate onto the
//! class primary, verbatim, with the survivor and exemption rules the pass applies.

use super::{engine_with, prerequisites};
use crate::{
    agent::maintenance::canonicalize::catch_up,
    engine::Engine,
    event::{Cardinality, EventPayload, LinkPosture, LinkSource, Teller, Visibility},
    ids::{MemoryId, Namespace, Seq},
    model::{ModelClient, ScriptedModel},
    vocabulary::RelationName,
};

/// The `knows` (directed) and `married_to` (symmetric) relations, registered so the re-home tests can
/// hang non-`same_as` edges off class members.
fn link_relations() -> Vec<EventPayload> {
    vec![
        EventPayload::LinkTypeRegistered {
            name: RelationName::new("knows"),
            inverse: RelationName::new("known_by"),
            from_card: Cardinality::Many,
            to_card: Cardinality::Many,
            symmetric: false,
            reflexive: false,
            description: String::new(),
        },
        EventPayload::LinkTypeRegistered {
            name: RelationName::new("married_to"),
            inverse: RelationName::new("married_to"),
            from_card: Cardinality::One,
            to_card: Cardinality::One,
            symmetric: true,
            reflexive: false,
            description: String::new(),
        },
    ]
}

/// A committed operator `same_as` binding `a` to `b`, with `a` designated the class primary.
fn merge_designating(a: MemoryId, b: MemoryId) -> Vec<EventPayload> {
    vec![
        EventPayload::link_created(
            a,
            b,
            RelationName::SameAs,
            LinkPosture {
                source: LinkSource::Operator,
                told_by: None,
                told_in: None,
                visibility: Visibility::Public,
            },
        ),
        EventPayload::ClassPrimaryDesignated {
            memory: a,
            designated: true,
        },
    ]
}

/// The `(from, to, relation, posture)` of every `LinkCreated`, and the `(from, to, relation)` of every
/// `LinkRemoved`, that the sweep appended after `after` — the sweep's own link effects, isolated from
/// the seeded edges.
type LinkEffects = (
    Vec<(MemoryId, MemoryId, RelationName, LinkPosture)>,
    Vec<(MemoryId, MemoryId, RelationName)>,
);

fn link_effects_after(engine: &Engine, after: Seq) -> LinkEffects {
    let mut created = Vec::new();
    let mut removed = Vec::new();
    for event in engine.store.lock().read_from(after.next()).unwrap() {
        match event.payload {
            EventPayload::LinkCreated {
                from,
                to,
                relation,
                source,
                told_by,
                told_in,
                visibility,
            } => created.push((
                from,
                to,
                relation,
                LinkPosture {
                    source,
                    told_by,
                    told_in,
                    visibility,
                },
            )),
            EventPayload::LinkRemoved { from, to, relation } => removed.push((from, to, relation)),
            _ => {}
        }
    }
    (created, removed)
}

#[tokio::test]
async fn a_pre_class_edge_rehomes_to_the_primary_with_its_posture_verbatim() {
    // A `knows` edge accrued on a platform stub before the stub's `same_as` class formed, so it is
    // stranded on a non-primary member. The re-home step moves it onto the class primary — a `LinkRemoved`
    // off the stub and a `LinkCreated` on the primary — carrying source, told_by, told_in, and visibility
    // over unchanged, since this is a move, not a re-assertion (the write-time visibility resolution must
    // not re-run).
    let mut ids = [MemoryId::generate(), MemoryId::generate()];
    ids.sort();
    let [primary, stub] = ids;
    let far = MemoryId::generate();
    let teller = MemoryId::generate();
    let posture = LinkPosture {
        source: LinkSource::Agent,
        told_by: Some(Teller::Participant(teller)),
        told_in: None,
        // A non-default posture is the sentinel: were the resolver to re-run for this agent link, it
        // would not reproduce `Attributed`.
        visibility: Visibility::Attributed,
    };
    let mut events = prerequisites();
    events.extend(link_relations());
    events.extend([
        EventPayload::memory_created(primary, Namespace::Person.with_name("rowan")),
        EventPayload::memory_created(stub, Namespace::Person.with_name("rowan@discord")),
        EventPayload::memory_created(far, Namespace::Person.with_name("erin")),
        EventPayload::memory_created(teller, Namespace::Person.with_name("quinn")),
    ]);
    events.extend(merge_designating(primary, stub));
    events.push(EventPayload::link_created(
        stub,
        far,
        RelationName::new("knows"),
        posture.clone(),
    ));
    let engine = engine_with(events);
    let head_before = engine.store.lock().head().unwrap();
    let model = ScriptedModel::new([]);

    let outcome = catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    let (created, removed) = link_effects_after(&engine, head_before);
    assert_eq!(
        removed,
        vec![(stub, far, RelationName::new("knows"))],
        "the stub's stranded edge is withdrawn"
    );
    assert_eq!(
        created,
        vec![(primary, far, RelationName::new("knows"), posture)],
        "the edge is re-asserted on the primary with its posture verbatim"
    );
    assert_eq!(
        outcome.actions, 1,
        "the one re-homed link counts as an action"
    );
}

#[tokio::test]
async fn a_parallel_edge_keeps_the_primary_and_drops_the_stub() {
    // Both the primary and the stub carry `knows` to the same far memory, with differing postures. The
    // re-home makes explicit the choice read-side dedup was making arbitrarily: the primary's edge wins,
    // so only the stub's parallel copy is withdrawn and no create is emitted.
    let mut ids = [MemoryId::generate(), MemoryId::generate()];
    ids.sort();
    let [primary, stub] = ids;
    let far = MemoryId::generate();
    let mut events = prerequisites();
    events.extend(link_relations());
    events.extend([
        EventPayload::memory_created(primary, Namespace::Person.with_name("rowan")),
        EventPayload::memory_created(stub, Namespace::Person.with_name("rowan@discord")),
        EventPayload::memory_created(far, Namespace::Person.with_name("erin")),
    ]);
    events.extend(merge_designating(primary, stub));
    events.extend([
        EventPayload::link_created(
            primary,
            far,
            RelationName::new("knows"),
            LinkPosture {
                source: LinkSource::Agent,
                told_by: Some(Teller::Agent),
                told_in: None,
                visibility: Visibility::Public,
            },
        ),
        EventPayload::link_created(
            stub,
            far,
            RelationName::new("knows"),
            LinkPosture {
                source: LinkSource::Agent,
                told_by: Some(Teller::Agent),
                told_in: None,
                visibility: Visibility::Attributed,
            },
        ),
    ]);
    let engine = engine_with(events);
    let head_before = engine.store.lock().head().unwrap();
    let model = ScriptedModel::new([]);

    catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    let (created, removed) = link_effects_after(&engine, head_before);
    assert_eq!(
        removed,
        vec![(stub, far, RelationName::new("knows"))],
        "the stub's parallel copy is dropped"
    );
    assert!(
        created.is_empty(),
        "the primary's edge wins the parallel: no create is emitted, got {created:?}"
    );
}

#[tokio::test]
async fn a_symmetric_parallel_edge_does_not_reverse_duplicate() {
    // A symmetric `married_to` edge sits on the primary as `(primary, far)` and on the stub as the
    // reversed `(far, stub)`. Endpoints are unordered, so the stub's copy canonicalizes to the same edge
    // the primary already holds — the re-home drops it rather than creating a reversed duplicate.
    let mut ids = [MemoryId::generate(), MemoryId::generate()];
    ids.sort();
    let [primary, stub] = ids;
    let far = MemoryId::generate();
    let mut events = prerequisites();
    events.extend(link_relations());
    events.extend([
        EventPayload::memory_created(primary, Namespace::Person.with_name("rowan")),
        EventPayload::memory_created(stub, Namespace::Person.with_name("rowan@discord")),
        EventPayload::memory_created(far, Namespace::Person.with_name("erin")),
    ]);
    events.extend(merge_designating(primary, stub));
    events.extend([
        EventPayload::link_created(
            primary,
            far,
            RelationName::new("married_to"),
            LinkPosture {
                source: LinkSource::Agent,
                told_by: Some(Teller::Agent),
                told_in: None,
                visibility: Visibility::Public,
            },
        ),
        EventPayload::link_created(
            far,
            stub,
            RelationName::new("married_to"),
            LinkPosture {
                source: LinkSource::Agent,
                told_by: Some(Teller::Agent),
                told_in: None,
                visibility: Visibility::Public,
            },
        ),
    ]);
    let engine = engine_with(events);
    let head_before = engine.store.lock().head().unwrap();
    let model = ScriptedModel::new([]);

    catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    let (created, removed) = link_effects_after(&engine, head_before);
    // A symmetric edge is stored at canonical (lower-id-first) endpoints, so the withdrawal names the
    // stub's copy in that order.
    let (lo, hi) = if far < stub { (far, stub) } else { (stub, far) };
    assert_eq!(
        removed,
        vec![(lo, hi, RelationName::new("married_to"))],
        "the stub's reversed copy is withdrawn"
    );
    assert!(
        created.is_empty(),
        "no reversed duplicate is created on the primary, got {created:?}"
    );
}

#[tokio::test]
async fn same_as_and_connector_edges_are_left_untouched() {
    // The class's own `same_as` plumbing (member-level by definition) and a connector-maintained
    // structural edge on the stub (identifiable by its `PlatformConnector` source) are both exempt from
    // the re-home: neither is withdrawn.
    let mut ids = [MemoryId::generate(), MemoryId::generate()];
    ids.sort();
    let [primary, stub] = ids;
    let guild = MemoryId::generate();
    let mut events = prerequisites();
    events.extend(link_relations());
    events.extend([
        EventPayload::memory_created(primary, Namespace::Person.with_name("rowan")),
        EventPayload::memory_created(stub, Namespace::Person.with_name("rowan@discord")),
        EventPayload::memory_created(guild, Namespace::Context.with_name("guild@discord")),
    ]);
    events.extend(merge_designating(primary, stub));
    // A connector-authored structural edge on the stub — the connector holds the platform id, so it
    // stays on the exact stub.
    events.push(EventPayload::link_created(
        stub,
        guild,
        RelationName::new("knows"),
        LinkPosture {
            source: LinkSource::PlatformConnector("discord".to_owned()),
            told_by: None,
            told_in: None,
            visibility: Visibility::Public,
        },
    ));
    let engine = engine_with(events);
    let head_before = engine.store.lock().head().unwrap();
    let model = ScriptedModel::new([]);

    catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    let (created, removed) = link_effects_after(&engine, head_before);
    assert!(
        removed.is_empty() && created.is_empty(),
        "neither the same_as plumbing nor the connector edge is re-homed: created {created:?}, \
         removed {removed:?}"
    );
}
