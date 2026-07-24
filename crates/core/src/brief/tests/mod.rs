//! Contextual-brief composition tests (spec appendix scenarios 2, 14, 21 — the deterministic
//! `[brief]`/`[predicate]` surface). Each builds a materialized graph, composes a brief for a
//! present set, and asserts a fact is present or absent — model-free, because composition is
//! deterministic.
use crate::{
    brief::{self, BriefRequest},
    event::{Cardinality, EventPayload, EventSource, LinkPosture, LinkSource, Teller, Visibility},
    graph::Graph,
    ids::{EntryId, MemoryId, MemoryName},
    settings::BriefSettings,
    store::{MemoryStore, Store},
    time::{TemporalRef, Timestamp},
    vocabulary::RelationName,
};

/// Compose a brief at the epoch (these deterministic tests don't exercise the time-relative
/// `<upcoming/>` window unless they plant a future occurrence, so a fixed `now` keeps them stable).
pub(super) fn compose_at_epoch(
    graph: &Graph,
    settings: &BriefSettings,
    present_set: &[MemoryId],
    current_context: Option<MemoryId>,
    working_set: &[MemoryId],
) -> String {
    compose_at_epoch_answering(
        graph,
        settings,
        present_set,
        &[],
        current_context,
        working_set,
    )
}

/// [`compose_at_epoch`] with an explicit speaker set — the participants the session opens to answer,
/// each guaranteed a full block.
pub(super) fn compose_at_epoch_answering(
    graph: &Graph,
    settings: &BriefSettings,
    present_set: &[MemoryId],
    speakers: &[MemoryId],
    current_context: Option<MemoryId>,
    working_set: &[MemoryId],
) -> String {
    brief::compose(
        graph,
        settings,
        &BriefRequest {
            present_set,
            speakers,
            current_context,
            working_set,
            now: Timestamp::from_millis(0),
        },
    )
    .unwrap()
}

/// A content append carrying an `occurred_at` (the `appended` helper below leaves it `None`).
pub(super) fn appended_at(
    id: MemoryId,
    occurred_at: TemporalRef,
    text: &str,
    told_by: Teller,
    visibility: Visibility,
) -> EventPayload {
    EventPayload::MemoryContentAppended {
        id,
        entry_id: EntryId::generate(),
        asserted_at: Timestamp::from_millis(0),
        occurred_at: Some(occurred_at),
        text: text.to_owned(),
        told_by,
        told_in: None,
        visibility,
    }
}

/// Build a store, append `payloads`, and materialize a fresh in-memory graph from them.
pub(super) fn materialized(payloads: Vec<EventPayload>) -> (MemoryStore, Graph) {
    let mut store = MemoryStore::new();
    store
        .append(Timestamp::from_millis(1_000), EventSource::Agent, payloads)
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    (store, graph)
}

pub(super) fn created(id: MemoryId, name: &str) -> EventPayload {
    EventPayload::memory_created(id, MemoryName::new(name))
}

pub(super) fn appended(
    id: MemoryId,
    at_ms: i64,
    text: &str,
    told_by: Teller,
    visibility: Visibility,
) -> EventPayload {
    EventPayload::MemoryContentAppended {
        id,
        entry_id: EntryId::generate(),
        asserted_at: Timestamp::from_millis(at_ms),
        occurred_at: None,
        text: text.to_owned(),
        told_by,
        told_in: None,
        visibility,
    }
}

/// Register a relation `name`/`inverse` (both `Many`, asymmetric) so a link can be created under it.
pub(super) fn register_relation(name: &str, inverse: &str) -> EventPayload {
    EventPayload::LinkTypeRegistered {
        name: RelationName::new(name),
        inverse: RelationName::new(inverse),
        from_card: Cardinality::Many,
        to_card: Cardinality::Many,
        symmetric: false,
        reflexive: false,
        description: String::new(),
    }
}

/// A public `relation` link from `from` to `to`.
pub(super) fn linked(from: MemoryId, to: MemoryId, relation: &str) -> EventPayload {
    EventPayload::link_created(
        from,
        to,
        RelationName::new(relation),
        LinkPosture {
            source: LinkSource::Agent,
            told_by: None,
            told_in: None,
            visibility: Visibility::Public,
        },
    )
}

/// The relationship lines of a rendered brief, in order — the `- {source} → {relation} → {target}`
/// bullets under `<relationships>`, so a test can assert the ranking without pinning the whole block.
pub(super) fn relationship_lines(rendered: &str) -> Vec<String> {
    rendered
        .lines()
        .skip_while(|line| *line != "<relationships>")
        .skip(1)
        .take_while(|line| *line != "</relationships>")
        .map(str::to_owned)
        .collect()
}

mod budget;
mod composition;
mod join_brief;
mod relationships;
mod upcoming;
