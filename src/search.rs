//! Multi-signal memory search (spec §Time → search scoring).
//!
//! Ranking blends a semantic signal (cosine over embeddings, via the [`VectorIndex`] seam), a
//! lexical signal (FTS5 bm25 over name/description/content), and a recency bonus that decays with
//! the memory's volatility. The blend weights and decay constants live in [`SearchSettings`], the
//! search slice of [`Settings`](crate::settings::Settings), which is read from the log. The query is
//! embedded by the caller (the embedder is async; the ranker is synchronous), so this stays testable
//! with the fake embedder and in-memory index.
//!
//! This first cut indexes a semantic vector per memory (its description) and ranks live memories;
//! per-entry vectors, the tag signal, and the namespace filter follow.

use std::collections::{BTreeMap, BTreeSet};

use ulid::Ulid;

use crate::{
    event::Volatility,
    graph::{Graph, GraphError, MemoryView},
    ids::{MemoryId, Timestamp},
    settings::SearchSettings,
    vector::VectorIndex,
};

const MILLIS_PER_DAY: f32 = 86_400_000.0;

/// A ranked search result.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    pub memory: MemoryView,
    pub score: f32,
}

/// Rank live memories for `query`, blending semantic similarity (`query_embedding` against the
/// vector index), lexical bm25, and a recency bonus. `now` drives recency decay.
pub fn search(
    graph: &Graph,
    vectors: &dyn VectorIndex,
    query: &str,
    query_embedding: &[f32],
    settings: &SearchSettings,
    now: Timestamp,
    limit: usize,
) -> Result<Vec<SearchHit>, GraphError> {
    let over_fetch = limit.saturating_mul(4).max(20);

    // Semantic: cosine per memory, clamped to [0, 1] (negative similarity contributes nothing).
    let mut cosine: BTreeMap<MemoryId, f32> = BTreeMap::new();
    for hit in vectors.search(query_embedding, over_fetch) {
        if let Ok(ulid) = Ulid::from_string(hit.id.0.as_str()) {
            cosine.insert(MemoryId(ulid), hit.score.max(0.0));
        }
    }

    // Lexical: normalized bm25 per memory.
    let bm25 = normalize_bm25(&graph.search_lexical(query, over_fetch)?);

    let candidates: BTreeSet<MemoryId> = cosine.keys().chain(bm25.keys()).copied().collect();

    let mut hits = Vec::new();
    for id in candidates {
        let Some(memory) = graph.memory_by_id(id)? else {
            continue;
        };
        let recency = recency_bonus(&memory, graph, now, settings)?;
        let score = settings.cosine * cosine.get(&id).copied().unwrap_or(0.0)
            + settings.bm25 * bm25.get(&id).copied().unwrap_or(0.0)
            + settings.recency.bonus * recency;
        hits.push(SearchHit { memory, score });
    }
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    hits.truncate(limit);
    Ok(hits)
}

/// Normalize raw bm25 scores (more negative is a better match) to `[0, 1]`, best at 1.
fn normalize_bm25(lexical: &[(MemoryId, f32)]) -> BTreeMap<MemoryId, f32> {
    let min = lexical
        .iter()
        .map(|(_, s)| *s)
        .fold(f32::INFINITY, f32::min);
    let max = lexical
        .iter()
        .map(|(_, s)| *s)
        .fold(f32::NEG_INFINITY, f32::max);
    let range = max - min;
    lexical
        .iter()
        .map(|(id, score)| {
            let normalized = if range > 0.0 {
                (max - score) / range
            } else {
                1.0
            };
            (*id, normalized)
        })
        .collect()
}

/// `exp(-Δt / τ(volatility))` over the memory's most recent assertion time (falling back to its
/// creation time). Bounded to `[0, 1]`; future-dated times count as no decay.
fn recency_bonus(
    memory: &MemoryView,
    graph: &Graph,
    now: Timestamp,
    settings: &SearchSettings,
) -> Result<f32, GraphError> {
    let latest_assertion = graph
        .entries(memory.id)?
        .iter()
        .map(|entry| entry.asserted_at.as_millis())
        .max()
        .unwrap_or_else(|| memory.created_at.as_millis());
    let delta_days = (now.as_millis() - latest_assertion).max(0) as f32 / MILLIS_PER_DAY;
    let tau = match memory.volatility {
        Volatility::High => settings.recency.tau_days.high,
        Volatility::Medium => settings.recency.tau_days.medium,
        Volatility::Low => settings.recency.tau_days.low,
    };
    Ok((-delta_days / tau).exp())
}
