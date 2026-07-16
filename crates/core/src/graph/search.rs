//! Semantic and lexical search over the projection.

use crate::{
    db::{query_map_into, query_opt_into},
    graph::{Graph, GraphError, MemoryView, parse_ulid},
    ids::MemoryId,
};
use rusqlite::params;

/// A lexical (FTS5) hit: the memory, its raw bm25 score (more negative is a better match), and a
/// `snippet` of the matched text — FTS5's own extract over whichever indexed column matched, with
/// elided context marked by an ellipsis. The FTS index holds only public content (spec §Visibility →
/// public-only lexical indexing), so the snippet is public-safe and needs no visibility filter.
pub struct LexicalHit {
    pub id: MemoryId,
    pub score: f32,
    pub snippet: String,
}

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

    /// Lexical hits with their raw FTS5 bm25 score (more negative is a better match) and a snippet of
    /// the matched text, for the multi-signal ranker to normalize, blend, and render. Deleted memories
    /// are excluded. `snippet(memories_fts, -1, …)` extracts around the match in whichever indexed
    /// column matched (name, description, or content), marking elided context with an ellipsis and
    /// capping the window at ~10 tokens so the fragment stays legible on a result line.
    pub fn search_lexical(&self, query: &str, limit: usize) -> Result<Vec<LexicalHit>, GraphError> {
        let match_query = build_match(query);
        if match_query.is_empty() {
            return Ok(Vec::new());
        }
        let stmt = self.conn.prepare(
            "SELECT f.memory_id, bm25(memories_fts) AS score,
                    snippet(memories_fts, -1, '', '', '…', 10) AS snip
             FROM memories_fts f JOIN memories m ON m.id = f.memory_id
             WHERE memories_fts MATCH ?1 AND m.deleted = 0
             ORDER BY score LIMIT ?2",
        )?;
        query_map_into(stmt, params![match_query, limit as i64], |row| {
            let (id, score, snippet): (String, f64, String) = row.try_into()?;
            Ok(LexicalHit {
                id: MemoryId(parse_ulid(&id)?),
                score: score as f32,
                snippet,
            })
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
