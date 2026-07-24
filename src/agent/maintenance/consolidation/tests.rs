//! Consolidation pass integration tests: driving the whole `catch_up` sweep against a real store,
//! graph, and embedder. Two properties are covered end to end: that connector-origin entries are
//! excluded from both tiers, and that tier 1's two-stage design honours the model's membership
//! selection — the pass consolidates only the selected subset of a candidate cluster and leaves the
//! rest live, or declines when fewer than two members are validly selected.
//!
//! The clustering unit tests fabricate an [`EntryOrigin`](crate::graph::EntryOrigin) directly on an
//! `EntryView`, so they cannot cover the projection: that an entry recorded under
//! [`EventSource::PlatformConnector`] actually materialises with a connector origin, and that the pass
//! then spares it. These tests seed connector-sourced events, materialise the graph, and drive the
//! sweep, so the whole store → graph → pass → store round trip is under test.

use std::sync::Arc;

use crate::{
    agent::maintenance::consolidation::catch_up,
    clock::ManualClock,
    engine::Engine,
    event::{EventPayload, EventSource, PromptTemplateName, Teller, Visibility},
    graph::Graph,
    ids::{EntryId, MemoryId, Namespace, Seq},
    model::{
        Completion, ModelClient, ScriptedModel,
        embed::{CpuEmbedder, Embedder},
    },
    store::{MemoryStore, Store},
    time::Timestamp,
    vector::InMemoryVectorIndex,
};

/// Build an `Arc<Engine>` with semantic retrieval attached (the consolidation pass no-ops without it):
/// `agent_events` are committed under [`EventSource::Agent`], `connector_events` under a Discord
/// [`EventSource::PlatformConnector`], then the graph is materialised.
fn engine_with_retrieval(
    agent_events: Vec<EventPayload>,
    connector_events: Vec<EventPayload>,
) -> Arc<Engine> {
    let embedder: Arc<dyn Embedder> = CpuEmbedder::shared();
    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            agent_events,
        )
        .unwrap();
    if !connector_events.is_empty() {
        store
            .append(
                Timestamp::from_millis(1_100),
                EventSource::PlatformConnector("discord".to_owned()),
                connector_events,
            )
            .unwrap();
    }
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    Engine::with_retrieval(
        Box::new(store),
        graph,
        Box::new(ManualClock::new(Timestamp::from_millis(2_000))),
        embedder,
        Box::new(InMemoryVectorIndex::new()),
    )
}

/// The `EntryConsolidation` template the sweep needs to avoid an early return.
fn template() -> EventPayload {
    EventPayload::prompt_template_registered(
        PromptTemplateName::EntryConsolidation,
        2,
        "synthesize the cluster".to_owned(),
    )
}

/// A content append with a caller-chosen entry id, so a test can trace a specific entry through a
/// consolidation.
fn append(
    id: MemoryId,
    entry_id: EntryId,
    text: &str,
    told_by: Teller,
    visibility: Visibility,
) -> EventPayload {
    EventPayload::MemoryContentAppended {
        id,
        entry_id,
        asserted_at: Timestamp::from_millis(1_000),
        occurred_at: None,
        text: text.to_owned(),
        told_by,
        told_in: None,
        visibility,
    }
}

/// A scripted tier-1 synthesis reply: the model's selected subset of the candidate cluster plus the
/// consolidated text, as the structured JSON the synthesis parser reads.
fn synthesis_reply(selected: &[EntryId], text: &str) -> Completion {
    let ids: Vec<String> = selected.iter().map(|id| id.0.to_string()).collect();
    let body = serde_json::json!({
        "selected_entry_ids": ids,
        "consolidated_text": text,
    });
    Completion::Reply(body.to_string())
}

/// Whether an entry id is still live on its class (present and not superseded) in the materialised
/// graph.
fn is_live(engine: &Engine, memory: MemoryId, entry: EntryId) -> bool {
    let graph = engine.graph.lock();
    graph
        .class_entries(memory)
        .unwrap()
        .into_iter()
        .any(|view| view.entry_id == entry && view.superseded_by.is_none())
}

