//! Read queries over the projection: memories, entries, tags, relations, links, and search. Every
//! agent-facing read filters soft-deleted memories.

use std::collections::BTreeSet;

use rusqlite::{OptionalExtension, params};

use super::{
    EntryView, Graph, GraphError, LinkView, MemoryView, RelationView, SessionView, backend,
    parse_ulid,
};
use crate::{
    event::{Cardinality, Teller, Volatility},
    ids::{
        ConversationId, ConversationLocator, EntryId, MemoryId, MemoryName, RelationName,
        SessionId, TagName, Timestamp, TurnId,
    },
    time::TemporalRef,
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
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id FROM memories
                 WHERE class_id = (SELECT class_id FROM memories WHERE id = ?1 AND deleted = 0)
                   AND deleted = 0
                 ORDER BY id",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![id.0.to_string()], |r| r.get::<_, String>(0))
            .map_err(backend)?;
        let mut members = Vec::new();
        for row in rows {
            members.push(MemoryId(parse_ulid(&row.map_err(backend)?)?));
        }
        Ok(members)
    }

    /// All live memories whose name begins with `prefix` (e.g. `"person/"`), ordered by name.
    pub fn memories_in_namespace(&self, prefix: &str) -> Result<Vec<MemoryView>, GraphError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, description, volatility, created_at FROM memories
                 WHERE name LIKE ?1 || '%' AND deleted = 0 ORDER BY name",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![prefix], row_to_memory_columns)
            .map_err(backend)?;

        let mut memories = Vec::new();
        for row in rows {
            memories.push(self.assemble_memory(row.map_err(backend)?)?);
        }
        Ok(memories)
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
        let mut stmt = self
            .conn
            .prepare(
                "SELECT m.id, m.name, m.description, m.volatility, m.created_at,
                        e.entry_id, e.asserted_at, e.occurred_sort, e.text, e.told_by, e.told_in,
                        e.visibility
                 FROM content_entries e JOIN memories m ON m.id = e.memory_id
                 WHERE m.deleted = 0 AND e.occurred_sort IS NOT NULL
                   AND e.occurred_sort BETWEEN ?1 AND ?2
                 ORDER BY e.occurred_sort, e.seq",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![from.as_millis(), to.as_millis()], |r| {
                Ok((
                    (
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                    ),
                    (
                        r.get::<_, String>(5)?,
                        r.get::<_, i64>(6)?,
                        r.get::<_, Option<i64>>(7)?,
                        r.get::<_, String>(8)?,
                        r.get::<_, String>(9)?,
                        r.get::<_, Option<String>>(10)?,
                        r.get::<_, String>(11)?,
                    ),
                ))
            })
            .map_err(backend)?;

        let mut out = Vec::new();
        for row in rows {
            let (
                memory_columns,
                (entry_id, asserted_at, occurred_sort, text, told_by, told_in, visibility),
            ) = row.map_err(backend)?;
            let memory = self.assemble_memory(memory_columns)?;
            let entry = EntryView::from_db(
                EntryId(parse_ulid(&entry_id)?),
                asserted_at,
                occurred_sort,
                text,
                &told_by,
                told_in.as_deref(),
                &visibility,
            )?;
            out.push((memory, entry));
        }
        Ok(out)
    }

    /// Live memories that carry a `Recurring` occurrence — the `calendar.recurring()` listing. These
    /// have a null `occurred_sort`, so they never appear in [`Graph::occurrences_in_window`]; this
    /// parses the stored `occurred_at` to keep only true recurrences (an unresolved `BeforeAfter` is
    /// also sort-null). Instances are not expanded here (spec §Known limitations).
    pub fn recurring_memories(&self) -> Result<Vec<MemoryView>, GraphError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT m.id, m.name, m.description, m.volatility, m.created_at, e.occurred_at
                 FROM content_entries e JOIN memories m ON m.id = e.memory_id
                 WHERE m.deleted = 0 AND e.occurred_sort IS NULL AND e.occurred_at IS NOT NULL
                 ORDER BY m.name",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    (
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                    ),
                    r.get::<_, String>(5)?,
                ))
            })
            .map_err(backend)?;

        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for row in rows {
            let (memory_columns, occurred_json) = row.map_err(backend)?;
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

    /// A memory's own content entries, in commit order — the per-stub read primitive that
    /// class-aware reads compose across a `same_as` class. Low-level: not filtered by soft delete.
    /// See [`Graph::class_entries`] for the traversing form.
    pub fn entries_local(&self, id: MemoryId) -> Result<Vec<EntryView>, GraphError> {
        self.collect_entries(
            "SELECT entry_id, asserted_at, occurred_sort, text, told_by, told_in, visibility
             FROM content_entries WHERE memory_id = ?1 ORDER BY seq",
            id,
        )
    }

    /// The content entries of `id`'s whole live `same_as` class, in global commit order — the
    /// read-time traversal that surfaces a merged identity as one. For a singleton class this equals
    /// [`Graph::entries_local`]. Synthesis (description regeneration, belief arbitration) composes
    /// over this rather than a single stub, so a merged identity has one unified description instead
    /// of one per stub (spec §Visibility → synthesis traverses the `same_as` class). Entries of a
    /// soft-deleted member are excluded.
    pub fn class_entries(&self, id: MemoryId) -> Result<Vec<EntryView>, GraphError> {
        self.collect_entries(
            "SELECT entry_id, asserted_at, occurred_sort, text, told_by, told_in, visibility
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

    /// A single entry by id, with its live owning memory — or `None` if the entry is unknown or its
    /// memory is soft-deleted. The visibility predicate needs both: the entry's teller/visibility and
    /// the memory's subject. Used to resolve and filter an entry-vector search hit.
    pub fn entry_by_id(
        &self,
        entry_id: EntryId,
    ) -> Result<Option<(MemoryView, EntryView)>, GraphError> {
        let row = self
            .conn
            .query_row(
                "SELECT memory_id, asserted_at, occurred_sort, text, told_by, told_in, visibility
                 FROM content_entries WHERE entry_id = ?1",
                params![entry_id.0.to_string()],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, String>(6)?,
                    ))
                },
            )
            .optional()
            .map_err(backend)?;
        let Some((memory_id, asserted_at, occurred_sort, text, told_by, told_in, visibility)) = row
        else {
            return Ok(None);
        };
        let Some(memory) = self.memory_by_id(MemoryId(parse_ulid(&memory_id)?))? else {
            return Ok(None);
        };
        let entry = EntryView::from_db(
            entry_id,
            asserted_at,
            occurred_sort,
            text,
            &told_by,
            told_in.as_deref(),
            &visibility,
        )?;
        Ok(Some((memory, entry)))
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

    /// A registered relation by its canonical name, or `None`.
    pub fn relation(&self, name: &str) -> Result<Option<RelationView>, GraphError> {
        self.conn
            .query_row(
                "SELECT name, inverse, from_card, to_card, symmetric, reflexive
                 FROM relations WHERE name = ?1",
                params![name],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(backend)?
            .map(
                |(name, inverse, from_card, to_card, symmetric, reflexive)| {
                    Ok(RelationView {
                        name: RelationName::new(name),
                        inverse: RelationName::new(inverse),
                        from_card: parse_cardinality(&from_card)?,
                        to_card: parse_cardinality(&to_card)?,
                        symmetric: symmetric != 0,
                        reflexive: reflexive != 0,
                    })
                },
            )
            .transpose()
    }

    /// Live neighbours reachable from `id` under `relation` (given as either label). Resolves the
    /// label through the registry, follows the canonical edge in the right direction (both
    /// directions for a symmetric relation), and skips soft-deleted neighbours.
    pub fn outgoing(&self, id: MemoryId, relation: &str) -> Result<Vec<MemoryView>, GraphError> {
        let resolved: Option<(String, i64)> = self
            .conn
            .query_row(
                "SELECT name, symmetric FROM relations WHERE name = ?1 OR inverse = ?1",
                params![relation],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(backend)?;
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
        let mut stmt = self
            .conn
            .prepare(
                "SELECT l.from_id, l.to_id, l.relation FROM links l
                 JOIN memories mf ON mf.id = l.from_id
                 JOIN memories mt ON mt.id = l.to_id
                 WHERE (l.from_id = ?1 OR l.to_id = ?1) AND mf.deleted = 0 AND mt.deleted = 0
                 ORDER BY l.relation, l.to_id",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![id.0.to_string()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(backend)?;

        let mut links = Vec::new();
        for row in rows {
            let (from, to, relation) = row.map_err(backend)?;
            links.push(LinkView {
                from: MemoryId(parse_ulid(&from)?),
                to: MemoryId(parse_ulid(&to)?),
                relation: RelationName::new(relation),
            });
        }
        Ok(links)
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
        let row = self
            .conn
            .query_row(
                "SELECT id, conversation, started_at, seeded_from_turn, brief
                 FROM sessions WHERE id = ?1",
                params![id.0.to_string()],
                session_columns,
            )
            .optional()
            .map_err(backend)?;
        row.map(|columns| self.assemble_session(columns))
            .transpose()
    }

    /// A conversation's sessions, oldest first (commit order).
    pub fn sessions_in(
        &self,
        conversation: ConversationId,
    ) -> Result<Vec<SessionView>, GraphError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, conversation, started_at, seeded_from_turn, brief
                 FROM sessions WHERE conversation = ?1 ORDER BY seq",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![conversation.0.to_string()], session_columns)
            .map_err(backend)?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(self.assemble_session(row.map_err(backend)?)?);
        }
        Ok(sessions)
    }

    /// A session's participants — the present set at open plus anyone who joined — ordered by id.
    pub fn session_participants(&self, session: SessionId) -> Result<Vec<MemoryId>, GraphError> {
        let mut stmt = self
            .conn
            .prepare("SELECT memory FROM session_participants WHERE session = ?1 ORDER BY memory")
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![session.0.to_string()], |r| r.get::<_, String>(0))
            .map_err(backend)?;
        let mut participants = Vec::new();
        for row in rows {
            participants.push(MemoryId(parse_ulid(&row.map_err(backend)?)?));
        }
        Ok(participants)
    }

    /// Full-text search over name, description, and content, best match first. Over-fetches and
    /// filters soft-deleted memories, mirroring how visibility-aware search will filter hits later.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryView>, GraphError> {
        let match_query = build_match(query);
        if match_query.is_empty() {
            return Ok(Vec::new());
        }
        let over_fetch = limit.saturating_mul(4).max(limit + 10) as i64;
        let mut stmt = self
            .conn
            .prepare(
                "SELECT memory_id FROM memories_fts WHERE memories_fts MATCH ?1
                 ORDER BY rank LIMIT ?2",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![match_query, over_fetch], |r| r.get::<_, String>(0))
            .map_err(backend)?;

        let mut hits = Vec::new();
        for row in rows {
            let id = MemoryId(parse_ulid(&row.map_err(backend)?)?);
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
        let mut stmt = self
            .conn
            .prepare(
                "SELECT f.memory_id, bm25(memories_fts) AS score
                 FROM memories_fts f JOIN memories m ON m.id = f.memory_id
                 WHERE memories_fts MATCH ?1 AND m.deleted = 0
                 ORDER BY score LIMIT ?2",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![match_query, limit as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
            })
            .map_err(backend)?;

        let mut hits = Vec::new();
        for row in rows {
            let (id, score) = row.map_err(backend)?;
            hits.push((MemoryId(parse_ulid(&id)?), score as f32));
        }
        Ok(hits)
    }

    fn fetch_memory(&self, column: &str, value: &str) -> Result<Option<MemoryView>, GraphError> {
        let sql = format!(
            "SELECT id, name, description, volatility, created_at FROM memories
             WHERE {column} = ?1 AND deleted = 0"
        );
        let row = self
            .conn
            .query_row(&sql, params![value], row_to_memory_columns)
            .optional()
            .map_err(backend)?;
        match row {
            Some(columns) => Ok(Some(self.assemble_memory(columns)?)),
            None => Ok(None),
        }
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

    fn assemble_session(&self, columns: SessionColumns) -> Result<SessionView, GraphError> {
        let (id, conversation, started_at, seeded_from_turn, brief) = columns;
        let id = SessionId(parse_ulid(&id)?);
        Ok(SessionView {
            id,
            conversation: ConversationId(parse_ulid(&conversation)?),
            started_at: Timestamp::from_millis(started_at),
            seeded_from_turn: seeded_from_turn
                .map(|turn| parse_ulid(&turn).map(TurnId))
                .transpose()?,
            brief,
            participants: self.session_participants(id)?,
        })
    }

    fn tags_of(&self, memory_id: &str) -> Result<Vec<TagName>, GraphError> {
        let mut stmt = self
            .conn
            .prepare("SELECT tag FROM memory_tags WHERE memory_id = ?1 ORDER BY tag")
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![memory_id], |r| r.get::<_, String>(0))
            .map_err(backend)?;
        let mut tags = Vec::new();
        for row in rows {
            tags.push(TagName::new(row.map_err(backend)?));
        }
        Ok(tags)
    }

    /// Run an entry query whose sole bound parameter is a memory id, mapping each row to an
    /// [`EntryView`]. Shared by [`Graph::entries_local`] and [`Graph::class_entries`].
    fn collect_entries(&self, sql: &str, id: MemoryId) -> Result<Vec<EntryView>, GraphError> {
        let mut stmt = self.conn.prepare(sql).map_err(backend)?;
        let rows = stmt
            .query_map(params![id.0.to_string()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })
            .map_err(backend)?;

        let mut entries = Vec::new();
        for row in rows {
            let (entry_id, asserted_at, occurred_sort, text, told_by, told_in, visibility) =
                row.map_err(backend)?;
            entries.push(EntryView::from_db(
                EntryId(parse_ulid(&entry_id)?),
                asserted_at,
                occurred_sort,
                text,
                &told_by,
                told_in.as_deref(),
                &visibility,
            )?);
        }
        Ok(entries)
    }

    fn query_ids(&self, sql: &str, id: &str, relation: &str) -> Result<Vec<String>, GraphError> {
        let mut stmt = self.conn.prepare(sql).map_err(backend)?;
        let rows = stmt
            .query_map(params![id, relation], |r| r.get::<_, String>(0))
            .map_err(backend)?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.map_err(backend)?);
        }
        Ok(ids)
    }
}

/// The raw memory columns, shared by the single- and multi-row read paths.
type MemoryColumns = (String, String, String, String, i64);

fn row_to_memory_columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryColumns> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}

/// The raw session columns (id, conversation, started_at, seeded_from_turn, brief), shared by the
/// single- and multi-row read paths.
type SessionColumns = (String, String, i64, Option<String>, String);

fn session_columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionColumns> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
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
