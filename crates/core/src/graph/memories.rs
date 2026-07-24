//! Memory lookups: by name, id, former name, and the same-identity (`same_as`) class.

use crate::{
    db::{query_map_into, query_opt_into},
    graph::{Graph, GraphError, MemoryView, backend, parse_ulid},
    ids::{MemoryId, MemoryName},
};
use rusqlite::{OptionalExtension, params};

impl Graph {
    /// Fetch a live (non-deleted) memory by its agent-facing name. Takes any handle — a `MemoryName`, a
    /// `NamespacedMemoryName`, or a boundary string — so callers pass a typed handle rather than its
    /// `&str`; the conversion to the bare string the SQL needs happens once, here.
    pub fn memory_by_name(
        &self,
        name: impl Into<MemoryName>,
    ) -> Result<Option<MemoryView>, GraphError> {
        self.fetch_memory("name", name.into().as_str())
    }

    /// The agent's `self` memory, or `None` before genesis seeds it — the reserved self-model handle
    /// looked up by name. A first-class read because the brief, the system prompt, and the write guard
    /// all reach for it.
    pub fn self_memory(&self) -> Result<Option<MemoryView>, GraphError> {
        self.memory_by_name(MemoryName::self_handle())
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
    pub fn memory_id_for_former_name(
        &self,
        name: impl Into<MemoryName>,
    ) -> Result<Option<MemoryId>, GraphError> {
        let name = name.into();
        let stmt = self.conn.prepare(
            "SELECT a.memory_id FROM memory_aliases a
             JOIN memories m ON m.id = a.memory_id
             WHERE a.former_name = ?1 AND m.deleted = 0",
        )?;
        let id: Option<String> = query_opt_into(stmt, params![name.as_str()], |row| {
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

    /// Whether the operator has pinned `id` as its `same_as` class's primary (spec §Cross-platform
    /// identity). `false` for an unpinned, unknown, or soft-deleted memory. The pin lives on the stub
    /// regardless of whether it currently wins its class, so a console can mark a designation the
    /// earliest-ULID rule would otherwise mask.
    pub fn is_primary_designated(&self, id: MemoryId) -> Result<bool, GraphError> {
        let designated: Option<i64> = self
            .conn
            .query_row(
                "SELECT designated_primary FROM memories WHERE id = ?1 AND deleted = 0",
                params![id.0.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        Ok(designated == Some(1))
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

    /// The ids of every live memory whose name begins with `prefix`, ordered by name — the read
    /// behind `memory.list`, the handle-discovery-by-stem lookup. The prefix is matched *literally*:
    /// its LIKE metacharacters (`%`, `_`, `\`) are escaped and matched with an explicit `ESCAPE`
    /// clause, so a stem like `person/dav_` matches a literal underscore rather than wildcarding any
    /// character. Returns ids only — no per-memory tag subquery — so listing a broad stem stays cheap;
    /// the caller caps the result and mints the handles.
    pub fn memory_ids_with_name_prefix(&self, prefix: &str) -> Result<Vec<MemoryId>, GraphError> {
        let pattern = format!("{}%", escape_like(prefix));
        let stmt = self.conn.prepare(
            "SELECT id FROM memories WHERE name LIKE ?1 ESCAPE '\\' AND deleted = 0 ORDER BY name",
        )?;
        query_map_into(stmt, params![pattern], |row| {
            let id: String = row.get(0)?;
            Ok::<_, GraphError>(MemoryId(parse_ulid(&id)?))
        })
    }

    /// Every live memory id that begins with `prefix` (a full id matches itself), ordered by id — the
    /// resolution primitive the operator's offline identity commands (`debug designate-primary`,
    /// `debug merge`) use to turn a typed id or unique prefix into exactly one memory, erroring when the
    /// prefix is ambiguous. Matched case-insensitively against the stored uppercase ULID, so an operator
    /// may paste either casing. A soft-deleted memory is excluded, so a stale id resolves to nothing
    /// rather than a hidden memory.
    pub fn memory_ids_with_prefix(&self, prefix: &str) -> Result<Vec<MemoryId>, GraphError> {
        let stmt = self.conn.prepare(
            "SELECT id FROM memories WHERE id LIKE ?1 || '%' AND deleted = 0 ORDER BY id",
        )?;
        let ids: Vec<String> = query_map_into(stmt, params![prefix.to_uppercase()], |row| {
            Ok::<_, GraphError>(row.get("id")?)
        })?;
        ids.iter().map(|id| Ok(MemoryId(parse_ulid(id)?))).collect()
    }

    /// The count of live (non-deleted) memories — the agent's knowledge footprint, surfaced as a
    /// gauge. A `COUNT(*)` rather than materializing the memories, so a metrics scrape stays cheap
    /// on a large graph.
    pub fn memory_count(&self) -> Result<u64, GraphError> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE deleted = 0",
            [],
            |row| row.get::<_, i64>(0),
        )? as u64)
    }

    /// The count of live (non-superseded) content entries — how much has been written into the
    /// graph, the growth signal a runaway writer would surface first.
    pub fn entry_count(&self) -> Result<u64, GraphError> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM content_entries WHERE superseded_by IS NULL",
            [],
            |row| row.get::<_, i64>(0),
        )? as u64)
    }

    /// The count of links in the graph.
    pub fn link_count(&self) -> Result<u64, GraphError> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM links", [], |row| row.get::<_, i64>(0))?
            as u64)
    }

    /// The names of every live memory whose name begins with `prefix`, ordered by name — the
    /// narrow candidate fetch behind the name-collision suggestions, so a collision on a graph with
    /// thousands of memories ranks a first-character slice rather than the whole namespace. The
    /// prefix is matched as an index range scan (`name >= prefix AND name < successor`) on the
    /// unique `name` index, not with LIKE: the column's BINARY collation means the default
    /// (ASCII-case-insensitive) LIKE cannot use the index and falls back to a table scan, while the
    /// range form searches the index (`EXPLAIN QUERY PLAN` confirms it, and a test guards it) —
    /// and, comparing literally, it needs no metacharacter escaping. Names are valid UTF-8, whose
    /// bytewise order is code-point order, so the range `[prefix, successor)` captures exactly the
    /// names carrying that character prefix. The range is case-sensitive; a caller wanting LIKE's
    /// ASCII-case-insensitive semantics fetches each case variant's range.
    pub fn memory_names_with_prefix(&self, prefix: &str) -> Result<Vec<MemoryName>, GraphError> {
        match prefix_range_end(prefix) {
            Some(end) => {
                let stmt = self.conn.prepare(
                    "SELECT name FROM memories
                     WHERE name >= ?1 AND name < ?2 AND deleted = 0 ORDER BY name",
                )?;
                query_map_into(stmt, params![prefix, end], |row| {
                    let name: String = row.get(0)?;
                    Ok::<_, GraphError>(MemoryName::new(name))
                })
            }
            // No exclusive upper bound is representable (the prefix is empty, or every character is
            // the maximum scalar value); scan the index tail and keep the true prefix matches.
            None => {
                let stmt = self.conn.prepare(
                    "SELECT name FROM memories WHERE name >= ?1 AND deleted = 0 ORDER BY name",
                )?;
                let names: Vec<MemoryName> = query_map_into(stmt, params![prefix], |row| {
                    let name: String = row.get(0)?;
                    Ok::<_, GraphError>(MemoryName::new(name))
                })?;
                Ok(names
                    .into_iter()
                    .filter(|name| name.as_str().starts_with(prefix))
                    .collect())
            }
        }
    }
}

