//! Memory event materialization arms, extracted from [`Graph::apply`](crate::graph::Graph::apply).

use rusqlite::params;

use crate::{
    event::{Event, EventPayload, Visibility},
    graph::{GraphError, backend},
};

use crate::graph::Graph;

impl Graph {
    /// Materialize the memory-event arm of [`Graph::apply`](crate::graph::Graph::apply). Returns `Ok(true)`
    /// if the payload was a memory event and was handled, `Ok(false)` otherwise.
    pub(super) fn apply_memory_event(&mut self, event: &Event) -> Result<bool, GraphError> {
        match &event.payload {
            EventPayload::MemoryCreated { id, name } => {
                // A lone memory is its own class; a later same_as merge recomputes class_id.
                self.conn
                    .execute(
                        "INSERT INTO memories (id, name, created_at, class_id, last_content_seq)
                         VALUES (?1, ?2, ?3, ?1, ?4)",
                        params![
                            id.0.to_string(),
                            name.as_str(),
                            event.recorded_at.as_millisecond(),
                            event.seq.0 as i64,
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
            EventPayload::MemoryRenamed {
                id,
                old_name,
                new_name,
            } => {
                // A rename changes no content, but the description is synthesized under the memory's
                // name, so it must be re-described under the new handle — mark it stale by advancing
                // `last_content_seq`, matching the write-set the describer keys on.
                self.conn
                    .execute(
                        "UPDATE memories SET name = ?1, last_content_seq = ?2 WHERE id = ?3",
                        params![new_name.as_str(), event.seq.0 as i64, id.0.to_string()],
                    )
                    .map_err(backend)?;
                self.conn
                    .execute(
                        "UPDATE memories_fts SET name = ?1 WHERE memory_id = ?2",
                        params![new_name.as_str(), id.0.to_string()],
                    )
                    .map_err(backend)?;
                // Record the vacated name as an alias of this memory, so an exact lookup by an old name
                // resolves to the renamed memory (flagged as a former name; spec §Identity → Renaming).
                // Last-writer-wins on the rare collision where two memories shed the same name.
                self.conn
                    .execute(
                        "INSERT OR REPLACE INTO memory_aliases (former_name, memory_id)
                         VALUES (?1, ?2)",
                        params![old_name.as_str(), id.0.to_string()],
                    )
                    .map_err(backend)?;
                // Keep the old name searchable (alias-aware search): a renamed person found by an old
                // name surfaces in `memory.search`, not only `memory.get`. Folded into the FTS content
                // (ranking only — never displayed), so it never broadcasts; the hit's `[formerly …]`
                // marker is what the agent reads.
                self.conn
                    .execute(
                        "UPDATE memories_fts SET content = content || ' ' || ?1 WHERE memory_id = ?2",
                        params![old_name.as_str(), id.0.to_string()],
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
                // A deleted context memory takes its conversation with it: the room is the
                // conversation's identity, so a conversation whose room is gone is dropped from the
                // projection, along with its sessions and their participants (there are no foreign
                // keys, so the cascade is explicit — and it must be complete, or the idle sweep's
                // `open_sessions` would surface an orphaned open session and flush a turn into a room
                // that no longer exists). Ordered inner-to-outer so each delete's subquery still
                // resolves the conversation. The `ConversationStarted`/`SessionStarted` events stay in
                // the log (this is the materialized graph, rebuilt from it), and a non-context memory
                // matches no row.
                let id = id.0.to_string();
                self.conn
                    .execute(
                        "DELETE FROM session_participants WHERE session IN (
                             SELECT s.id FROM sessions s
                             JOIN conversations c ON s.conversation = c.id
                             WHERE c.context_memory = ?1
                         )",
                        params![id],
                    )
                    .map_err(backend)?;
                self.conn
                    .execute(
                        "DELETE FROM sessions WHERE conversation IN (
                             SELECT id FROM conversations WHERE context_memory = ?1
                         )",
                        params![id],
                    )
                    .map_err(backend)?;
                self.conn
                    .execute(
                        "DELETE FROM conversations WHERE context_memory = ?1",
                        params![id],
                    )
                    .map_err(backend)?;
            }
            EventPayload::MemoryContentAppended {
                id,
                entry_id,
                asserted_at,
                occurred_at,
                text,
                told_by,
                told_in,
                visibility,
            } => {
                // Denormalize the typed `occurred_at` into sortable columns at materialization time
                // (spec §Time); see `occurrence_columns`.
                let occurrence = self.occurrence_columns(occurred_at.as_ref())?;
                // An occurrence carried by the append is authored — the agent stamped it — and so is
                // ground truth a later extracted occurrence must never shadow (an untimed append has no
                // occurrence to classify, so it is not authored).
                let occurred_authored = i64::from(occurred_at.is_some());
                self.conn
                    .execute(
                        "INSERT INTO content_entries \
                         (entry_id, memory_id, asserted_at, occurred_at, occurred_sort, \
                          occurred_lo, occurred_hi, occurred_authored, text, told_by, told_in, \
                          visibility, seq)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                        params![
                            entry_id.0.to_string(),
                            id.0.to_string(),
                            asserted_at.as_millisecond(),
                            occurrence.json,
                            occurrence.sort,
                            occurrence.lo,
                            occurrence.hi,
                            occurred_authored,
                            text,
                            serde_json::to_string(told_by).map_err(GraphError::Serialize)?,
                            told_in
                                .as_ref()
                                .map(|r| {
                                    serde_json::to_string(r).map_err(GraphError::Serialize)
                                })
                                .transpose()?,
                            serde_json::to_string(visibility).map_err(GraphError::Serialize)?,
                            event.seq.0 as i64,
                        ],
                    )
                    .map_err(backend)?;
                // Advance the memory's content watermark so it reads as stale until the describer's
                // next pass considers it (spec §Write path → regenerate off the hot path).
                self.conn
                    .execute(
                        "UPDATE memories SET last_content_seq = ?1 WHERE id = ?2",
                        params![event.seq.0 as i64, id.0.to_string()],
                    )
                    .map_err(backend)?;
                // Only public content enters the lexical index: name and description are already
                // public-safe, so keeping FTS public-only means a lexical hit needs no visibility
                // filter. Non-public content — attributed or private — stays retrievable only via its
                // entry vector, which carries the provenance marker a lexical hit could not.
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
            EventPayload::MemorySuperseded {
                entry,
                superseded_by,
                ..
            } => {
                // Stamp the superseded entry's pointer in place; the original append row is otherwise
                // immutable. Live reads exclude it (spec §Visibility → superseded entries are not
                // live); history reads keep it. The lexical FTS blob is left as-is — a superseded
                // fact's words lingering there is a ranking artifact, not a leak, since a lexical hit
                // returns the memory (with its regenerated description), never the superseded entry.
                self.conn
                    .execute(
                        "UPDATE content_entries SET superseded_by = ?1 WHERE entry_id = ?2",
                        params![superseded_by.0.to_string(), entry.0.to_string()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::EntriesConsolidated {
                sources,
                replacement,
                ..
            } => {
                // Tombstone each source entry by stamping `superseded_by` = the replacement entry id,
                // the same mechanism `MemorySuperseded` uses for a single entry. Live reads exclude
                // them (`superseded_by IS NULL`); history reads keep them. The `EntriesConsolidated`
                // event itself carries the full many-to-one relationship that `MemorySuperseded`'s
                // one-to-one shape cannot express — a reader finds the consolidation relationship by
                // reading the event, not a side table.
                for source in sources {
                    self.conn
                        .execute(
                            "UPDATE content_entries SET superseded_by = ?1 WHERE entry_id = ?2",
                            params![replacement.0.to_string(), source.0.to_string()],
                        )
                        .map_err(backend)?;
                }
            }
            EventPayload::EntryRetracted { entry, reason, .. } => {
                // Tombstone the retracted entry: stamp its own id into superseded_by so every live
                // filter (`superseded_by IS NULL`) hides it exactly as a supersession would — with no
                // successor to point at — and record the reason, which the history reads surface. The
                // FTS blob is left as-is for the same reason supersession leaves it: a lexical hit
                // returns the memory, never the tombstoned entry.
                self.conn
                    .execute(
                        "UPDATE content_entries SET superseded_by = ?1, retracted_reason = ?2
                         WHERE entry_id = ?1",
                        params![entry.0.to_string(), reason],
                    )
                    .map_err(backend)?;
            }
            EventPayload::EntryDescriptionMirrored { entry_id, .. } => {
                // Flag the seed entry as a description mirror in place; the append row is otherwise
                // immutable. The turn-end temporal extraction then skips it, so its untimed text never
                // acquires a fabricated occurrence that would collide with a later dated append.
                self.conn
                    .execute(
                        "UPDATE content_entries SET description_mirror = 1 WHERE entry_id = ?1",
                        params![entry_id.0.to_string()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::EntryTemporalResolved {
                entry_id,
                occurred_at,
                ..
            } => {
                // The extraction pass resolved this entry's occurrence after it was appended;
                // recompute its denormalized columns in place (text and FTS are untouched). This
                // occurrence is inferred, not authored, so `occurred_authored` stays 0 — a
                // representative-date projection must not let this guess shadow a stated date.
                let occurrence = self.occurrence_columns(Some(occurred_at))?;
                self.conn
                    .execute(
                        "UPDATE content_entries
                         SET occurred_at = ?1, occurred_sort = ?2, occurred_lo = ?3, occurred_hi = ?4,
                             occurred_authored = 0
                         WHERE entry_id = ?5",
                        params![
                            occurrence.json,
                            occurrence.sort,
                            occurrence.lo,
                            occurrence.hi,
                            entry_id.0.to_string(),
                        ],
                    )
                    .map_err(backend)?;
            }
            EventPayload::ScheduledJobFired {
                entry_id, fired_at, ..
            } => {
                // Clear `surfaced_at` so a firing re-arms the surface: a recurring entry fires once per
                // instance, and each firing must become pending again for the drain. A concrete entry
                // fires only once, so clearing its (already-null) surface is a no-op for it.
                self.conn
                    .execute(
                        "UPDATE content_entries SET fired_at = ?1, surfaced_at = NULL \
                         WHERE entry_id = ?2",
                        params![fired_at.as_millisecond(), entry_id.0.to_string()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::ScheduledItemSurfaced {
                entry_id,
                surfaced_at,
                ..
            } => {
                self.conn
                    .execute(
                        "UPDATE content_entries SET surfaced_at = ?1 WHERE entry_id = ?2",
                        params![surfaced_at.as_millisecond(), entry_id.0.to_string()],
                    )
                    .map_err(backend)?;
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
            EventPayload::ClassPrimaryDesignated { memory, designated } => {
                // The flag lives on the memory, so it travels with the stub across a later unmerge and
                // is inert while the stub names no live memory. Recompute the classes so the pin (or its
                // release) takes effect on this read, the same whole recompute a `same_as` change runs.
                self.conn
                    .execute(
                        "UPDATE memories SET designated_primary = ?1 WHERE id = ?2",
                        params![i64::from(*designated), memory.0.to_string()],
                    )
                    .map_err(backend)?;
                self.recompute_classes()?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}
