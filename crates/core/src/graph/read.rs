//! Read queries over the projection: memories, entries, tags, relations, links, and search. Every
//! agent-facing read filters soft-deleted memories.

use std::collections::BTreeSet;

use rusqlite::{OptionalExtension, params};

use super::{
    ClassLinkView, EntryView, Graph, GraphError, LinkView, MemoryView, OpenSessionView,
    RelationView, SessionView, TagVocabularyEntry, backend, parse_ulid,
};
use crate::{
    db::{query_map_into, query_opt_into},
    event::{Cardinality, LinkSource, Teller, Volatility},
    ids::{
        ConversationId, ConversationLocator, EntryId, MemoryId, MemoryName, Namespace, Seq,
        SessionId, TurnId,
    },
    time::{self, TemporalRef, Timestamp},
    vocabulary::{RelationName, TagName},
};

impl Graph {
    /// Fetch a live (non-deleted) memory by its agent-facing name.
    pub fn memory_by_name(&self, name: &str) -> Result<Option<MemoryView>, GraphError> {
        self.fetch_memory("name", name)
    }

    /// Fetch a live (non-deleted) memory by its internal id.
    pub fn memory_by_id(&self, id: MemoryId) -> Result<Option<MemoryView>, GraphError> {
        self.fetch_memory("id", &id.0.to_string())
    }

