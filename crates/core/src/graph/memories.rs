//! Memory lookups: by name, id, former name, and the same-identity (`same_as`) class.

use super::{Graph, GraphError, MemoryView, backend, parse_ulid};
use crate::{
    db::{query_map_into, query_opt_into},
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
}
