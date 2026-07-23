//! Occurrence queries: due items, recurring schedules, and pending wake-ups.

use crate::{
    db::query_map_into,
    graph::{
        EntryView, Graph, GraphError, MemoryColumns, MemoryView, entries::entry_from_row,
        parse_ulid, timestamp_column,
    },
    ids::{EntryId, MemoryId},
    time::{self, TemporalRef, Timestamp},
};
use rusqlite::params;
use std::collections::BTreeSet;

impl Graph {
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
                    e.entry_id, e.asserted_at, e.occurred_sort, e.occurred_at, e.occurred_authored,
                    e.text, e.told_by, e.told_in,
                    e.visibility, e.superseded_by, e.retracted_reason, e.origin_platform
             FROM content_entries e JOIN memories m ON m.id = e.memory_id
             WHERE m.deleted = 0 AND e.superseded_by IS NULL AND e.occurred_sort IS NOT NULL
               AND e.occurred_sort BETWEEN ?1 AND ?2
             ORDER BY e.occurred_sort, e.seq",
        )?;
        query_map_into(
            stmt,
            params![from.as_millisecond(), to.as_millisecond()],
            |row| self.occurrence_row(row),
        )
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
        query_map_into(stmt, params![now.as_millisecond()], |row| {
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
            let asserted_at = timestamp_column(asserted_at, "asserted_at")?;
            let baseline = match fired_at {
                Some(millis) => timestamp_column(millis, "fired_at")?,
                None => asserted_at,
            };
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
                    e.entry_id, e.asserted_at, e.occurred_sort, e.occurred_at, e.occurred_authored,
                    e.text, e.told_by, e.told_in,
                    e.visibility, e.superseded_by, e.retracted_reason, e.origin_platform
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
        let after = Timestamp::from_millis(from.as_millisecond().saturating_sub(1));
        let mut hits = Vec::new();
        for (columns, asserted_at, occurred_json) in rows {
            let Ok(TemporalRef::Recurring(rrule)) =
                serde_json::from_str::<TemporalRef>(&occurred_json)
            else {
                continue;
            };
            let asserted_at = timestamp_column(asserted_at, "asserted_at")?;
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
        let seed = Timestamp::from_millis(from.as_millisecond().saturating_sub(1));
        let mut hits = Vec::new();
        for (columns, asserted_at, occurred_json, text) in rows {
            let Ok(TemporalRef::Recurring(rrule)) =
                serde_json::from_str::<TemporalRef>(&occurred_json)
            else {
                continue;
            };
            let asserted_at = timestamp_column(asserted_at, "asserted_at")?;
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
}
