//! The materializer: folding committed events into the graph projection in `Seq` order. Dispatch is
//! on the payload's `(type, version)`; a wrong arm silently mis-projects, so the arms warrant care.

use rusqlite::params;

use super::{Graph, GraphError, backend};
use crate::{
    event::{Event, EventPayload},
    vocabulary::RelationName,
};

/// The denormalized occurrence values for one entry's `content_entries` row: the tagged-JSON
/// `occurred_at` and the `(sort, lo, hi)` millisecond bounds derived from it.
pub(super) struct OccurrenceColumns {
    json: Option<String>,
    sort: Option<i64>,
    lo: Option<i64>,
    hi: Option<i64>,
}

impl Graph {
    /// Fold a single event into the projection, then advance the head. The match arm is the
    /// `(type, version)` dispatch; a wrong arm is a silent-leak class — a mis-dispatched event folds
    /// into the wrong projection state with no error, so the match must stay exhaustive and exact.
    pub fn apply(&mut self, event: &Event) -> Result<(), GraphError> {
        match &event.payload {
            // No graph projection: orchestration/behavioral config which the server reads from the log
            // rather than the graph.
            EventPayload::PromptTemplateRegistered { .. }
            | EventPayload::ConfigSet { .. }
            | EventPayload::LuaExecuted { .. }
            | EventPayload::ConversationTurn { .. } => {}
            // The genesis marker baselines the describer: every memory that exists at genesis (the
            // seeded `self`) is treated as already described, so the first describer tick after a fresh
            // genesis regenerates nothing before real content lands. Setting `last_described_seq` to
            // each memory's `last_content_seq` clears any staleness the seeding writes created.
            EventPayload::GenesisCompleted { .. } => {
                self.conn
                    .execute(
                        "UPDATE memories SET last_described_seq = last_content_seq",
                        [],
                    )
                    .map_err(backend)?;
            }
            EventPayload::DescribePassCompleted { memories } => {
                for memory in memories {
                    self.conn
                        .execute(
                            "UPDATE memories SET last_described_seq = ?1 WHERE id = ?2",
                            params![event.seq.0 as i64, memory.0.to_string()],
                        )
                        .map_err(backend)?;
                }
            }
            // The model-interaction record is log-only telemetry, read from the log rather than
            // projected (spec §Observability), and replay-inert by construction.
            EventPayload::ModelCalled { .. } => {}
            // An embedding-model swap bears only on the vector index (a separate projection); it is
            // acted on at boot, never in the graph materializer.
            EventPayload::EmbeddingModelChanged { .. } => {}
            // The merge proposal and its adjudication are log-only audit records; neither touches the
            // projection. A proposal is deliberately inert (it leaves both stubs in their own classes,
            // so nothing surfaces across the would-be merge), and an *accepted* adjudication does its
            // merging through a separately-emitted `same_as` link (which recomputes classes), not here.
            EventPayload::EntryTemporalResolveFailed { .. }
            | EventPayload::MergeProposed { .. }
            | EventPayload::MergeAdjudicated { .. }
            | EventPayload::LinksInferred { .. } => {}
            // The arbitration's reconciling resolution stays a log-only audit record, but its
            // unresolved competing entries are projected so reads can mark a fact as disputed (spec
            // §Write path → arbitration). Each synthesis cycle replaces the memory's prior dispute
            // state; a resolution that credits a side clears it, since the disagreement is settled.
            // The "≥2 live competing entries" rule is applied at read time, so superseding one
            // account ends the dispute without a second apply pass.
            EventPayload::BeliefArbitrated {
                memory,
                competing_entries,
                resolution,
                ..
            } => {
                self.conn
                    .execute(
                        "DELETE FROM entry_disputes WHERE memory_id = ?1",
                        params![memory.0.to_string()],
                    )
                    .map_err(backend)?;
                if resolution.credited.is_empty() {
                    for entry in competing_entries {
                        self.conn
                            .execute(
                                "INSERT OR REPLACE INTO entry_disputes (entry_id, memory_id, statement)
                                 VALUES (?1, ?2, ?3)",
                                params![
                                    entry.0.to_string(),
                                    memory.0.to_string(),
                                    resolution.statement
                                ],
                            )
                            .map_err(backend)?;
                    }
                }
            }
            EventPayload::MemoryCreated { .. }
            | EventPayload::MemoryRenamed { .. }
            | EventPayload::MemoryDeleted { .. }
            | EventPayload::MemoryContentAppended { .. }
            | EventPayload::MemorySuperseded { .. }
            | EventPayload::EntryDescriptionMirrored { .. }
            | EventPayload::EntryTemporalResolved { .. }
            | EventPayload::ScheduledJobFired { .. }
            | EventPayload::ScheduledItemSurfaced { .. }
            | EventPayload::MemoryDescriptionRegenerated { .. }
            | EventPayload::MemoryVolatilitySet { .. } => {
                self.apply_memory_event(event)?;
            }
            EventPayload::TagCreated { name, description } => {
                self.conn
                    .execute(
                        "INSERT INTO tags (name, description) VALUES (?1, ?2)",
                        params![name.as_str(), description],
                    )
                    .map_err(backend)?;
            }
            EventPayload::TagDescriptionChanged {
                name,
                new_description,
            } => {
                self.conn
                    .execute(
                        "UPDATE tags SET description = ?1 WHERE name = ?2",
                        params![new_description, name.as_str()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::TagAppliedToMemory { memory, tag } => {
                self.conn
                    .execute(
                        "INSERT OR IGNORE INTO memory_tags (memory_id, tag) VALUES (?1, ?2)",
                        params![memory.0.to_string(), tag.as_str()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::TagRemovedFromMemory { memory, tag } => {
                self.conn
                    .execute(
                        "DELETE FROM memory_tags WHERE memory_id = ?1 AND tag = ?2",
                        params![memory.0.to_string(), tag.as_str()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::LinkTypeRegistered {
                name,
                inverse,
                from_card,
                to_card,
                symmetric,
                reflexive,
                description,
            } => {
                self.conn
                    .execute(
                        "INSERT INTO relations (name, inverse, from_card, to_card, symmetric, reflexive, description)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                         ON CONFLICT(name) DO UPDATE SET
                             inverse = excluded.inverse, from_card = excluded.from_card,
                             to_card = excluded.to_card, symmetric = excluded.symmetric,
                             reflexive = excluded.reflexive, description = excluded.description",
                        params![
                            name.as_str(),
                            inverse.as_str(),
                            from_card.as_str(),
                            to_card.as_str(),
                            i64::from(*symmetric),
                            i64::from(*reflexive),
                            description,
                        ],
                    )
                    .map_err(backend)?;
            }
            EventPayload::LinkCreated {
                from,
                to,
                relation,
                source,
                told_by,
                told_in,
                visibility,
            } => {
                let edge = self.canonical_edge(*from, *to, relation)?;
                let told_by = told_by
                    .as_ref()
                    .map(|teller| serde_json::to_string(teller).map_err(GraphError::Serialize))
                    .transpose()?;
                let told_in = told_in
                    .as_ref()
                    .map(|r| serde_json::to_string(r).map_err(GraphError::Serialize))
                    .transpose()?;
                let visibility_json =
                    serde_json::to_string(visibility).map_err(GraphError::Serialize)?;
                self.conn
                    .execute(
                        "INSERT OR IGNORE INTO links (from_id, to_id, relation, source, told_by, told_in, visibility)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            edge.0,
                            edge.1,
                            edge.2,
                            source.as_str(),
                            told_by,
                            told_in,
                            visibility_json
                        ],
                    )
                    .map_err(backend)?;
                if RelationName::new(edge.2.as_str()) == RelationName::SameAs {
                    self.recompute_classes()?;
                }
            }
            EventPayload::LinkRemoved { from, to, relation } => {
                let edge = self.canonical_edge(*from, *to, relation)?;
                self.conn
                    .execute(
                        "DELETE FROM links WHERE from_id = ?1 AND to_id = ?2 AND relation = ?3",
                        params![edge.0, edge.1, edge.2],
                    )
                    .map_err(backend)?;
                if RelationName::new(edge.2.as_str()) == RelationName::SameAs {
                    self.recompute_classes()?;
                }
            }
            EventPayload::ConversationStarted { .. }
            | EventPayload::ConversationEnded { .. }
            | EventPayload::SessionStarted { .. }
            | EventPayload::SessionEnded { .. }
            | EventPayload::ParticipantJoined { .. }
            | EventPayload::ParticipantIdentified { .. } => {
                self.apply_session_event(event)?;
            }
        }

        self.conn
            .execute(
                "INSERT INTO meta (key, value) VALUES ('graph_head', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![event.seq.0 as i64],
            )
            .map_err(backend)?;
        Ok(())
    }
}

mod helpers;
mod memory_events;
mod session_events;

#[cfg(test)]
mod tests;
