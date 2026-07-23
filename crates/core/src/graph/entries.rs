//! Content entry reads: live, history, class-wide, disputed, and by-id.

use crate::{
    db::{query_map_into, query_opt_into},
    event::Cardinality,
    graph::{
        AttestationView, EntryOrigin, EntryView, Graph, GraphError, MemoryView, RecurringEntry,
        backend, parse_ulid, timestamp_column,
    },
    ids::{EntryId, MemoryId, Namespace},
    time::temporal::TemporalRef,
    vocabulary::RelationName,
};
use rusqlite::{params, params_from_iter};
use std::collections::{BTreeSet, HashMap};

/// Which attestations an entry read carries: the live set (every agent-facing and live console
/// read), or the full set including withdrawn rows with their reasons (the history read, where the
/// console renders a withdrawal struck-through). The visibility predicate and the chip engine skip
/// withdrawn rows regardless, so the wider scope never changes what an agent-facing surface shows.
#[derive(Clone, Copy)]
pub(super) enum AttestationScope {
    Live,
    WithWithdrawn,
}

impl Graph {
    /// Every live entry across all memories that carries a recurrence rule, with the memory it belongs
    /// to. The graph, not a re-fold of the log, is the authority on which entries recur: an entry's
    /// `occurred_at` decodes to a [`TemporalRef::Recurring`], and a superseded or retracted entry is
    /// excluded (`superseded_by IS NULL`). Ordered by memory, then commit order.
    pub fn recurring_entries(&self) -> Result<Vec<RecurringEntry>, GraphError> {
        let stmt = self
            .conn
            .prepare(
                "SELECT memory_id, text, occurred_at FROM content_entries
                 WHERE occurred_at IS NOT NULL AND superseded_by IS NULL ORDER BY memory_id, seq",
            )
            .map_err(backend)?;
        let rows: Vec<Option<RecurringEntry>> =
            query_map_into(stmt, params![], |row| -> Result<_, GraphError> {
                let occurred_at: String = row.get("occurred_at")?;
                let TemporalRef::Recurring(rrule) =
                    serde_json::from_str::<TemporalRef>(&occurred_at)?
                else {
                    return Ok(None);
                };
                let memory: String = row.get("memory_id")?;
                Ok(Some(RecurringEntry {
                    memory: MemoryId(parse_ulid(&memory)?),
                    text: row.get("text")?,
                    rrule: rrule.0.to_string(),
                }))
            })?;
        Ok(rows.into_iter().flatten().collect())
    }

    /// A memory's own live content entries, in commit order — the per-stub read primitive that
    /// class-aware reads compose across a `same_as` class. Excludes superseded entries (a live
    /// surface, spec §Visibility); see [`Graph::entries_local_history`] for the unfiltered form, and
    /// [`Graph::class_entries`] for the traversing form. Not filtered by soft delete (a single-memory
    /// read; the class read carries the deleted filter).
    pub fn entries_local(&self, id: MemoryId) -> Result<Vec<EntryView>, GraphError> {
        self.collect_entries(
            "SELECT entry_id, asserted_at, occurred_sort, occurred_at, occurred_authored, text, told_by, told_in, visibility,
                    superseded_by, retracted_reason, origin_platform
             FROM content_entries WHERE memory_id = ?1 AND superseded_by IS NULL ORDER BY seq",
            id,
        )
    }

    /// As [`Graph::entries_local`], but including superseded entries — the per-stub history primitive
    /// for the surfaces where history is the point (`mem:history()`, the console), which deliberately
    /// bypass the live filter (spec §Visibility → superseded entries are not live).
    pub fn entries_local_history(&self, id: MemoryId) -> Result<Vec<EntryView>, GraphError> {
        self.collect_entries(
            "SELECT entry_id, asserted_at, occurred_sort, occurred_at, occurred_authored, text, told_by, told_in, visibility,
                    superseded_by, retracted_reason, origin_platform
             FROM content_entries WHERE memory_id = ?1 ORDER BY seq",
            id,
        )
    }

