//! Link reads: outgoing edges, full link lists, and class-wide links.

use super::{ClassLinkView, Graph, GraphError, LinkView, MemoryView, NeighborLinkView, parse_ulid};
use crate::{
    db::{query_map_into, query_opt_into},
    ids::{MemoryId, MemoryName},
    vocabulary::RelationName,
};
use rusqlite::params;

impl Graph {
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
            "SELECT l.from_id, l.to_id, l.relation, l.told_by, l.told_in, l.visibility
             FROM links l
             JOIN memories mf ON mf.id = l.from_id
             JOIN memories mt ON mt.id = l.to_id
             WHERE (l.from_id = ?1 OR l.to_id = ?1) AND mf.deleted = 0 AND mt.deleted = 0
             ORDER BY l.relation, l.to_id",
        )?;
        query_map_into(stmt, params![id.0.to_string()], |row| {
            let from: String = row.get("from_id")?;
            let to: String = row.get("to_id")?;
            let relation: String = row.get("relation")?;
            let told_by: Option<String> = row.get("told_by")?;
            let told_in: Option<String> = row.get("told_in")?;
            let visibility: String = row.get("visibility")?;
            Ok(LinkView {
                from: MemoryId(parse_ulid(&from)?),
                to: MemoryId(parse_ulid(&to)?),
                relation: RelationName::new(&relation),
                told_by: told_by
                    .map(|json| serde_json::from_str(&json))
                    .transpose()?,
                told_in: told_in
                    .map(|id| parse_ulid(&id).map(MemoryId))
                    .transpose()?,
                visibility: serde_json::from_str(&visibility)?,
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
            "SELECT l.from_id, l.to_id, l.relation, l.source, l.told_by, l.told_in, l.visibility
             FROM links l
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
            let told_in: Option<String> = row.get("told_in")?;
            let visibility: String = row.get("visibility")?;
            Ok(ClassLinkView {
                from: MemoryId(parse_ulid(&from)?),
                to: MemoryId(parse_ulid(&to)?),
                relation: RelationName::new(&relation),
                source: source.parse().map_err(|()| {
                    GraphError::Malformed(format!("unknown link source {source:?}"))
                })?,
                told_by: told_by
                    .map(|json| serde_json::from_str(&json))
                    .transpose()?,
                told_in: told_in
                    .map(|id| parse_ulid(&id).map(MemoryId))
                    .transpose()?,
                visibility: serde_json::from_str(&visibility)?,
            })
        })
    }

    /// Every edge leaving `id`'s whole `same_as` class — oriented against the class, carrying the far
    /// memory's resolved name, ordered most-recently created first (by the link's insertion `rowid`) —
    /// the raw neighbor set a search hit distills into its salient-relations line. Edges internal to the
    /// class (both endpoints class members) are dropped: those are the `same_as` plumbing within an
    /// identity, not a relationship pointing out of it. Committed state; visibility-filtered through
    /// `link_visible` when an audience is present, mirroring the content entry reads.
    pub fn class_neighbor_links(&self, id: MemoryId) -> Result<Vec<NeighborLinkView>, GraphError> {
        let stmt = self.conn.prepare(
            "WITH cls AS (
                 SELECT id FROM memories
                 WHERE class_id = (SELECT class_id FROM memories WHERE id = ?1 AND deleted = 0)
                   AND deleted = 0
             )
             SELECT l.relation AS relation,
                    (l.from_id NOT IN (SELECT id FROM cls)) AS incoming,
                    CASE WHEN l.from_id IN (SELECT id FROM cls) THEN l.to_id ELSE l.from_id END
                        AS other_id,
                    CASE WHEN l.from_id IN (SELECT id FROM cls) THEN mt.name ELSE mf.name END
                        AS other_name,
                    l.from_id AS from_id,
                    l.to_id AS to_id,
                    l.told_by AS told_by,
                    l.told_in AS told_in,
                    l.visibility AS visibility
             FROM links l
             JOIN memories mf ON mf.id = l.from_id AND mf.deleted = 0
             JOIN memories mt ON mt.id = l.to_id   AND mt.deleted = 0
             WHERE (l.from_id IN (SELECT id FROM cls) OR l.to_id IN (SELECT id FROM cls))
               AND NOT (l.from_id IN (SELECT id FROM cls) AND l.to_id IN (SELECT id FROM cls))
             ORDER BY l.rowid DESC",
        )?;
        query_map_into(stmt, params![id.0.to_string()], |row| {
            let relation: String = row.get("relation")?;
            let incoming: bool = row.get("incoming")?;
            let other_id: String = row.get("other_id")?;
            let other_name: String = row.get("other_name")?;
            let from_id: String = row.get("from_id")?;
            let to_id: String = row.get("to_id")?;
            let told_by: Option<String> = row.get("told_by")?;
            let told_in: Option<String> = row.get("told_in")?;
            let visibility: String = row.get("visibility")?;
            Ok(NeighborLinkView {
                relation: RelationName::new(&relation),
                incoming,
                other: MemoryId(parse_ulid(&other_id)?),
                other_name: MemoryName::new(&other_name),
                from: MemoryId(parse_ulid(&from_id)?),
                to: MemoryId(parse_ulid(&to_id)?),
                told_by: told_by
                    .map(|json| serde_json::from_str(&json))
                    .transpose()?,
                told_in: told_in
                    .map(|id| parse_ulid(&id).map(MemoryId))
                    .transpose()?,
                visibility: serde_json::from_str(&visibility)?,
            })
        })
    }
}