/// Every `EntriesConsolidated` the sweep committed, as `(sources, replacement)`.
fn consolidations(engine: &Engine) -> Vec<(Vec<EntryId>, EntryId)> {
    engine
        .store
        .lock()
        .read_from(Seq::ZERO)
        .unwrap()
        .into_iter()
        .filter_map(|event| match event.payload {
            EventPayload::EntriesConsolidated {
                sources,
                replacement,
                ..
            } => Some((sources, replacement)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn tier2_spares_a_connector_entry_while_retiring_its_recorded_twin() {
    // Three entries with deliberately identical text (so similarity is unambiguous under the real
    // embedder): a public recorded entry, a private recorded near-duplicate that tier 2 should retire
    // into it, and a private connector-maintained near-duplicate that must be left untouched. The
    // recorded twin is the positive control: it proves the dedup fires, so the connector entry's
    // survival is the exclusion at work, not the dedup simply not triggering.
    let dave: MemoryId = MemoryId::generate();
    let public = EntryId::generate();
    let recorded_private = EntryId::generate();
    let connector_private = EntryId::generate();
    let fact = "Dave is a senior backend engineer.";

    let agent = vec![
        template(),
        EventPayload::memory_created(dave, Namespace::Person.with_name("dave")),
        append(dave, public, fact, Teller::Agent, Visibility::Public),
        append(
            dave,
            recorded_private,
            fact,
            Teller::Agent,
            Visibility::PrivateToTeller,
        ),
    ];
    let connector = vec![append(
        dave,
        connector_private,
        fact,
        Teller::Agent,
        Visibility::PrivateToTeller,
    )];
    let engine = engine_with_retrieval(agent, connector);
    // Every tier-1 group is a singleton, so no synthesis is attempted — an unexpected model call
    // would panic the scripted model.
    let model = ScriptedModel::new([]);

    catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    let consolidations = consolidations(&engine);
    assert_eq!(
        consolidations,
        vec![(vec![recorded_private], public)],
        "only the recorded private twin is retired, into the public entry"
    );
    assert!(
        !consolidations
            .iter()
            .any(|(sources, _)| sources.contains(&connector_private)),
        "the connector-maintained entry is never a consolidation source"
    );
}

#[tokio::test]
async fn tier1_never_synthesizes_connector_only_duplicates() {
    // Two connector-maintained entries with identical text would ordinarily cluster and be
    // synthesized. Excluded from grouping, they form no cluster, so the pass never calls the model
    // (the empty scripted model would panic on any call) and writes no consolidation.
    let dave: MemoryId = MemoryId::generate();
    let first = EntryId::generate();
    let second = EntryId::generate();
    let handle = "dave goes by dave in the channel";

    let agent = vec![
        template(),
        EventPayload::memory_created(dave, Namespace::Person.with_name("dave")),
    ];
    let connector = vec![
        append(dave, first, handle, Teller::Agent, Visibility::Public),
        append(dave, second, handle, Teller::Agent, Visibility::Public),
    ];
    let engine = engine_with_retrieval(agent, connector);
    let model = ScriptedModel::new([]);

    catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    assert!(
        consolidations(&engine).is_empty(),
        "connector-maintained duplicates are never consolidated"
    );
}

#[tokio::test]
async fn tier2_retires_a_recorded_private_duplicate_into_its_public_entry() {
    // A pure recorded-entry control (no connector origin) to anchor the positive path the exclusion
    // test contrasts against: a private near-duplicate of a public entry is retired into it, with the
    // public entry as the replacement and no new text written.
    let dave: MemoryId = MemoryId::generate();
    let public = EntryId::generate();
    let private = EntryId::generate();
    let fact = "Dave lives in the inner north of Melbourne.";

    let agent = vec![
        template(),
        EventPayload::memory_created(dave, Namespace::Person.with_name("dave")),
        append(dave, public, fact, Teller::Agent, Visibility::Public),
        append(
            dave,
            private,
            fact,
            Teller::Agent,
            Visibility::PrivateToTeller,
        ),
    ];
    let engine = engine_with_retrieval(agent, Vec::new());
    let model = ScriptedModel::new([]);

    catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    assert_eq!(
        consolidations(&engine),
        vec![(vec![private], public)],
        "the private near-duplicate is retired into the existing public entry"
    );
}

#[tokio::test]
async fn tier1_consolidates_only_the_model_selected_subset() {
    // Three near-identical public entries cluster together at the loose candidate bar, so all three
    // reach one synthesis call. The scripted model selects only the first two; the pass consolidates
    // exactly that subset, and the unselected third stays live rather than being folded in or
    // re-clustered this sweep. This is the two-stage design end to end: geometry gathers the candidate
    // cluster, the model disposes on membership.
    let alex: MemoryId = MemoryId::generate();
    let first = EntryId::generate();
    let second = EntryId::generate();
    let third = EntryId::generate();

    let agent = vec![
        template(),
        EventPayload::memory_created(alex, Namespace::Person.with_name("alex")),
        append(
            alex,
            first,
            "Alex is a senior backend engineer.",
            Teller::Agent,
            Visibility::Public,
        ),
        append(
            alex,
            second,
            "Alex works as a senior backend engineer.",
            Teller::Agent,
            Visibility::Public,
        ),
        append(
            alex,
            third,
            "Alex is employed as a senior backend engineer.",
            Teller::Agent,
            Visibility::Public,
        ),
    ];
    let engine = engine_with_retrieval(agent, Vec::new());
    // Exactly one synthesis call is expected (the one candidate cluster); it selects the first two.
    let model = ScriptedModel::new([synthesis_reply(
        &[first, second],
        "Alex is a senior backend engineer.",
    )]);

    catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    let consolidations = consolidations(&engine);
    assert_eq!(
        consolidations.len(),
        1,
        "exactly one consolidation is written for the single candidate cluster"
    );
    assert_eq!(
        consolidations[0].0,
        vec![first, second],
        "only the model-selected subset is consolidated"
    );
    assert!(
        !is_live(&engine, alex, first) && !is_live(&engine, alex, second),
        "the selected pair is tombstoned"
    );
    assert!(
        is_live(&engine, alex, third),
        "the unselected third entry stays live"
    );
}

#[tokio::test]
async fn tier1_declines_when_the_model_selects_too_few() {
    // The same three-entry candidate cluster, but the model names only one valid id (plus an id that
    // resolves to no cluster member). A validated selection of fewer than two members is a decline, so
    // nothing is consolidated and all three entries stay live.
    let alex: MemoryId = MemoryId::generate();
    let first = EntryId::generate();
    let second = EntryId::generate();
    let third = EntryId::generate();

    let agent = vec![
        template(),
        EventPayload::memory_created(alex, Namespace::Person.with_name("alex")),
        append(
            alex,
            first,
            "Alex is a senior backend engineer.",
            Teller::Agent,
            Visibility::Public,
        ),
        append(
            alex,
            second,
            "Alex works as a senior backend engineer.",
            Teller::Agent,
            Visibility::Public,
        ),
        append(
            alex,
            third,
            "Alex is employed as a senior backend engineer.",
            Teller::Agent,
            Visibility::Public,
        ),
    ];
    let engine = engine_with_retrieval(agent, Vec::new());
    // One valid id and one that resolves to no member: a single validated selection, below the merge
    // floor, so the pass declines.
    let bogus = EntryId::generate();
    let model = ScriptedModel::new([synthesis_reply(
        &[first, bogus],
        "Alex is a senior backend engineer.",
    )]);

    catch_up(&engine, &model as &dyn ModelClient, Seq::ZERO)
        .await
        .unwrap();

    assert!(
        consolidations(&engine).is_empty(),
        "a sub-two selection declines, so nothing is consolidated"
    );
    assert!(
        is_live(&engine, alex, first)
            && is_live(&engine, alex, second)
            && is_live(&engine, alex, third),
        "all three candidates stay live after a decline"
    );
}

#[test]
fn a_sweep_set_collapses_a_class_to_one_representative() {
    // A merged identity's members would each drive a full cluster-and-write iteration over the
    // same class entries; the sweep set collapses to the class id, so a class is processed once.
    let bare = MemoryId::generate();
    let stub = MemoryId::generate();
    let engine = engine_with_retrieval(
        vec![
            EventPayload::LinkTypeRegistered {
                name: crate::vocabulary::RelationName::SameAs,
                inverse: crate::vocabulary::RelationName::SameAs,
                from_card: crate::event::Cardinality::Many,
                to_card: crate::event::Cardinality::Many,
                symmetric: true,
                reflexive: false,
                description: String::new(),
            },
            EventPayload::memory_created(bare, Namespace::Person.with_name("rowan")),
            EventPayload::memory_created(stub, Namespace::Person.with_name("1234567890@testchat")),
            EventPayload::link_created(
                stub,
                bare,
                crate::vocabulary::RelationName::SameAs,
                crate::event::LinkPosture {
                    source: crate::event::LinkSource::Operator,
                    told_by: None,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            ),
        ],
        Vec::new(),
    );
    let deduped = crate::agent::maintenance::dedupe_by_class(&engine, vec![bare, stub]).unwrap();
    assert_eq!(
        deduped.len(),
        1,
        "both members collapse to one class representative: {deduped:?}"
    );
}