    /// The live content entries of `id`'s whole `same_as` class, in global commit order — the
    /// read-time traversal that surfaces a merged identity as one. For a singleton class this equals
    /// [`Graph::entries_local`]. Synthesis (description regeneration, belief arbitration) composes
    /// over this rather than a single stub, so a merged identity has one unified description instead
    /// of one per stub (spec §Visibility → synthesis traverses the `same_as` class). Entries of a
    /// soft-deleted member, and superseded entries, are excluded.
    pub fn class_entries(&self, id: MemoryId) -> Result<Vec<EntryView>, GraphError> {
        self.collect_entries(
            "SELECT entry_id, asserted_at, occurred_sort, occurred_at, occurred_authored, text, told_by, told_in, visibility,
                    superseded_by, retracted_reason, origin_platform
             FROM content_entries
             WHERE memory_id IN (
                 SELECT id FROM memories
                 WHERE class_id = (SELECT class_id FROM memories WHERE id = ?1 AND deleted = 0)
                   AND deleted = 0
             )
               AND superseded_by IS NULL
             ORDER BY seq",
            id,
        )
    }

    /// The recorded facts of the non-person memories this person *owns* — the [`Namespace::Event`]
    /// memories and the like reached by the links off their class — so a reader weighing a merge can see
    /// the specifics the agent filed on separate event memories, not only the facts written directly
    /// on the stub (spec §Cross-platform identity). Other [`Namespace::Person`]
    /// memories the person is merely linked to (a friend, a mentor) are excluded: those are someone
    /// else's facts, not this person's identity, and pulling them in would weigh a stranger's
    /// confidences in the wrong person's merge.
    /// Each linked memory contributes once, in link order.
    pub fn owned_context_entries(&self, id: MemoryId) -> Result<Vec<EntryView>, GraphError> {
        let members: BTreeSet<MemoryId> = self.class_members(id)?.into_iter().collect();
        let mut seen = BTreeSet::new();
        let mut entries = Vec::new();
        for link in self.class_links(id)? {
            if link.relation == RelationName::SameAs {
                continue;
            }
            // The endpoint that is not this person's own class is the linked memory.
            let other = if members.contains(&link.from) {
                link.to
            } else {
                link.from
            };
            if members.contains(&other) || !seen.insert(other) {
                continue;
            }
            let Some(memory) = self.memory_by_id(other)? else {
                continue;
            };
            if memory.name.namespaced().map(|n| n.namespace) == Ok(Namespace::Person) {
                continue;
            }
            entries.extend(self.class_entries(other)?);
        }
        Ok(entries)
    }

    /// As [`Graph::class_entries`], but including superseded entries — the class-wide history read
    /// for `mem:history()` and the console (spec §Visibility → superseded entries are not live).
    pub fn class_history(&self, id: MemoryId) -> Result<Vec<EntryView>, GraphError> {
        self.collect_entries_scoped(
            "SELECT entry_id, asserted_at, occurred_sort, occurred_at, occurred_authored, text, told_by, told_in, visibility,
                    superseded_by, retracted_reason, origin_platform
             FROM content_entries
             WHERE memory_id IN (
                 SELECT id FROM memories
                 WHERE class_id = (SELECT class_id FROM memories WHERE id = ?1 AND deleted = 0)
                   AND deleted = 0
             )
             ORDER BY seq",
            id,
            AttestationScope::WithWithdrawn,
        )
    }

