//! Per-memory described-state queries: which memories the describer still owes a pass, and the
//! windowing a per-memory temporal extraction reads.
//!
//! A memory is **stale** — it needs (re)describing — exactly while its `last_content_seq` outruns its
//! `last_described_seq` (see [`crate::graph::apply`]). Both watermarks are materialized from the log,
//! so the describe backlog is derived state that survives a restart rather than an in-memory cursor
//! reset at every boot (spec §Write path → regenerate off the hot path, as a catch-up).

use rusqlite::{OptionalExtension, params};

use super::{Graph, GraphError, backend, parse_ulid};
use crate::{
    db::query_map_into,
    ids::{EntryId, MemoryId, Seq},
};

impl Graph {
    /// Every live memory the describer still owes a pass — those whose content has changed since it
    /// was last considered — in backlog order (oldest pending content first, then id for a stable
    /// tie-break). The whole-log describer pass walks this set.
    pub fn stale_memories(&self) -> Result<Vec<MemoryId>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT id FROM memories
             WHERE deleted = 0 AND last_content_seq > last_described_seq
             ORDER BY last_content_seq, id",
        )?;
        query_map_into(stmt, [], |row| {
            let id: String = row.get(0)?;
            Ok::<_, GraphError>(MemoryId(parse_ulid(&id)?))
        })
    }

    /// The stale memories among `ids` — the narrowed pass a session open runs over its brief's read
    /// set, so it pays only for the descriptions that brief will read (spec §Starvation bound). A
    /// memory absent, deleted, or already fresh is dropped; the rest keep the same backlog order as
    /// [`Graph::stale_memories`]. Input ids are de-duplicated by the `IN` set.
    pub fn stale_memories_among(&self, ids: &[MemoryId]) -> Result<Vec<MemoryId>, GraphError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT id FROM memories
             WHERE deleted = 0 AND last_content_seq > last_described_seq AND id IN ({placeholders})
             ORDER BY last_content_seq, id"
        );
        let stmt = self.conn.prepare(&sql)?;
        let params = ids.iter().map(|id| id.0.to_string()).collect::<Vec<_>>();
        query_map_into(stmt, rusqlite::params_from_iter(params.iter()), |row| {
            let id: String = row.get(0)?;
            Ok::<_, GraphError>(MemoryId(parse_ulid(&id)?))
        })
    }

    /// The count of stale memories — the describer's backlog depth, reported as a gauge (spec
    /// §Observability → metrics). A `COUNT(*)` so a scrape stays cheap.
    pub fn stale_memory_count(&self) -> Result<u64, GraphError> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM memories
             WHERE deleted = 0 AND last_content_seq > last_described_seq",
            [],
            |row| row.get::<_, i64>(0),
        )? as u64)
    }

    /// The `last_content_seq` of the most-starved stale memory — the smallest content watermark among
    /// memories the describer still owes a pass — or `None` when the backlog is empty. Paired with the
    /// log's `recorded_at` for that seq, it dates the oldest pending description, which the describer's
    /// staleness escape compares against its horizon. Ordering by `last_content_seq` matches
    /// [`Graph::stale_memories`], so this is the watermark of the memory that pass reaches last.
    pub fn oldest_stale_content_seq(&self) -> Result<Option<Seq>, GraphError> {
        let seq: Option<i64> = self
            .conn
            .query_row(
                "SELECT MIN(last_content_seq) FROM memories
                 WHERE deleted = 0 AND last_content_seq > last_described_seq",
                [],
                |row| row.get(0),
            )
            .map_err(backend)?;
        Ok(seq.map(|seq| Seq(seq as u64)))
    }

    /// A memory's `(last_content_seq, last_described_seq)` watermarks, or `None` if it is unknown or
    /// soft-deleted. The describer re-reads these under its per-memory guard to skip a memory a
    /// concurrent pass already caught up, and reads `last_described_seq` as the lower bound of the
    /// temporal-extraction window (see [`Graph::untimed_entries_since`]).
    pub fn described_state(&self, id: MemoryId) -> Result<Option<(Seq, Seq)>, GraphError> {
        let seqs: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT last_content_seq, last_described_seq FROM memories
                 WHERE id = ?1 AND deleted = 0",
                params![id.0.to_string()],
                |row| row.try_into(),
            )
            .optional()
            .map_err(backend)?;
        Ok(seqs.map(|(content, described)| (Seq(content as u64), Seq(described as u64))))
    }

    /// The still-untimed entries of `id` appended after `after` — the entries a per-memory temporal
    /// extraction is allowed to resolve (spec §Time → in the same pass). `occurred_at IS NULL`
    /// excludes an entry the agent timed explicitly and one a prior pass already resolved, so
    /// extraction never overrides a deliberate occurrence and resolves each entry once; superseded
    /// entries are skipped as dead. A description mirror is skipped too: it restates what the memory
    /// *is* and names no time, so timing it would fabricate a "now" that collides with a later dated
    /// append (see [`crate::event::EventPayload::EntryDescriptionMirrored`]). Bounding by `seq > after`
    /// (the memory's `last_described_seq`) keeps the window to what this pass has not yet considered.
    pub fn untimed_entries_since(
        &self,
        id: MemoryId,
        after: Seq,
    ) -> Result<Vec<EntryId>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT entry_id FROM content_entries
             WHERE memory_id = ?1 AND occurred_at IS NULL AND superseded_by IS NULL
                   AND description_mirror = 0 AND seq > ?2
             ORDER BY seq",
        )?;
        query_map_into(stmt, params![id.0.to_string(), after.0 as i64], |row| {
            let entry_id: String = row.get(0)?;
            Ok::<_, GraphError>(EntryId(parse_ulid(&entry_id)?))
        })
    }
}
