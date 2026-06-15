//! The materializer: folding committed events into the graph projection in `Seq` order. Dispatch is
//! on the payload's `(type, version)`; a wrong arm is a silent-leak class the eval harness backstops.

use std::collections::BTreeMap;

use rusqlite::params;

use super::{Graph, GraphError, backend};
use crate::{
    db::{query_map_into, query_opt_into},
    event::{Event, EventPayload, Visibility},
    ids::{MemoryId, MemoryName},
    time::{BEFORE_AFTER_EPSILON_MILLIS, OccurrenceBounds, TemporalRef, Timestamp},
    vocabulary::RelationName,
};

/// The denormalized occurrence values for one entry's `content_entries` row: the tagged-JSON
/// `occurred_at` and the `(sort, lo, hi)` millisecond bounds derived from it.
struct OccurrenceColumns {
    json: Option<String>,
    sort: Option<i64>,
    lo: Option<i64>,
    hi: Option<i64>,
}

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
            // The model-interaction record is log-only telemetry, read from the log rather than
            // projected (spec §Observability), and replay-inert by construction.
            EventPayload::ModelCalled { .. } => {}
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
            EventPayload::MemoryCreated { id, name } => {
                // A lone memory is its own class; a later same_as merge recomputes class_id.
                self.conn
                    .execute(
                        "INSERT INTO memories (id, name, created_at, class_id)
                         VALUES (?1, ?2, ?3, ?1)",
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
                occurred_at,
                text,
                told_by,
                told_in,
                visibility,
            } => {
                // Denormalize the typed `occurred_at` into sortable columns at materialization time
                // (spec §Time); see `occurrence_columns`.
                let occurrence = self.occurrence_columns(occurred_at.as_ref())?;
                self.conn
                    .execute(
                        "INSERT INTO content_entries \
                         (entry_id, memory_id, asserted_at, occurred_at, occurred_sort, \
                          occurred_lo, occurred_hi, text, told_by, told_in, visibility, seq)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                        params![
                            entry_id.0.to_string(),
                            id.0.to_string(),
                            asserted_at.as_millis(),
                            occurrence.json,
                            occurrence.sort,
                            occurrence.lo,
                            occurrence.hi,
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
            EventPayload::EntryTemporalResolved {
                entry_id,
                occurred_at,
                ..
            } => {
                // The extraction pass resolved this entry's occurrence after it was appended;
                // recompute its denormalized columns in place (text and FTS are untouched).
                let occurrence = self.occurrence_columns(Some(occurred_at))?;
                self.conn
                    .execute(
                        "UPDATE content_entries
                         SET occurred_at = ?1, occurred_sort = ?2, occurred_lo = ?3, occurred_hi = ?4
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
                        params![fired_at.as_millis(), entry_id.0.to_string()],
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
                        params![surfaced_at.as_millis(), entry_id.0.to_string()],
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
            EventPayload::ConversationStarted {
                id,
                locator,
                context_memory,
            } => {
                // Idempotent: the room is opened once; a re-seen locator is a no-op, not a duplicate.
                self.conn
                    .execute(
                        "INSERT OR IGNORE INTO conversations (id, platform, scope_path, context_memory)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![
                            id.0.to_string(),
                            locator.platform.as_str(),
                            locator.scope_path.as_str(),
                            context_memory.0.to_string(),
                        ],
                    )
                    .map_err(backend)?;
            }
            EventPayload::ConversationEnded { id } => {
                self.conn
                    .execute(
                        "UPDATE conversations SET ended = 1 WHERE id = ?1",
                        params![id.0.to_string()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::SessionStarted {
                conversation,
                id,
                participants,
                started_at,
                seeded_from_turn,
                brief,
            } => {
                self.conn
                    .execute(
                        "INSERT INTO sessions
                         (id, conversation, started_at, seeded_from_turn, brief, seq)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            id.0.to_string(),
                            conversation.0.to_string(),
                            started_at.as_millis(),
                            seeded_from_turn.map(|turn| turn.0.to_string()),
                            brief,
                            event.seq.0 as i64,
                        ],
                    )
                    .map_err(backend)?;
                // The present set at open carries no joining turn; a join records its `at_turn`.
                for participant in participants {
                    self.conn
                        .execute(
                            "INSERT OR IGNORE INTO session_participants (session, memory, at_turn)
                             VALUES (?1, ?2, NULL)",
                            params![id.0.to_string(), participant.0.to_string()],
                        )
                        .map_err(backend)?;
                }
            }
            EventPayload::SessionEnded { id, .. } => {
                self.conn
                    .execute(
                        "UPDATE sessions SET ended = 1 WHERE id = ?1",
                        params![id.0.to_string()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::ParticipantJoined {
                session,
                participant,
                at_turn,
                ..
            } => {
                self.conn
                    .execute(
                        "INSERT OR IGNORE INTO session_participants (session, memory, at_turn)
                         VALUES (?1, ?2, ?3)",
                        params![
                            session.0.to_string(),
                            participant.0.to_string(),
                            at_turn.0.to_string(),
                        ],
                    )
                    .map_err(backend)?;
            }
            EventPayload::ParticipantIdentified {
                memory,
                platform,
                platform_user_id,
            } => {
                self.conn
                    .execute(
                        "INSERT OR IGNORE INTO participant_identities
                         (platform, platform_user_id, memory) VALUES (?1, ?2, ?3)",
                        params![
                            platform.as_str(),
                            platform_user_id.as_str(),
                            memory.0.to_string(),
                        ],
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

    /// Denormalize an `occurred_at` reference into the values the `content_entries` occurrence
    /// columns store: the tagged JSON plus the `(sort, lo, hi)` millisecond bounds. A `BeforeAfter`
    /// resolves its anchor against the projection so far (`anchor_bounds`); every other variant is
    /// pure. Shared by the append and the `EntryTemporalResolved` arms so they denormalize identically.
    fn occurrence_columns(
        &self,
        occurred_at: Option<&TemporalRef>,
    ) -> Result<OccurrenceColumns, GraphError> {
        let bounds = match occurred_at {
            Some(reference) => {
                let anchor = match reference {
                    TemporalRef::BeforeAfter { anchor, .. } => self.anchor_bounds(anchor)?,
                    _ => None,
                };
                reference.bounds(anchor, BEFORE_AFTER_EPSILON_MILLIS)
            }
            None => OccurrenceBounds::default(),
        };
        Ok(OccurrenceColumns {
            json: occurred_at
                .map(serde_json::to_string)
                .transpose()
                .map_err(GraphError::Serialize)?,
            sort: bounds.sort.map(Timestamp::as_millis),
            lo: bounds.lo.map(Timestamp::as_millis),
            hi: bounds.hi.map(Timestamp::as_millis),
        })
    }

    /// The representative bounds of a `BeforeAfter` anchor, by name, for occurrence denormalization
    /// (spec §Time). Resolved from the entries already projected, taking the anchor's earliest timed
    /// entry. Deliberately **not** filtered by soft delete: `MemoryDeleted` preserves contents, so a
    /// deleted anchor's occurrence stays resolvable (spec §Known limitations → `BeforeAfter`). `None`
    /// when the anchor name is unknown or has no timed entry — the caller then derives empty bounds.
    fn anchor_bounds(&self, anchor: &MemoryName) -> Result<Option<OccurrenceBounds>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT e.occurred_sort, e.occurred_lo, e.occurred_hi
             FROM content_entries e JOIN memories m ON m.id = e.memory_id
             WHERE m.name = ?1 AND e.occurred_sort IS NOT NULL
             ORDER BY e.occurred_sort LIMIT 1",
        )?;
        query_opt_into(stmt, params![anchor.as_str()], |row| {
            let (sort, lo, hi): (Option<i64>, Option<i64>, Option<i64>) = row.try_into()?;
            Ok::<_, GraphError>(OccurrenceBounds {
                sort: sort.map(Timestamp::from_millis),
                lo: lo.map(Timestamp::from_millis),
                hi: hi.map(Timestamp::from_millis),
            })
        })
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

        let stmt = self
            .conn
            .prepare("SELECT name, symmetric FROM relations WHERE name = ?1 OR inverse = ?1")?;
        let resolved = query_opt_into(stmt, params![label], |row| {
            Ok::<(String, i64), GraphError>(row.try_into()?)
        })?;

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

    /// Recompute the denormalized `class_id` on every memory by union-find over the `same_as` edges,
    /// setting each class's id to its **earliest member by ULID** — the primary stub. Run on every
    /// `same_as` link change: a merge unions two classes, an unmerge re-splits the component, and a
    /// whole recompute is correct for both without a local patch (trivial at personal-agent class
    /// sizes). Operator-designated primaries are a later refinement.
    fn recompute_classes(&self) -> Result<(), GraphError> {
        let ids: Vec<String> =
            query_map_into(self.conn.prepare("SELECT id FROM memories")?, [], |row| {
                Ok::<_, GraphError>(row.get(0)?)
            })?;
        let edges: Vec<(String, String)> = query_map_into(
            self.conn
                .prepare("SELECT from_id, to_id FROM links WHERE relation = ?1")?,
            params![RelationName::SameAs.as_str()],
            |row| Ok::<(String, String), GraphError>(row.try_into()?),
        )?;

        let mut parent: BTreeMap<String, String> =
            ids.iter().map(|id| (id.clone(), id.clone())).collect();
        for (a, b) in &edges {
            let (ra, rb) = (find(&parent, a), find(&parent, b));
            if ra != rb {
                parent.insert(ra, rb);
            }
        }

        // Each component's class id is its earliest member by ULID (ULIDs sort chronologically).
        let mut primary: BTreeMap<String, String> = BTreeMap::new();
        for id in &ids {
            let root = find(&parent, id);
            let slot = primary.entry(root).or_insert_with(|| id.clone());
            if id < slot {
                *slot = id.clone();
            }
        }
        for id in &ids {
            self.conn
                .execute(
                    "UPDATE memories SET class_id = ?1 WHERE id = ?2",
                    params![primary[&find(&parent, id)], id],
                )
                .map_err(backend)?;
        }
        Ok(())
    }
}

/// Union-find root of `x`, following parent pointers (no path compression — classes are tiny).
fn find(parent: &BTreeMap<String, String>, x: &str) -> String {
    let mut cur = x.to_owned();
    while let Some(next) = parent.get(&cur) {
        if *next == cur {
            break;
        }
        cur = next.clone();
    }
    cur
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use super::Graph;
    use crate::{
        event::{ArbitrationResolution, Event, EventPayload, Teller, Visibility},
        ids::{EntryId, MemoryId, MemoryName, Seq},
        time::{BEFORE_AFTER_EPSILON_MILLIS, CivilDate, Direction, TemporalRef, Timestamp},
    };

    fn event(seq: u64, payload: EventPayload) -> Event {
        Event {
            seq: Seq(seq),
            recorded_at: Timestamp::from_millis(1),
            payload,
        }
    }

    /// The materializer must write all three denormalized columns from a single `TemporalRef`, in
    /// the right slots — `occurred_sort` alone (the only column a read-side view exposes today) can't
    /// catch a lo/hi column-order slip, so this asserts against the columns directly.
    #[test]
    fn occurrence_columns_match_the_derived_bounds() {
        let mut graph = Graph::open_in_memory().unwrap();
        let id = MemoryId::generate();
        let entry = EntryId::generate();
        let occurred = TemporalRef::Day(CivilDate("2026-06-03".into()));
        graph
            .apply(&event(
                1,
                EventPayload::MemoryCreated {
                    id,
                    name: MemoryName::new("event/cleaning"),
                },
            ))
            .unwrap();
        graph
            .apply(&event(
                2,
                EventPayload::MemoryContentAppended {
                    id,
                    entry_id: entry,
                    asserted_at: Timestamp::from_millis(1),
                    occurred_at: Some(occurred.clone()),
                    text: "scheduled cleaning".to_owned(),
                    told_by: Teller::Agent,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            ))
            .unwrap();

        let columns: (Option<i64>, Option<i64>, Option<i64>) = graph
            .conn
            .query_row(
                "SELECT occurred_sort, occurred_lo, occurred_hi
                 FROM content_entries WHERE entry_id = ?1",
                params![entry.0.to_string()],
                |r| r.try_into(),
            )
            .unwrap();
        let bounds = occurred.bounds(None, 0);
        assert_eq!(columns.0, bounds.sort.map(Timestamp::as_millis));
        assert_eq!(columns.1, bounds.lo.map(Timestamp::as_millis));
        assert_eq!(columns.2, bounds.hi.map(Timestamp::as_millis));
        assert!(columns.1 < columns.0 && columns.0 < columns.2);
    }

    /// `EntryTemporalResolved` updates an already-appended (untimed) entry's occurrence columns in
    /// place, resolving a `BeforeAfter` against the projection just like an explicit occurrence.
    #[test]
    fn entry_temporal_resolved_updates_columns_in_place() {
        let mut graph = Graph::open_in_memory().unwrap();
        let anchor = MemoryId::generate();
        let dependent = MemoryId::generate();
        let entry = EntryId::generate();
        let anchor_at = 1_000_000;
        let untimed = |id, entry_id| EventPayload::MemoryContentAppended {
            id,
            entry_id,
            asserted_at: Timestamp::from_millis(1),
            occurred_at: None,
            text: "fact".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        };
        let events = [
            EventPayload::MemoryCreated {
                id: anchor,
                name: MemoryName::new("event/wedding"),
            },
            EventPayload::MemoryContentAppended {
                id: anchor,
                entry_id: EntryId::generate(),
                asserted_at: Timestamp::from_millis(1),
                occurred_at: Some(TemporalRef::Instant(Timestamp::from_millis(anchor_at))),
                text: "the wedding".to_owned(),
                told_by: Teller::Agent,
                told_in: None,
                visibility: Visibility::Public,
            },
            EventPayload::MemoryCreated {
                id: dependent,
                name: MemoryName::new("event/reception"),
            },
            untimed(dependent, entry),
        ];
        for (seq, payload) in events.into_iter().enumerate() {
            graph.apply(&event(seq as u64 + 1, payload)).unwrap();
        }
        // The dependent entry starts untimed.
        let sort_before: Option<i64> = graph
            .conn
            .query_row(
                "SELECT occurred_sort FROM content_entries WHERE entry_id = ?1",
                rusqlite::params![entry.0.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sort_before, None);

        graph
            .apply(&event(
                5,
                EventPayload::EntryTemporalResolved {
                    id: dependent,
                    entry_id: entry,
                    occurred_at: TemporalRef::BeforeAfter {
                        dir: Direction::After,
                        anchor: MemoryName::new("event/wedding"),
                    },
                    produced_by: None,
                },
            ))
            .unwrap();
        let sort_after: Option<i64> = graph
            .conn
            .query_row(
                "SELECT occurred_sort FROM content_entries WHERE entry_id = ?1",
                rusqlite::params![entry.0.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sort_after, Some(anchor_at + BEFORE_AFTER_EPSILON_MILLIS));
    }

    /// An unresolved arbitration (crediting neither side) projects its competing entries as disputed;
    /// crediting a side clears them, superseding one account drops the dispute (the ≥2-live rule), and
    /// a fresh arbitration replaces the prior memory's state.
    #[test]
    fn disputed_entries_track_the_latest_unresolved_arbitration() {
        let mut graph = Graph::open_in_memory().unwrap();
        let memory = MemoryId::generate();
        let a = EntryId::generate();
        let b = EntryId::generate();
        let append = |seq, entry, text: &str| {
            event(
                seq,
                EventPayload::MemoryContentAppended {
                    id: memory,
                    entry_id: entry,
                    asserted_at: Timestamp::from_millis(1),
                    occurred_at: None,
                    text: text.to_owned(),
                    told_by: Teller::Agent,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            )
        };
        let arbitrate = |seq, credited: Vec<EntryId>| {
            event(
                seq,
                EventPayload::BeliefArbitrated {
                    memory,
                    competing_entries: vec![a, b],
                    resolution: ArbitrationResolution {
                        credited,
                        statement: "one says auditorium, another rooftop".to_owned(),
                    },
                    produced_by: None,
                },
            )
        };
        graph
            .apply(&event(
                1,
                EventPayload::MemoryCreated {
                    id: memory,
                    name: MemoryName::new("event/all-hands"),
                },
            ))
            .unwrap();
        graph.apply(&append(2, a, "in the auditorium")).unwrap();
        graph.apply(&append(3, b, "on the rooftop")).unwrap();

        // Unresolved: both competing entries are disputed.
        graph.apply(&arbitrate(4, vec![])).unwrap();
        assert_eq!(
            graph.disputed_entries(memory).unwrap(),
            [a, b].into_iter().collect()
        );

        // Crediting a side settles it: nothing disputed.
        graph.apply(&arbitrate(5, vec![a])).unwrap();
        assert!(graph.disputed_entries(memory).unwrap().is_empty());

        // Back to unresolved, then supersede one account — one live competitor is not a dispute.
        graph.apply(&arbitrate(6, vec![])).unwrap();
        let c = EntryId::generate();
        graph
            .apply(&append(7, c, "confirmed: the rooftop"))
            .unwrap();
        graph
            .apply(&event(
                8,
                EventPayload::MemorySuperseded {
                    id: memory,
                    entry: a,
                    superseded_by: c,
                },
            ))
            .unwrap();
        assert!(graph.disputed_entries(memory).unwrap().is_empty());
    }
}