/// Escape a `memory.list` prefix's LIKE metacharacters so the stem matches literally. `%`, `_`, and
/// the escape character `\` itself are each backslash-prefixed, to be paired with an `ESCAPE '\'`
/// clause; every other character passes through. So a prefix carrying a `%` matches that percent sign
/// rather than wildcarding the rest of the name.
fn escape_like(prefix: &str) -> String {
    let mut escaped = String::with_capacity(prefix.len());
    for ch in prefix.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// The exclusive upper bound of the index range holding every string prefixed by `prefix`: the
/// prefix with its final character replaced by the next Unicode scalar value. A final character
/// with no successor (`char::MAX`) is dropped and the increment carries leftward — every string it
/// prefixes still sorts below the shortened successor. `None` when no bound exists at all: an empty
/// prefix, or one made entirely of the maximum scalar value.
fn prefix_range_end(prefix: &str) -> Option<String> {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        if let Some(next) = next_scalar(last) {
            chars.push(next);
            return Some(chars.into_iter().collect());
        }
    }
    None
}

/// The next Unicode scalar value after `c`, skipping the surrogate gap; `None` at `char::MAX`.
fn next_scalar(c: char) -> Option<char> {
    let mut code = u32::from(c) + 1;
    if (0xD800..=0xDFFF).contains(&code) {
        code = 0xE000;
    }
    char::from_u32(code)
}