    /// The entries of `id` currently under an unresolved belief arbitration — the facts the agent
    /// should surface as contested rather than assert as settled (spec §Write path → arbitration). An
    /// entry is disputed when it is a competing entry of the memory's latest arbitration, that
    /// arbitration credited neither side, and at least two of its competing entries are still live —
    /// so superseding one account (resolving the conflict) ends the dispute without a fresh
    /// arbitration. Keyed on the memory itself; a merged class's cross-stub disputes are out of scope
    /// until agent-driven merge lands.
    pub fn disputed_entries(&self, id: MemoryId) -> Result<BTreeSet<EntryId>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT ed.entry_id FROM entry_disputes ed
             JOIN content_entries ce ON ce.entry_id = ed.entry_id
             WHERE ed.memory_id = ?1 AND ce.superseded_by IS NULL",
        )?;
        let live: Vec<EntryId> = query_map_into(stmt, params![id.0.to_string()], |row| {
            let entry_id: String = row.get(0)?;
            Ok::<_, GraphError>(EntryId(parse_ulid(&entry_id)?))
        })?;
        Ok(if live.len() >= 2 {
            live.into_iter().collect()
        } else {
            BTreeSet::new()
        })
    }

    /// A single entry by id, with its live owning memory — or `None` if the entry is unknown or its
    /// memory is soft-deleted. The visibility predicate needs both: the entry's teller/visibility and
    /// the memory's subject. Used to resolve and filter an entry-vector search hit.
    pub fn entry_by_id(
        &self,
        entry_id: EntryId,
    ) -> Result<Option<(MemoryView, EntryView)>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT entry_id, memory_id, asserted_at, occurred_sort, occurred_at, occurred_authored,
                    text, told_by, told_in, visibility, superseded_by, retracted_reason,
                    origin_platform
             FROM content_entries WHERE entry_id = ?1",
        )?;
        let mapped = query_opt_into(stmt, params![entry_id.0.to_string()], |row| {
            let memory_id: String = row.get("memory_id")?;
            let entry = entry_from_row(row)?;
            Ok::<_, GraphError>((memory_id, entry))
        })?;
        let Some((memory_id, mut entry)) = mapped else {
            return Ok(None);
        };
        self.attach_attestations(std::slice::from_mut(&mut entry), AttestationScope::Live)?;
        Ok(self
            .memory_by_id(MemoryId(parse_ulid(&memory_id)?))?
            .map(|m| (m, entry)))
    }

    /// Run an entry query whose sole bound parameter is a memory id, mapping each row to an
    /// [`EntryView`] through [`entry_from_row`]. Shared by the live and history entry reads; each must
    /// select the columns [`entry_from_row`] reads.
    fn collect_entries(&self, sql: &str, id: MemoryId) -> Result<Vec<EntryView>, GraphError> {
        self.collect_entries_scoped(sql, id, AttestationScope::Live)
    }

    /// As [`Graph::collect_entries`], with the attestation scope explicit — the history read carries
    /// withdrawn attestations so the console can render them struck-through with their reasons, while
    /// every live read stays live-only.
    fn collect_entries_scoped(
        &self,
        sql: &str,
        id: MemoryId,
        scope: AttestationScope,
    ) -> Result<Vec<EntryView>, GraphError> {
        let stmt = self.conn.prepare(sql)?;
        let mut entries = query_map_into(stmt, params![id.0.to_string()], entry_from_row)?;
        self.attach_attestations(&mut entries, scope)?;
        Ok(entries)
    }

    /// Fill in each entry's live attestation set from `entry_attestations` in one batched query over
    /// the whole set — collect the entry ids, fetch every live attestation for them at once, and
    /// attach each entry its own (founding first, then by commit order). One query for the read rather
    /// than one per row, mirroring how the tag reads batch.
    fn attach_attestations(
        &self,
        entries: &mut [EntryView],
        scope: AttestationScope,
    ) -> Result<(), GraphError> {
        let ids: Vec<EntryId> = entries.iter().map(|entry| entry.entry_id).collect();
        let mut by_entry = self.attestations_for(&ids, scope)?;
        for entry in entries.iter_mut() {
            entry.attestations = by_entry.remove(&entry.entry_id).unwrap_or_default();
        }
        Ok(())
    }

    /// The live attestations of every entry in `ids`, keyed by entry id and ordered founding first
    /// then by commit order (`seq`). Only live attestations participate (`retracted_reason IS NULL`),
    /// mirroring the live entry reads; a whole-entry or per-teller retraction drops its attestations
    /// from this set. One query over the id set — the batched fetch the entry reads share.
    pub(super) fn attestations_for(
        &self,
        ids: &[EntryId],
        scope: AttestationScope,
    ) -> Result<HashMap<EntryId, Vec<AttestationView>>, GraphError> {
        let mut by_entry: HashMap<EntryId, Vec<AttestationView>> = HashMap::new();
        if ids.is_empty() {
            return Ok(by_entry);
        }
        let placeholders = vec!["?"; ids.len()].join(", ");
        let withdrawn_filter = match scope {
            AttestationScope::Live => " AND retracted_reason IS NULL",
            AttestationScope::WithWithdrawn => "",
        };
        let sql = format!(
            "SELECT entry_id, teller, told_in, asserted_at, posture, phrasing, source_entry, \
                    retracted_reason, seq
             FROM entry_attestations
             WHERE entry_id IN ({placeholders}){withdrawn_filter}
             ORDER BY entry_id, seq"
        );
        let stmt = self.conn.prepare(&sql)?;
        let rows: Vec<(String, AttestationView)> = query_map_into(
            stmt,
            params_from_iter(ids.iter().map(|id| id.0.to_string())),
            |row| {
                let entry_id: String = row.get("entry_id")?;
                Ok::<_, GraphError>((entry_id, attestation_from_row(row)?))
            },
        )?;
        for (entry_id, attestation) in rows {
            by_entry
                .entry(EntryId(parse_ulid(&entry_id)?))
                .or_default()
                .push(attestation);
        }
        Ok(by_entry)
    }

    /// The source entries consolidated into `replacement` by an `EntriesConsolidated` event, in
    /// commit order. Each source's `superseded_by` column points to the replacement (the same
    /// mechanism `MemorySuperseded` uses), so this reads the materialized graph rather than
    /// scanning the event log: it returns every tombstoned entry whose successor is `replacement`.
    /// A history read uses this to show the consolidation relationship.
    pub fn consolidation_sources(
        &self,
        replacement: EntryId,
    ) -> Result<Vec<EntryView>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT entry_id, asserted_at, occurred_sort, occurred_at, occurred_authored, text, told_by, told_in, visibility,
                    superseded_by, retracted_reason, origin_platform
             FROM content_entries
             WHERE superseded_by = ?1
             ORDER BY seq",
        )?;
        let mut entries = query_map_into(stmt, params![replacement.0.to_string()], entry_from_row)?;
        self.attach_attestations(&mut entries, AttestationScope::Live)?;
        Ok(entries)
    }
}