    /// The names a memory used to go by, most recent first — the aliases a rename left behind, so a
    /// read can label a renamed memory ("person/sarah, formerly person/dave") and the agent connects
    /// its older, old-name content to the same person (spec §Identity → Renaming). Empty for a memory
    /// that was never renamed.
    pub fn former_names(&self, id: MemoryId) -> Result<Vec<MemoryName>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT former_name FROM memory_aliases WHERE memory_id = ?1 ORDER BY rowid DESC",
        )?;
        query_map_into(stmt, params![id.0.to_string()], |row| {
            Ok(MemoryName::new(row.get::<_, String>(0)?))
        })
    }

    /// Resolve a *former* name to the live memory that now holds it under a different handle — the
    /// alias fallback behind a renamed person being found by an old name (spec §Identity → Renaming).
    /// Only consulted after a current-name lookup misses, so a current name always wins; returns `None`
    /// if no memory shed this name, or the one that did has since been deleted.
    pub fn memory_id_for_former_name(&self, name: &str) -> Result<Option<MemoryId>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT a.memory_id FROM memory_aliases a
             JOIN memories m ON m.id = a.memory_id
             WHERE a.former_name = ?1 AND m.deleted = 0",
        )?;
        let id: Option<String> = query_opt_into(stmt, params![name], |row| {
            Ok::<String, GraphError>(row.get(0)?)
        })?;
        id.map(|id| Ok(MemoryId(parse_ulid(&id)?))).transpose()
    }

    /// The `same_as`-class id of `id` (its class's primary stub), or `None` if the memory is unknown
    /// or soft-deleted. A lone memory is its own class. The denormalized identity key for presence
    /// and membership tests.
    pub fn class_id(&self, id: MemoryId) -> Result<Option<MemoryId>, GraphError> {
        let class: Option<String> = self
            .conn
            .query_row(
                "SELECT class_id FROM memories WHERE id = ?1 AND deleted = 0",
                params![id.0.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        class.map(|id| parse_ulid(&id).map(MemoryId)).transpose()
    }

    /// The live members of `id`'s `same_as` class (including `id`), ordered by id. Empty if the
    /// memory is unknown or soft-deleted.
    pub fn class_members(&self, id: MemoryId) -> Result<Vec<MemoryId>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT id FROM memories
             WHERE class_id = (SELECT class_id FROM memories WHERE id = ?1 AND deleted = 0)
               AND deleted = 0
             ORDER BY id",
        )?;
        query_map_into(stmt, params![id.0.to_string()], |row| {
            let id: String = row.get(0)?;
            Ok::<_, GraphError>(MemoryId(parse_ulid(&id)?))
        })
    }

    /// All live memories whose name begins with `prefix` (e.g. `"person/"`), ordered by name.
    pub fn memories_in_namespace(&self, prefix: &str) -> Result<Vec<MemoryView>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT id, name, description, volatility, created_at FROM memories
             WHERE name LIKE ?1 || '%' AND deleted = 0 ORDER BY name",
        )?;
        query_map_into(stmt, params![prefix], |row| {
            self.assemble_memory(row.try_into()?)
        })
    }

    /// Live memories with a concrete occurrence in `[from, to]`, each paired with the matching entry,
    /// ordered soonest first — the calendar-as-view query (spec §Calendar). Only entries with a
    /// denormalized `occurred_sort` (instant/day/range/approx) participate; a `Recurring` entry has a
    /// null sort and is found via [`Graph::recurring_memories`] instead. A memory with several
    /// occurrences in the window appears once per occurrence.
    pub fn occurrences_in_window(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<(MemoryView, EntryView)>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT m.id, m.name, m.description, m.volatility, m.created_at,
                    e.entry_id, e.asserted_at, e.occurred_sort, e.occurred_at, e.text, e.told_by,
                    e.told_in,
                    e.visibility, e.superseded_by
             FROM content_entries e JOIN memories m ON m.id = e.memory_id
             WHERE m.deleted = 0 AND e.superseded_by IS NULL AND e.occurred_sort IS NOT NULL
               AND e.occurred_sort BETWEEN ?1 AND ?2
             ORDER BY e.occurred_sort, e.seq",
        )?;
        query_map_into(stmt, params![from.as_millis(), to.as_millis()], |row| {
            self.occurrence_row(row)
        })
    }

    /// Live entries whose scheduled occurrence has come due but not yet fired — the scheduler's input
    /// (spec §Scheduled work). The comes-due rule: a concrete `occurred_sort` that has passed `now` and
    /// was later than the entry's own `asserted_at`, so an event scheduled for the future fires while a
    /// past event recorded after the fact never does. Recurring entries (null sort) are excluded.
    pub fn due_occurrences(&self, now: Timestamp) -> Result<Vec<(MemoryId, EntryId)>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT e.memory_id, e.entry_id
             FROM content_entries e JOIN memories m ON m.id = e.memory_id
             WHERE m.deleted = 0 AND e.superseded_by IS NULL AND e.fired_at IS NULL
               AND e.occurred_sort IS NOT NULL
               AND e.occurred_sort > e.asserted_at
               AND e.occurred_sort <= ?1
             ORDER BY e.occurred_sort, e.seq",
        )?;
        query_map_into(stmt, params![now.as_millis()], |row| {
            let memory: String = row.get("memory_id")?;
            let entry: String = row.get("entry_id")?;
            Ok::<_, GraphError>((MemoryId(parse_ulid(&memory)?), EntryId(parse_ulid(&entry)?)))
        })
    }

    /// Recurring entries whose next instance has come due by `now` — the recurring half of the wake-up
    /// scheduler (spec §Recurring materialization and wake-up arming), the complement to
    /// [`Graph::due_occurrences`], which handles only concrete occurrences. For each live recurring
    /// entry, the next instance (anchored at `asserted_at`, since the rrule carries no `DTSTART`) is
    /// computed strictly after its last firing — `fired_at`, or `asserted_at` if it has never fired —
    /// and the entry is due when that instance is at or before `now`. Each firing re-arms it: the next
    /// call computes the instance after the firing just recorded, so exactly one trigger is live per
    /// recurring entry, never a backlog. A rule `next_occurrence` cannot interpret simply never fires.
    pub fn due_recurring(&self, now: Timestamp) -> Result<Vec<(MemoryId, EntryId)>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT e.memory_id, e.entry_id, e.asserted_at, e.fired_at, e.occurred_at
             FROM content_entries e JOIN memories m ON m.id = e.memory_id
             WHERE m.deleted = 0 AND e.superseded_by IS NULL
               AND e.occurred_sort IS NULL AND e.occurred_at IS NOT NULL
             ORDER BY e.seq",
        )?;
        let rows: Vec<(String, String, i64, Option<i64>, String)> =
            query_map_into(stmt, [], |row| {
                Ok::<_, GraphError>((
                    row.get("memory_id")?,
                    row.get("entry_id")?,
                    row.get("asserted_at")?,
                    row.get("fired_at")?,
                    row.get("occurred_at")?,
                ))
            })?;

        let mut due = Vec::new();
        for (memory, entry, asserted_at, fired_at, occurred_json) in rows {
            // An unresolved `BeforeAfter` is also sort-null; keep only true recurrences.
            let Ok(TemporalRef::Recurring(rrule)) =
                serde_json::from_str::<TemporalRef>(&occurred_json)
            else {
                continue;
            };
            let asserted_at = Timestamp::from_millis(asserted_at);
            let baseline = fired_at.map_or(asserted_at, Timestamp::from_millis);
            if let Some(instant) = time::next_occurrence(&rrule, asserted_at, baseline)
                && instant <= now
            {
                due.push((MemoryId(parse_ulid(&memory)?), EntryId(parse_ulid(&entry)?)));
            }
        }
        Ok(due)
    }

    /// Live entries that have fired but are not yet surfaced — the wake-up surface the drain consumes
    /// (spec §Agent-initiated speech), each paired with its memory, soonest occurrence first.
    pub fn pending_wakeups(&self) -> Result<Vec<(MemoryView, EntryView)>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT m.id, m.name, m.description, m.volatility, m.created_at,
                    e.entry_id, e.asserted_at, e.occurred_sort, e.occurred_at, e.text, e.told_by,
                    e.told_in,
                    e.visibility, e.superseded_by
             FROM content_entries e JOIN memories m ON m.id = e.memory_id
             WHERE m.deleted = 0 AND e.superseded_by IS NULL
               AND e.fired_at IS NOT NULL AND e.surfaced_at IS NULL
             ORDER BY e.occurred_sort, e.seq",
        )?;
        query_map_into(stmt, [], |row| self.occurrence_row(row))
    }

    /// Live memories that carry a `Recurring` occurrence — the `calendar.recurring()` listing. These
    /// have a null `occurred_sort`, so they never appear in [`Graph::occurrences_in_window`]; this
    /// parses the stored `occurred_at` to keep only true recurrences (an unresolved `BeforeAfter` is
    /// also sort-null). Instances are not expanded here (spec §Known limitations).
    pub fn recurring_memories(&self) -> Result<Vec<MemoryView>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT m.id, m.name, m.description, m.volatility, m.created_at, e.occurred_at
             FROM content_entries e JOIN memories m ON m.id = e.memory_id
             WHERE m.deleted = 0 AND e.superseded_by IS NULL
               AND e.occurred_sort IS NULL AND e.occurred_at IS NOT NULL
             ORDER BY m.name",
        )?;
        let rows: Vec<(MemoryColumns, String)> = query_map_into(stmt, [], |row| {
            let columns = (
                row.get("id")?,
                row.get("name")?,
                row.get("description")?,
                row.get("volatility")?,
                row.get("created_at")?,
            );
            Ok::<_, GraphError>((columns, row.get("occurred_at")?))
        })?;

        // Dedup by memory before assembling, so an entry's tags are fetched once per memory.
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for (memory_columns, occurred_json) in rows {
            if !matches!(
                serde_json::from_str::<TemporalRef>(&occurred_json),
                Ok(TemporalRef::Recurring(_))
            ) {
                continue;
            }
            if seen.insert(memory_columns.0.clone()) {
                out.push(self.assemble_memory(memory_columns)?);
            }
        }
        Ok(out)
    }

    /// Live recurring memories whose next instance falls within `[from, to]`, each paired with that
    /// instance and ordered soonest first — the recurring complement to
    /// [`Graph::occurrences_in_window`], so `calendar.upcoming`/`calendar.on` surface a weekly standup
    /// the same way they surface a one-off (spec §Recurring materialization). The instance is the
    /// earliest occurrence at or after `from` (anchored at `asserted_at`); a memory appears once, at
    /// its soonest in-window instance, even if it carries several recurring entries. A rule
    /// `next_occurrence` cannot interpret is skipped, as is an unresolved `BeforeAfter` (also sort-null).
    pub fn recurring_in_window(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<(Timestamp, MemoryView)>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT m.id, m.name, m.description, m.volatility, m.created_at, e.asserted_at,
                    e.occurred_at
             FROM content_entries e JOIN memories m ON m.id = e.memory_id
             WHERE m.deleted = 0 AND e.superseded_by IS NULL
               AND e.occurred_sort IS NULL AND e.occurred_at IS NOT NULL
             ORDER BY e.seq",
        )?;
        let rows: Vec<(MemoryColumns, i64, String)> = query_map_into(stmt, [], |row| {
            let columns = (
                row.get("id")?,
                row.get("name")?,
                row.get("description")?,
                row.get("volatility")?,
                row.get("created_at")?,
            );
            Ok::<_, GraphError>((columns, row.get("asserted_at")?, row.get("occurred_at")?))
        })?;

        // `from - 1` as the "strictly after" bound so an instance landing exactly on `from` counts.
        let after = Timestamp::from_millis(from.as_millis().saturating_sub(1));
        let mut hits = Vec::new();
        for (columns, asserted_at, occurred_json) in rows {
            let Ok(TemporalRef::Recurring(rrule)) =
                serde_json::from_str::<TemporalRef>(&occurred_json)
            else {
                continue;
            };
            let asserted_at = Timestamp::from_millis(asserted_at);
            if let Some(instant) = time::next_occurrence(&rrule, asserted_at, after)
                && instant >= from
                && instant <= to
            {
                hits.push((instant, columns));
            }
        }

        // Soonest first, then one row per memory (its earliest in-window instance).
        hits.sort_by_key(|(instant, _)| *instant);
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for (instant, columns) in hits {
            if seen.insert(columns.0.clone()) {
                out.push((instant, self.assemble_memory(columns)?));
            }
        }
        Ok(out)
    }

    /// Every instance of each live recurring entry within `[from, to]` (up to `max_per_entry` per
    /// entry), each paired with its memory and the entry's text, ordered soonest first — the
    /// console's calendar *expansion*. Distinct from [`Graph::recurring_in_window`], which collapses
    /// to a memory's single next instance for the agent's `calendar.upcoming`; here a weekly standup
    /// yields a row for each of the coming weeks. Instances anchor at `asserted_at` (the rrule carries
    /// no `DTSTART`) and run through the same `next_occurrence`, so the expansion cannot drift from
    /// the agent's scheduling. A rule `next_occurrence` cannot interpret yields no instances.
    pub fn recurring_instances_in_window(
        &self,
        from: Timestamp,
        to: Timestamp,
        max_per_entry: usize,
    ) -> Result<Vec<(Timestamp, MemoryView, String)>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT m.id, m.name, m.description, m.volatility, m.created_at, e.asserted_at,
                    e.occurred_at, e.text
             FROM content_entries e JOIN memories m ON m.id = e.memory_id
             WHERE m.deleted = 0 AND e.superseded_by IS NULL
               AND e.occurred_sort IS NULL AND e.occurred_at IS NOT NULL
             ORDER BY e.seq",
        )?;
        let rows: Vec<(MemoryColumns, i64, String, String)> = query_map_into(stmt, [], |row| {
            let columns = (
                row.get("id")?,
                row.get("name")?,
                row.get("description")?,
                row.get("volatility")?,
                row.get("created_at")?,
            );
            Ok::<_, GraphError>((
                columns,
                row.get("asserted_at")?,
                row.get("occurred_at")?,
                row.get("text")?,
            ))
        })?;

        // `from - 1` as the "strictly after" seed so an instance landing exactly on `from` counts.
        let seed = Timestamp::from_millis(from.as_millis().saturating_sub(1));
        let mut hits = Vec::new();
        for (columns, asserted_at, occurred_json, text) in rows {
            let Ok(TemporalRef::Recurring(rrule)) =
                serde_json::from_str::<TemporalRef>(&occurred_json)
            else {
                continue;
            };
            let asserted_at = Timestamp::from_millis(asserted_at);
            let memory = self.assemble_memory(columns)?;
            let mut after = seed;
            for _ in 0..max_per_entry {
                let Some(instant) = time::next_occurrence(&rrule, asserted_at, after) else {
                    break;
                };
                if instant > to {
                    break;
                }
                hits.push((instant, memory.clone(), text.clone()));
                after = instant;
            }
        }
        hits.sort_by_key(|(instant, _, _)| *instant);
        Ok(hits)
    }

    /// A memory's own live content entries, in commit order — the per-stub read primitive that
    /// class-aware reads compose across a `same_as` class. Excludes superseded entries (a live
    /// surface, spec §Visibility); see [`Graph::entries_local_history`] for the unfiltered form, and
    /// [`Graph::class_entries`] for the traversing form. Not filtered by soft delete (a single-memory
    /// read; the class read carries the deleted filter).
    pub fn entries_local(&self, id: MemoryId) -> Result<Vec<EntryView>, GraphError> {
        self.collect_entries(
            "SELECT entry_id, asserted_at, occurred_sort, occurred_at, text, told_by, told_in, visibility,
                    superseded_by
             FROM content_entries WHERE memory_id = ?1 AND superseded_by IS NULL ORDER BY seq",
            id,
        )
    }

    /// As [`Graph::entries_local`], but including superseded entries — the per-stub history primitive
    /// for the surfaces where history is the point (`mem:history()`, the console), which deliberately
    /// bypass the live filter (spec §Visibility → superseded entries are not live).
    pub fn entries_local_history(&self, id: MemoryId) -> Result<Vec<EntryView>, GraphError> {
        self.collect_entries(
            "SELECT entry_id, asserted_at, occurred_sort, occurred_at, text, told_by, told_in, visibility,
                    superseded_by
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
            "SELECT entry_id, asserted_at, occurred_sort, occurred_at, text, told_by, told_in, visibility,
                    superseded_by
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

    /// The recorded facts of the non-person memories this person *owns* — the `event/`s and the like
    /// reached by the links off their class — so a merge adjudication can weigh the specifics the agent
    /// filed on separate event memories, not only the facts written directly on the stub (spec
    /// §Cross-platform identity → adjudicated merge). Other `person/` memories the person is merely
    /// linked to (a friend, a mentor) are excluded: those are someone else's facts, not this person's
    /// identity, and pulling them in would weigh a stranger's confidences in the wrong person's merge.
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
            if Namespace::Person.contains(memory.name.as_str()) {
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
            "SELECT entry_id, asserted_at, occurred_sort, occurred_at, text, told_by, told_in, visibility,
                    superseded_by
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
            "SELECT entry_id, memory_id, asserted_at, occurred_sort, occurred_at, text, told_by,
                    told_in, visibility, superseded_by
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

    /// A tag's description, or `None` if the tag was never created.
    pub fn tag_description(&self, name: &str) -> Result<Option<String>, GraphError> {
        self.conn
            .query_row(
                "SELECT description FROM tags WHERE name = ?1",
                params![name],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)
    }

    /// The whole tag vocabulary: every created tag with its one-line purpose and how many live
    /// memories carry it, ordered by name. Backs `tags.list` and the system prompt's tag-vocabulary
    /// block. The count joins only undeleted memories, so a tag applied solely to soft-deleted
    /// memories reads as zero uses, consistent with every other agent-facing read.
    pub fn all_tags(&self) -> Result<Vec<TagVocabularyEntry>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT t.name, t.description, COUNT(m.id) AS count
             FROM tags t
             LEFT JOIN memory_tags mt ON mt.tag = t.name
             LEFT JOIN memories m ON m.id = mt.memory_id AND m.deleted = 0
             GROUP BY t.name, t.description
             ORDER BY t.name",
        )?;
        query_map_into(stmt, [], |row| {
            let name: String = row.get("name")?;
            let description: String = row.get("description")?;
            let count: i64 = row.get("count")?;
            Ok(TagVocabularyEntry {
                name: TagName::new(name),
                description,
                count: count as usize,
            })
        })
    }

    /// A registered relation by either of its labels (canonical or inverse), or `None`. Resolving the
    /// inverse label too is what lets a relation be used under either name (spec §Data model: one
    /// relation, two labels) — both at `links.get` and when validating a `mem:link` asserted under the
    /// inverse label.
    pub fn relation(&self, name: &str) -> Result<Option<RelationView>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT name, inverse, from_card, to_card, symmetric, reflexive
             FROM relations WHERE name = ?1 OR inverse = ?1",
        )?;
        query_opt_into(stmt, params![name], |row| {
            let name: String = row.get("name")?;
            let inverse: String = row.get("inverse")?;
            let from_card: String = row.get("from_card")?;
            let to_card: String = row.get("to_card")?;
            let symmetric: i64 = row.get("symmetric")?;
            let reflexive: i64 = row.get("reflexive")?;
            Ok(RelationView {
                name: RelationName::new(name),
                inverse: RelationName::new(inverse),
                from_card: parse_cardinality(&from_card)?,
                to_card: parse_cardinality(&to_card)?,
                symmetric: symmetric != 0,
                reflexive: reflexive != 0,
            })
        })
    }

    /// Every registered relation, ordered by canonical name. Backs `links.list` and the system
    /// prompt's relation-registry block.
    pub fn all_relations(&self) -> Result<Vec<RelationView>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT name, inverse, from_card, to_card, symmetric, reflexive
             FROM relations ORDER BY name",
        )?;
        query_map_into(stmt, [], |row| {
            let name: String = row.get("name")?;
            let inverse: String = row.get("inverse")?;
            let from_card: String = row.get("from_card")?;
            let to_card: String = row.get("to_card")?;
            let symmetric: i64 = row.get("symmetric")?;
            let reflexive: i64 = row.get("reflexive")?;
            Ok(RelationView {
                name: RelationName::new(name),
                inverse: RelationName::new(inverse),
                from_card: parse_cardinality(&from_card)?,
                to_card: parse_cardinality(&to_card)?,
                symmetric: symmetric != 0,
                reflexive: reflexive != 0,
            })
        })
    }

    /// Live neighbours reachable from `id` under `relation` (given as either label). Resolves the
    /// label through the registry, follows the canonical edge in the right direction (both
    /// directions for a symmetric relation), and skips soft-deleted neighbours.
    pub fn outgoing(&self, id: MemoryId, relation: &str) -> Result<Vec<MemoryView>, GraphError> {
        let stmt = self
            .conn
            .prepare("SELECT name, symmetric FROM relations WHERE name = ?1 OR inverse = ?1")?;
        let resolved = query_opt_into(stmt, params![relation], |row| {
            Ok::<(String, i64), GraphError>(row.try_into()?)
        })?;
        let Some((canonical, symmetric)) = resolved else {
            return Ok(Vec::new());
        };

        let id = id.0.to_string();
        let neighbour_ids = if symmetric != 0 {
            self.query_ids(
                "SELECT to_id FROM links WHERE from_id = ?1 AND relation = ?2
                 UNION SELECT from_id FROM links WHERE to_id = ?1 AND relation = ?2",
                &id,
                &canonical,
            )?
        } else if relation == canonical {
            self.query_ids(
                "SELECT to_id FROM links WHERE from_id = ?1 AND relation = ?2",
                &id,
                &canonical,
            )?
        } else {
            self.query_ids(
                "SELECT from_id FROM links WHERE to_id = ?1 AND relation = ?2",
                &id,
                &canonical,
            )?
        };

        let mut neighbours = Vec::new();
        for neighbour in neighbour_ids {
            if let Some(memory) = self.memory_by_id(MemoryId(parse_ulid(&neighbour)?))? {
                neighbours.push(memory);
            }
        }
        Ok(neighbours)
    }

    /// All canonical edges touching `id`, with both endpoints live. For inspection and tests; the
    /// agent-facing oriented view is [`Graph::outgoing`].
    pub fn links(&self, id: MemoryId) -> Result<Vec<LinkView>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT l.from_id, l.to_id, l.relation FROM links l
             JOIN memories mf ON mf.id = l.from_id
             JOIN memories mt ON mt.id = l.to_id
             WHERE (l.from_id = ?1 OR l.to_id = ?1) AND mf.deleted = 0 AND mt.deleted = 0
             ORDER BY l.relation, l.to_id",
        )?;
        query_map_into(stmt, params![id.0.to_string()], |row| {
            let (from, to, relation): (String, String, String) = row.try_into()?;
            Ok(LinkView {
                from: MemoryId(parse_ulid(&from)?),
                to: MemoryId(parse_ulid(&to)?),
                relation: RelationName::new(relation),
            })
        })
    }

    /// Every canonical edge touching `id`'s whole `same_as` class, with both endpoints live and the
    /// edge's `source` carried for provenance — the class-traversing read behind the agent-facing
    /// `mem:outgoing`/`incoming`/`links` link readers (spec §Lua API → link readers). Includes edges
    /// internal to the class (both endpoints class members); the block layer drops those, since a
    /// relationship the agent cares about points *out* of the identity, not the `same_as` plumbing
    /// holding it together.
    pub fn class_links(&self, id: MemoryId) -> Result<Vec<ClassLinkView>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT l.from_id, l.to_id, l.relation, l.source, l.told_by FROM links l
             JOIN memories mf ON mf.id = l.from_id
             JOIN memories mt ON mt.id = l.to_id
             WHERE mf.deleted = 0 AND mt.deleted = 0
               AND (l.from_id IN (
                       SELECT id FROM memories
                       WHERE class_id = (SELECT class_id FROM memories WHERE id = ?1 AND deleted = 0)
                         AND deleted = 0)
                 OR l.to_id IN (
                       SELECT id FROM memories
                       WHERE class_id = (SELECT class_id FROM memories WHERE id = ?1 AND deleted = 0)
                         AND deleted = 0))
             ORDER BY l.relation, l.to_id",
        )?;
        query_map_into(stmt, params![id.0.to_string()], |row| {
            let from: String = row.get("from_id")?;
            let to: String = row.get("to_id")?;
            let relation: String = row.get("relation")?;
            let source: String = row.get("source")?;
            let told_by: Option<String> = row.get("told_by")?;
            Ok(ClassLinkView {
                from: MemoryId(parse_ulid(&from)?),
                to: MemoryId(parse_ulid(&to)?),
                relation: RelationName::new(relation),
                source: LinkSource::parse(&source).ok_or_else(|| {
                    GraphError::Malformed(format!("unknown link source {source:?}"))
                })?,
                told_by: told_by
                    .map(|json| serde_json::from_str(&json))
                    .transpose()?,
            })
        })
    }

    /// Resolve a conversation's locator to its id, or `None` if the room has never been seen. A
    /// retired (ended) conversation still resolves — the room is durable.
    pub fn conversation_for_locator(
        &self,
        locator: &ConversationLocator,
    ) -> Result<Option<ConversationId>, GraphError> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM conversations WHERE platform = ?1 AND scope_path = ?2",
                params![locator.platform.as_str(), locator.scope_path.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        id.map(|id| parse_ulid(&id).map(ConversationId)).transpose()
    }

    /// Resolve a platform participant `(platform, platform_user_id)` to its `person/*` stub, or
    /// `None` if that identity has never been seen.
    pub fn participant_for(
        &self,
        platform: &str,
        platform_user_id: &str,
    ) -> Result<Option<MemoryId>, GraphError> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT memory FROM participant_identities
                 WHERE platform = ?1 AND platform_user_id = ?2",
                params![platform, platform_user_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        id.map(|id| parse_ulid(&id).map(MemoryId)).transpose()
    }

    /// The `context/*` memory minted with a conversation, or `None` if the conversation is unknown.
    /// The locator resolves to the room and thence to its context (spec §Contexts are first-class).
    pub fn context_for_conversation(
        &self,
        conversation: ConversationId,
    ) -> Result<Option<MemoryId>, GraphError> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT context_memory FROM conversations WHERE id = ?1",
                params![conversation.0.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        id.map(|id| parse_ulid(&id).map(MemoryId)).transpose()
    }

    /// Resolve a teller to the display name a marker shows (a participant's handle, or a fixed label
    /// for the agent and genesis). Shared by search and brief composition.
    pub fn teller_display(&self, teller: &Teller) -> Result<String, GraphError> {
        Ok(match teller {
            Teller::Participant(id) => self
                .memory_by_id(*id)?
                .map(|memory| memory.name.as_str().to_owned())
                .unwrap_or_else(|| "someone".to_owned()),
            Teller::Agent => "the agent".to_owned(),
            Teller::Bootstrap => "genesis".to_owned(),
        })
    }

    /// Resolve a `told_in` context to its marker room — display name and `#confidential` flag — for
    /// the teller-private marker. `None` when the entry carries no room, or its context memory is
    /// gone. Shared by search and brief composition, both of which bake the marker at build time
    /// (spec §Visibility → marker).
    pub fn marker_room(
        &self,
        told_in: Option<MemoryId>,
    ) -> Result<Option<crate::visibility::MarkerRoom>, GraphError> {
        let Some(context_id) = told_in else {
            return Ok(None);
        };
        Ok(self
            .memory_by_id(context_id)?
            .map(|context| crate::visibility::MarkerRoom {
                name: crate::visibility::room_display(context.name.as_str()),
                confidential: context.tags.contains(&TagName::Confidential),
            }))
    }

    /// A session by id, with its participants, or `None` if unknown.
    pub fn session(&self, id: SessionId) -> Result<Option<SessionView>, GraphError> {
        let stmt = self.session_stmt("WHERE id = ?1")?;
        query_opt_into(stmt, params![id.0.to_string()], |row| {
            self.assemble_session(row)
        })
    }

    /// A conversation's sessions, oldest first (commit order).
    pub fn sessions_in(
        &self,
        conversation: ConversationId,
    ) -> Result<Vec<SessionView>, GraphError> {
        let stmt = self.session_stmt("WHERE conversation = ?1 ORDER BY seq")?;
        query_map_into(stmt, params![conversation.0.to_string()], |row| {
            self.assemble_session(row)
        })
    }

    /// The most recent unclosed session of a conversation — the live one a restart must recover — or
    /// `None` if every session has ended. The in-memory session map is process-local, so on boot this
    /// is how a session still open in the log (left by a restart, or a passive graceful exit) is found
    /// again, to resume within the idle gap or close-with-flush past it.
    pub fn last_open_session(
        &self,
        conversation: ConversationId,
    ) -> Result<Option<OpenSessionView>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT id, brief, started_at, seq, seeded_from_turn FROM sessions
             WHERE conversation = ?1 AND ended = 0 ORDER BY seq DESC LIMIT 1",
        )?;
        query_opt_into(stmt, params![conversation.0.to_string()], |row| {
            let id: String = row.get("id")?;
            let seeded: Option<String> = row.get("seeded_from_turn")?;
            Ok::<_, GraphError>(OpenSessionView {
                id: SessionId(parse_ulid(&id)?),
                brief: row.get("brief")?,
                started_at: Timestamp::from_millis(row.get("started_at")?),
                start_seq: Seq(row.get::<_, i64>("seq")? as u64),
                seeded: seeded.is_some(),
            })
        })
    }

    /// Prepare a `sessions` read over the columns [`Graph::assemble_session`] decodes, with `clause`
    /// supplying the differing `WHERE` (and any `ORDER BY`). Sharing the column list keeps the by-id
    /// and by-conversation reads provably returning the same row shape. `clause` is a static fragment,
    /// never agent input.
    fn session_stmt(&self, clause: &str) -> Result<rusqlite::Statement<'_>, GraphError> {
        Ok(self.conn.prepare(&format!(
            "SELECT id, conversation, started_at, seeded_from_turn, brief FROM sessions {clause}"
        ))?)
    }

    /// A session's participants — the present set at open plus anyone who joined — ordered by id.
    pub fn session_participants(&self, session: SessionId) -> Result<Vec<MemoryId>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT memory FROM session_participants WHERE session = ?1 ORDER BY memory",
        )?;
        query_map_into(stmt, params![session.0.to_string()], |row| {
            let memory: String = row.get(0)?;
            Ok(MemoryId(parse_ulid(&memory)?))
        })
    }

    /// Full-text search over name, description, and content, best match first. Over-fetches and
    /// filters soft-deleted memories, mirroring how visibility-aware search will filter hits later.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryView>, GraphError> {
        let match_query = build_match(query);
        if match_query.is_empty() {
            return Ok(Vec::new());
        }
        let over_fetch = limit.saturating_mul(4).max(limit + 10) as i64;
        let stmt = self.conn.prepare(
            "SELECT memory_id FROM memories_fts WHERE memories_fts MATCH ?1
             ORDER BY rank LIMIT ?2",
        )?;
        let ids: Vec<MemoryId> = query_map_into(stmt, params![match_query, over_fetch], |row| {
            let id: String = row.get(0)?;
            Ok::<_, GraphError>(MemoryId(parse_ulid(&id)?))
        })?;

        let mut hits = Vec::new();
        for id in ids {
            if let Some(memory) = self.memory_by_id(id)? {
                hits.push(memory);
                if hits.len() >= limit {
                    break;
                }
            }
        }
        Ok(hits)
    }

    /// Lexical hits with their raw FTS5 bm25 score (more negative is a better match), for the
    /// multi-signal ranker to normalize and blend. Deleted memories are excluded.
    pub fn search_lexical(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(MemoryId, f32)>, GraphError> {
        let match_query = build_match(query);
        if match_query.is_empty() {
            return Ok(Vec::new());
        }
        let stmt = self.conn.prepare(
            "SELECT f.memory_id, bm25(memories_fts) AS score
             FROM memories_fts f JOIN memories m ON m.id = f.memory_id
             WHERE memories_fts MATCH ?1 AND m.deleted = 0
             ORDER BY score LIMIT ?2",
        )?;
        query_map_into(stmt, params![match_query, limit as i64], |row| {
            let (id, score): (String, f64) = row.try_into()?;
            Ok((MemoryId(parse_ulid(&id)?), score as f32))
        })
    }

    fn fetch_memory(&self, column: &str, value: &str) -> Result<Option<MemoryView>, GraphError> {
        let sql = format!(
            "SELECT id, name, description, volatility, created_at FROM memories
             WHERE {column} = ?1 AND deleted = 0"
        );
        let stmt = self.conn.prepare(&sql)?;
        query_opt_into(stmt, params![value], |row| {
            self.assemble_memory(row.try_into()?)
        })
    }

    fn assemble_memory(&self, columns: MemoryColumns) -> Result<MemoryView, GraphError> {
        let (id, name, description, volatility, created_at) = columns;
        Ok(MemoryView {
            id: MemoryId(parse_ulid(&id)?),
            name: MemoryName::new(name),
            description,
            volatility: Volatility::parse(&volatility).ok_or_else(|| {
                GraphError::Malformed(format!("unknown volatility {volatility:?}"))
            })?,
            created_at: Timestamp::from_millis(created_at),
            tags: self.tags_of(&id)?,
        })
    }

    /// Decode the `(memory, entry)` row shared by the calendar and wake-up queries: the memory columns
    /// (`assemble_memory`) and the entry columns ([`entry_from_row`]) selected together
    /// ([`Graph::occurrences_in_window`], [`Graph::pending_wakeups`]).
    fn occurrence_row(
        &self,
        row: &rusqlite::Row<'_>,
    ) -> Result<(MemoryView, EntryView), GraphError> {
        let id: String = row.get("id")?;
        let name: String = row.get("name")?;
        let description: String = row.get("description")?;
        let volatility: String = row.get("volatility")?;
        let created_at: i64 = row.get("created_at")?;
        let memory = self.assemble_memory((id, name, description, volatility, created_at))?;
        let entry = entry_from_row(row)?;
        Ok((memory, entry))
    }

    /// Build a [`SessionView`] from a row selecting the columns [`Graph::session_stmt`] lists, then
    /// load its participants. Decoding from the row here keeps the column list and its reader together.
    fn assemble_session(&self, row: &rusqlite::Row<'_>) -> Result<SessionView, GraphError> {
        let id: String = row.get("id")?;
        let conversation: String = row.get("conversation")?;
        let seeded_from_turn: Option<String> = row.get("seeded_from_turn")?;
        let id = SessionId(parse_ulid(&id)?);
        Ok(SessionView {
            id,
            conversation: ConversationId(parse_ulid(&conversation)?),
            started_at: Timestamp::from_millis(row.get("started_at")?),
            seeded_from_turn: seeded_from_turn
                .map(|turn| parse_ulid(&turn).map(TurnId))
                .transpose()?,
            brief: row.get("brief")?,
            participants: self.session_participants(id)?,
        })
    }

    fn tags_of(&self, memory_id: &str) -> Result<Vec<TagName>, GraphError> {
        let stmt = self
            .conn
            .prepare("SELECT tag FROM memory_tags WHERE memory_id = ?1 ORDER BY tag")?;
        query_map_into(stmt, params![memory_id], |row| {
            let tag: String = row.get(0)?;
            Ok(TagName::new(tag))
        })
    }

    /// Run an entry query whose sole bound parameter is a memory id, mapping each row to an
    /// [`EntryView`] through [`entry_from_row`]. Shared by the live and history entry reads; each must
    /// select the columns [`entry_from_row`] reads.
    fn collect_entries(&self, sql: &str, id: MemoryId) -> Result<Vec<EntryView>, GraphError> {
        let stmt = self.conn.prepare(sql)?;
        query_map_into(stmt, params![id.0.to_string()], entry_from_row)
    }

    fn query_ids(&self, sql: &str, id: &str, relation: &str) -> Result<Vec<String>, GraphError> {
        let stmt = self.conn.prepare(sql)?;
        query_map_into(stmt, params![id, relation], |row| Ok(row.get(0)?))
    }
}

