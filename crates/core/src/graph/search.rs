//! Semantic and lexical search over the projection.

use super::{Graph, GraphError, MemoryView, parse_ulid};
use crate::{
    db::{query_map_into, query_opt_into},
    ids::MemoryId,
};
use rusqlite::params;

impl Graph {
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

    pub(super) fn fetch_memory(
        &self,
        column: &str,
        value: &str,
    ) -> Result<Option<MemoryView>, GraphError> {
        let sql = format!(
            "SELECT id, name, description, volatility, created_at FROM memories
             WHERE {column} = ?1 AND deleted = 0"
        );
        let stmt = self.conn.prepare(&sql)?;
        query_opt_into(stmt, params![value], |row| {
            self.assemble_memory(row.try_into()?)
        })
    }

    pub(super) fn query_ids(
        &self,
        sql: &str,
        id: &str,
        relation: &str,
    ) -> Result<Vec<String>, GraphError> {
        let stmt = self.conn.prepare(sql)?;
        query_map_into(stmt, params![id, relation], |row| Ok(row.get(0)?))
    }
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