/// Decode one content-entry row into an [`EntryView`], deserializing the structured `told_by` /
/// `told_in` / `visibility` and parsing the `superseded_by` id. A free helper rather than a
/// `TryFrom` impl because the entry shape is genuinely shared across the entry, calendar, and
/// wake-up queries, all of which select these columns by these names.
pub(super) fn entry_from_row(row: &rusqlite::Row<'_>) -> Result<EntryView, GraphError> {
    let entry_id: String = row.get("entry_id")?;
    let told_by: String = row.get("told_by")?;
    let told_in: Option<String> = row.get("told_in")?;
    let visibility: String = row.get("visibility")?;
    let superseded_by: Option<String> = row.get("superseded_by")?;
    let occurred_at: Option<String> = row.get("occurred_at")?;
    let origin_platform: Option<String> = row.get("origin_platform")?;
    Ok(EntryView {
        entry_id: EntryId(parse_ulid(&entry_id)?),
        asserted_at: timestamp_column(row.get("asserted_at")?, "asserted_at")?,
        occurred_sort: row
            .get::<_, Option<i64>>("occurred_sort")?
            .map(|millis| timestamp_column(millis, "occurred_sort"))
            .transpose()?,
        occurred_at: occurred_at
            .map(|json| serde_json::from_str(&json))
            .transpose()?,
        occurred_authored: row.get("occurred_authored")?,
        text: row.get("text")?,
        told_by: serde_json::from_str(&told_by)?,
        told_in: told_in
            .map(|json| serde_json::from_str(&json))
            .transpose()?,
        visibility: serde_json::from_str(&visibility)?,
        superseded_by: superseded_by
            .map(|id| parse_ulid(&id).map(EntryId))
            .transpose()?,
        retracted_reason: row.get("retracted_reason")?,
        origin: match origin_platform {
            Some(platform) => EntryOrigin::PlatformConnector(platform),
            None => EntryOrigin::Recorded,
        },
        // Populated by the batched attestation fetch after the row decode (see
        // [`Graph::attach_attestations`]); a bare row read leaves it empty.
        attestations: Vec::new(),
    })
}

/// Decode one `entry_attestations` row into an [`AttestationView`], deserializing the structured
/// `teller` / `told_in` / `posture` and parsing the `source_entry` id. The batched attestation
/// fetch reads these columns by these names.
pub(super) fn attestation_from_row(row: &rusqlite::Row<'_>) -> Result<AttestationView, GraphError> {
    let teller: String = row.get("teller")?;
    let told_in: Option<String> = row.get("told_in")?;
    let posture: String = row.get("posture")?;
    let source_entry: Option<String> = row.get("source_entry")?;
    Ok(AttestationView {
        teller: serde_json::from_str(&teller)?,
        told_in: told_in
            .map(|json| serde_json::from_str(&json))
            .transpose()?,
        asserted_at: timestamp_column(row.get("asserted_at")?, "asserted_at")?,
        posture: serde_json::from_str(&posture)?,
        phrasing: row.get("phrasing")?,
        source_entry: source_entry
            .map(|id| parse_ulid(&id).map(EntryId))
            .transpose()?,
        retracted_reason: row.get("retracted_reason")?,
    })
}

pub(super) fn parse_cardinality(text: &str) -> Result<Cardinality, GraphError> {
    text.parse()
        .map_err(|()| GraphError::Malformed(format!("unknown cardinality {text:?}")))
}