/// The raw memory columns the `memories` SELECT yields; consumed by [`Graph::assemble_memory`].
type MemoryColumns = (String, String, String, String, i64);

/// Decode one content-entry row into an [`EntryView`], deserializing the structured `told_by` /
/// `visibility` and parsing the `told_in` / `superseded_by` ids. A free helper rather than a
/// `TryFrom` impl because the entry shape is genuinely shared across the entry, calendar, and
/// wake-up queries, all of which select these columns by these names.
fn entry_from_row(row: &rusqlite::Row<'_>) -> Result<EntryView, GraphError> {
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
        text: row.get("text")?,
        told_by: serde_json::from_str(&told_by)?,
        told_in: told_in
            .map(|id| parse_ulid(&id).map(MemoryId))
            .transpose()?,
        visibility: serde_json::from_str(&visibility)?,
        superseded_by: superseded_by
            .map(|id| parse_ulid(&id).map(EntryId))
            .transpose()?,
    })
}

fn parse_cardinality(text: &str) -> Result<Cardinality, GraphError> {
    Cardinality::parse(text)
        .ok_or_else(|| GraphError::Malformed(format!("unknown cardinality {text:?}")))
}

/// Build an FTS5 MATCH expression from free text: each whitespace-separated term becomes a quoted
/// phrase (with embedded quotes doubled), joined as an implicit AND. Empty input yields an empty
/// string, which the caller treats as "no query".
fn build_match(query: &str) -> String {
    query
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}
