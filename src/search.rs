//! Multi-signal memory search (spec §Time → search scoring).
//!
//! Ranking blends a semantic signal (cosine over embeddings, via the [`VectorIndex`] seam), a
//! lexical signal (FTS5 bm25 over name/description/content), and a recency bonus that decays with
//! the memory's volatility. The blend weights and decay constants live in [`SearchSettings`], the
//! search slice of [`Settings`](crate::settings::Settings), which is read from the log. The query is
//! embedded by the caller (the embedder is async; the ranker is synchronous), so this stays testable
//! with the fake embedder and in-memory index.
//!
//! Both description and entry vectors are searched: a description hit surfaces its memory (built
//! from public entries, so it needs no filter), while an entry hit is resolved to its entry and
//! filtered by the visibility predicate against the present set before it can surface its memory — a
//! surviving private entry attaches the inline teller-private marker. The real sqlite-vec backend
//! follows.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    event::{Teller, Visibility, Volatility},
    graph::{Graph, GraphError, MemoryView},
    ids::{MemoryId, TagName, Timestamp},
    index::VectorKey,
    settings::SearchSettings,
    vector::{VectorError, VectorIndex},
    visibility,
};

const MILLIS_PER_DAY: f32 = 86_400_000.0;

/// A ranked search result. `marker` is the inline teller-private marker when the memory surfaced via
/// a private entry, and `None` otherwise.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    pub memory: MemoryView,
    pub score: f32,
    pub marker: Option<String>,
}

/// A search failure, from either the graph projection or the vector index.
#[derive(Debug)]
pub enum SearchError {
    Graph(GraphError),
    Vector(VectorError),
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchError::Graph(error) => write!(f, "search (graph): {error}"),
            SearchError::Vector(error) => write!(f, "search (vector): {error}"),
        }
    }
}

impl std::error::Error for SearchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SearchError::Graph(error) => Some(error),
            SearchError::Vector(error) => Some(error),
        }
    }
}

impl From<GraphError> for SearchError {
    fn from(error: GraphError) -> SearchError {
        SearchError::Graph(error)
    }
}

impl From<VectorError> for SearchError {
    fn from(error: VectorError) -> SearchError {
        SearchError::Vector(error)
    }
}

/// A search request: free text plus its `embedding` (computed by the caller), optionally narrowed to
/// a name `namespace` prefix and carrying `tags` whose overlap with a memory feeds the tag signal.
pub struct SearchQuery<'a> {
    pub text: &'a str,
    pub embedding: &'a [f32],
    /// Restrict results to memories whose name starts with this prefix (e.g. `"person/"`); `None`
    /// searches every namespace.
    pub namespace: Option<&'a str>,
    /// Tags the caller is looking for; the tag signal is the fraction of these a memory carries.
    pub tags: &'a [TagName],
    /// The participants present, against which the visibility predicate filters entry hits.
    pub present_set: &'a [MemoryId],
}

/// Rank live memories for `query`, blending semantic similarity (the query embedding against the
/// vector index), lexical bm25, tag overlap, and a recency bonus. `now` drives recency decay; the
/// namespace prefix, if any, filters candidates.
pub fn search(
    graph: &Graph,
    vectors: &dyn VectorIndex,
    query: &SearchQuery,
    settings: &SearchSettings,
    now: Timestamp,
    limit: usize,
) -> Result<Vec<SearchHit>, SearchError> {
    let over_fetch = limit.saturating_mul(4).max(20);
    // Resolve identity over the `same_as` class for the visibility predicate.
    let class_of = |id| graph.class_id(id).map(|class| class.unwrap_or(id));

    // Semantic: cosine per memory — the best over its description hit and any visible entry hits
    // (negative similarity clamped away). A description vector is public-safe; an entry vector must
    // pass the predicate, and a surviving private one contributes a marker.
    let mut cosine: BTreeMap<MemoryId, f32> = BTreeMap::new();
    let mut markers: BTreeMap<MemoryId, String> = BTreeMap::new();
    for hit in vectors.search(query.embedding, over_fetch)? {
        let score = hit.score.max(0.0);
        match VectorKey::parse(&hit.id) {
            Some(VectorKey::Description(id)) => raise(&mut cosine, id, score),
            Some(VectorKey::Entry(entry_id)) => {
                let Some((memory, entry)) = graph.entry_by_id(entry_id)? else {
                    continue;
                };
                if !visibility::visible(&entry, &memory, query.present_set, &class_of)? {
                    continue;
                }
                raise(&mut cosine, memory.id, score);
                if entry.visibility != Visibility::Public && !markers.contains_key(&memory.id) {
                    let teller = teller_display(graph, &entry.told_by)?;
                    markers.insert(memory.id, visibility::teller_private_marker(&teller));
                }
            }
            None => {}
        }
    }

    // Lexical: normalized bm25 per memory. FTS holds only public content, so a lexical hit needs no
    // visibility filter.
    let bm25 = normalize_bm25(&graph.search_lexical(query.text, over_fetch)?);

    let candidates: BTreeSet<MemoryId> = cosine.keys().chain(bm25.keys()).copied().collect();

    let mut hits = Vec::new();
    for id in candidates {
        let Some(memory) = graph.memory_by_id(id)? else {
            continue;
        };
        if let Some(prefix) = query.namespace
            && !memory.name.as_str().starts_with(prefix)
        {
            continue;
        }
        let recency = recency_bonus(&memory, graph, now, settings)?;
        let score = settings.cosine * cosine.get(&id).copied().unwrap_or(0.0)
            + settings.bm25 * bm25.get(&id).copied().unwrap_or(0.0)
            + settings.tag * tag_match(&memory, query.tags)
            + settings.recency.bonus * recency;
        hits.push(SearchHit {
            memory,
            score,
            marker: markers.get(&id).cloned(),
        });
    }
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    hits.truncate(limit);
    Ok(hits)
}

/// Keep the best (highest) cosine seen for a memory.
fn raise(cosine: &mut BTreeMap<MemoryId, f32>, id: MemoryId, score: f32) {
    let best = cosine.entry(id).or_insert(0.0);
    *best = best.max(score);
}

/// Resolve a teller to the display name the marker shows.
fn teller_display(graph: &Graph, teller: &Teller) -> Result<String, GraphError> {
    Ok(match teller {
        Teller::Participant(id) => graph
            .memory_by_id(*id)?
            .map(|memory| memory.name.as_str().to_owned())
            .unwrap_or_else(|| "someone".to_owned()),
        Teller::Agent => "the agent".to_owned(),
        Teller::Bootstrap => "genesis".to_owned(),
    })
}

/// The fraction of the query's `tags` a memory carries, in `[0, 1]`; zero when no tags are
/// requested, so the tag signal contributes nothing to a plain text search.
fn tag_match(memory: &MemoryView, query_tags: &[TagName]) -> f32 {
    if query_tags.is_empty() {
        return 0.0;
    }
    let matched = query_tags
        .iter()
        .filter(|tag| memory.tags.contains(tag))
        .count();
    matched as f32 / query_tags.len() as f32
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
        .class_entries(memory.id)?
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
