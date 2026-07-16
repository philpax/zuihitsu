//! Content entry reads: live, history, class-wide, disputed, and by-id.

use crate::{
    db::{query_map_into, query_opt_into},
    event::Cardinality,
    graph::{EntryView, Graph, GraphError, MemoryView, parse_ulid},
    ids::{EntryId, MemoryId, Namespace},
    time::Timestamp,
    vocabulary::RelationName,
};
use rusqlite::params;
use std::collections::BTreeSet;

impl Graph {
    /// A memory's own live content entries, in commit order — the per-stub read primitive that
    /// class-aware reads compose across a `same_as` class. Excludes superseded entries (a live
    /// surface, spec §Visibility); see [`Graph::entries_local_history`] for the unfiltered form, and
    /// [`Graph::class_entries`] for the traversing form. Not filtered by soft delete (a single-memory
    /// read; the class read carries the deleted filter).
    pub fn entries_local(&self, id: MemoryId) -> Result<Vec<EntryView>, GraphError> {
        self.collect_entries(
            "SELECT entry_id, asserted_at, occurred_sort, occurred_at, occurred_authored, text, told_by, told_in, visibility,
                    superseded_by, retracted_reason
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
                    superseded_by, retracted_reason
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
                    superseded_by, retracted_reason
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
    /// memories and the like reached by the links off their class — so a merge adjudication can weigh
    /// the specifics the agent filed on separate event memories, not only the facts written directly
    /// on the stub (spec §Cross-platform identity → adjudicated merge). Other [`Namespace::Person`]
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
        self.collect_entries(
            "SELECT entry_id, asserted_at, occurred_sort, occurred_at, occurred_authored, text, told_by, told_in, visibility,
                    superseded_by, retracted_reason
             FROM content_entries
             WHERE memory_id IN (
                 SELECT id FROM memories
                 WHERE class_id = (SELECT class_id FROM memories WHERE id = ?1 AND deleted = 0)
                   AND deleted = 0
             )
             ORDER BY seq",
            id,
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
                    text, told_by, told_in, visibility, superseded_by, retracted_reason
             FROM content_entries WHERE entry_id = ?1",
        )?;
        let mapped = query_opt_into(stmt, params![entry_id.0.to_string()], |row| {
            let memory_id: String = row.get("memory_id")?;
            let entry = entry_from_row(row)?;
            Ok::<_, GraphError>((memory_id, entry))
        })?;
        let Some((memory_id, entry)) = mapped else {
            return Ok(None);
        };
        Ok(self
            .memory_by_id(MemoryId(parse_ulid(&memory_id)?))?
            .map(|m| (m, entry)))
    }

    /// Run an entry query whose sole bound parameter is a memory id, mapping each row to an
    /// [`EntryView`] through [`entry_from_row`]. Shared by the live and history entry reads; each must
    /// select the columns [`entry_from_row`] reads.
    fn collect_entries(&self, sql: &str, id: MemoryId) -> Result<Vec<EntryView>, GraphError> {
        let stmt = self.conn.prepare(sql)?;
        query_map_into(stmt, params![id.0.to_string()], entry_from_row)
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
    Ok(EntryView {
        entry_id: EntryId(parse_ulid(&entry_id)?),
        asserted_at: Timestamp::from_millis(row.get("asserted_at")?),
        occurred_sort: row
            .get::<_, Option<i64>>("occurred_sort")?
            .map(Timestamp::from_millis),
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
    })
}

pub(super) fn parse_cardinality(text: &str) -> Result<Cardinality, GraphError> {
    text.parse()
        .map_err(|()| GraphError::Malformed(format!("unknown cardinality {text:?}")))
}
