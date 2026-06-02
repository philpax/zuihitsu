//! The materializer: folding committed events into the graph projection in `Seq` order. Dispatch is
//! on the payload's `(type, version)`; a wrong arm is a silent-leak class the eval harness backstops.

use rusqlite::{OptionalExtension, params};

use super::{Graph, GraphError, backend};
use crate::{
    event::{Event, EventPayload, Visibility},
    ids::{MemoryId, RelationName},
};

impl Graph {
    /// Fold a single event into the projection, then advance the head. The match arm is the
    /// `(type, version)` dispatch; a wrong arm is a silent-leak class the eval harness backstops.
    pub fn apply(&mut self, event: &Event) -> Result<(), GraphError> {
        match &event.payload {
            // No graph projection: genesis marker, and orchestration/behavioral config which the
            // server reads from the log rather than the graph.
            EventPayload::GenesisCompleted { .. }
            | EventPayload::PromptTemplateRegistered { .. }
            | EventPayload::ConfigSet { .. }
            | EventPayload::LuaExecuted { .. }
            | EventPayload::ConversationTurn { .. } => {}
            EventPayload::MemoryCreated { id, name } => {
                self.conn
                    .execute(
                        "INSERT INTO memories (id, name, created_at) VALUES (?1, ?2, ?3)",
                        params![
                            id.0.to_string(),
                            name.as_str(),
                            event.recorded_at.as_millis()
                        ],
                    )
                    .map_err(backend)?;
                self.conn
                    .execute(
                        "INSERT INTO memories_fts (memory_id, name, description, content)
                         VALUES (?1, ?2, '', '')",
                        params![id.0.to_string(), name.as_str()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::MemoryRenamed { id, new_name, .. } => {
                self.conn
                    .execute(
                        "UPDATE memories SET name = ?1 WHERE id = ?2",
                        params![new_name.as_str(), id.0.to_string()],
                    )
                    .map_err(backend)?;
                self.conn
                    .execute(
                        "UPDATE memories_fts SET name = ?1 WHERE memory_id = ?2",
                        params![new_name.as_str(), id.0.to_string()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::MemoryDeleted { id } => {
                self.conn
                    .execute(
                        "UPDATE memories SET deleted = 1 WHERE id = ?1",
                        params![id.0.to_string()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::MemoryContentAppended {
                id,
                entry_id,
                asserted_at,
                text,
                told_by,
                told_in,
                visibility,
            } => {
                self.conn
                    .execute(
                        "INSERT INTO content_entries \
                         (entry_id, memory_id, asserted_at, text, told_by, told_in, visibility, seq)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        params![
                            entry_id.0.to_string(),
                            id.0.to_string(),
                            asserted_at.as_millis(),
                            text,
                            serde_json::to_string(told_by).map_err(GraphError::Serialize)?,
                            told_in.map(|memory| memory.0.to_string()),
                            serde_json::to_string(visibility).map_err(GraphError::Serialize)?,
                            event.seq.0 as i64,
                        ],
                    )
                    .map_err(backend)?;
                // Only public content enters the lexical index: name and description are already
                // public-safe, so keeping FTS public-only means a lexical hit needs no visibility
                // filter. Private content stays retrievable only via its (predicate-filtered) entry
                // vector.
                if *visibility == Visibility::Public {
                    self.conn
                        .execute(
                            "UPDATE memories_fts SET content = content || ' ' || ?1
                             WHERE memory_id = ?2",
                            params![text, id.0.to_string()],
                        )
                        .map_err(backend)?;
                }
            }
            EventPayload::MemoryDescriptionRegenerated { id, new_text, .. } => {
                self.conn
                    .execute(
                        "UPDATE memories SET description = ?1 WHERE id = ?2",
                        params![new_text, id.0.to_string()],
                    )
                    .map_err(backend)?;
                self.conn
                    .execute(
                        "UPDATE memories_fts SET description = ?1 WHERE memory_id = ?2",
                        params![new_text, id.0.to_string()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::MemoryVolatilitySet { id, volatility } => {
                self.conn
                    .execute(
                        "UPDATE memories SET volatility = ?1 WHERE id = ?2",
                        params![volatility.as_str(), id.0.to_string()],
                    )
                    .map_err(backend)?;
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
            } => {
                self.conn
                    .execute(
                        "INSERT INTO relations (name, inverse, from_card, to_card, symmetric, reflexive)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                         ON CONFLICT(name) DO UPDATE SET
                             inverse = excluded.inverse, from_card = excluded.from_card,
                             to_card = excluded.to_card, symmetric = excluded.symmetric,
                             reflexive = excluded.reflexive",
                        params![
                            name.as_str(),
                            inverse.as_str(),
                            from_card.as_str(),
                            to_card.as_str(),
                            i64::from(*symmetric),
                            i64::from(*reflexive),
                        ],
                    )
                    .map_err(backend)?;
            }
            EventPayload::LinkCreated {
                from,
                to,
                relation,
                source,
            } => {
                let edge = self.canonical_edge(*from, *to, relation)?;
                self.conn
                    .execute(
                        "INSERT OR IGNORE INTO links (from_id, to_id, relation, source)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![edge.0, edge.1, edge.2, source.as_str()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::LinkRemoved { from, to, relation } => {
                let edge = self.canonical_edge(*from, *to, relation)?;
                self.conn
                    .execute(
                        "DELETE FROM links WHERE from_id = ?1 AND to_id = ?2 AND relation = ?3",
                        params![edge.0, edge.1, edge.2],
                    )
                    .map_err(backend)?;
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

    /// Resolve a link (asserted under either label) to its stored canonical direction:
    /// `(from_id, to_id, canonical_relation)`. A relation matched by its inverse swaps endpoints;
    /// a symmetric relation orders endpoints so `(a, b)` and `(b, a)` collapse to one edge. An
    /// unregistered relation is stored as given (the Lua layer enforces registration in Stage 4).
    fn canonical_edge(
        &self,
        from: MemoryId,
        to: MemoryId,
        relation: &RelationName,
    ) -> Result<(String, String, String), GraphError> {
        let from = from.0.to_string();
        let to = to.0.to_string();
        let label = relation.as_str();

        let resolved: Option<(String, i64)> = self
            .conn
            .query_row(
                "SELECT name, symmetric FROM relations WHERE name = ?1 OR inverse = ?1",
                params![label],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(backend)?;

        Ok(match resolved {
            None => (from, to, label.to_owned()),
            Some((canonical, symmetric)) if symmetric != 0 => {
                let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
                (lo, hi, canonical)
            }
            Some((canonical, _)) if label == canonical => (from, to, canonical),
            Some((canonical, _)) => (to, from, canonical),
        })
    }
}
