//! Link reads: outgoing edges, full link lists, and class-wide links.

use super::{ClassLinkView, Graph, GraphError, LinkView, MemoryView, parse_ulid};
use crate::{
    db::{query_map_into, query_opt_into},
    ids::MemoryId,
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
                relation: RelationName::new(&relation),
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
                relation: RelationName::new(&relation),
                source: source.parse().map_err(|()| {
                    GraphError::Malformed(format!("unknown link source {source:?}"))
                })?,
                told_by: told_by
                    .map(|json| serde_json::from_str(&json))
                    .transpose()?,
            })
        })
    }
}
