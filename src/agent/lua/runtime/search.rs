//! The `memory.search` runner: embed the query, rank under a brief read lock, map hits to Lua rows.

use serde::Deserialize;

use crate::{
    engine::Engine,
    ids::MemoryId,
    memory::search::{SalientRelation, SearchQuery, search},
    settings::Settings,
    time::TemporalRef,
    vocabulary::TagName,
};

use crate::agent::lua::error::MemorySearchError;

/// The default number of `memory.search` results when the caller gives no `limit`.
pub(crate) const DEFAULT_SEARCH_LIMIT: usize = 8;

/// The `opts` table `memory.search` accepts, deserialized from Lua.
#[derive(Default, Deserialize)]
#[serde(default)]
pub(crate) struct SearchOpts {
    pub(crate) namespace: Option<String>,
    pub(crate) tags: Vec<String>,
    pub(crate) limit: Option<usize>,
}

/// One ranked search result handed back to Lua as
/// `{ name, description, score, marker?, snippet?, occurred_at?, relations? }`. `snippet` is the matched
/// content that produced the hit, so a result stays legible even when the memory's description is stale
/// or empty; `occurred_at` is the memory's representative occurrence (the same tagged table `append`
/// takes), so a scheduled or dated fact's date rides on the result rather than surfacing only through
/// a separate `entries()` read; `relations` are the memory's most salient links (its cast), so the hit
/// passively carries who already participates in it — the recognition signal that steers a search
/// toward reusing the memory it found rather than minting a duplicate. `more_relations` counts the
/// salient links elided past the render cap, for the trailing `(+N more)` note. `id` backs the row's
/// double life as a memory handle: it rides as the hit table's `id` field so the hit's metatable can
/// fall through to the handle methods, letting `hit:append(…)` and `hit:details()` act on the found
/// memory without a `memory.get` round-trip.
pub(crate) struct SearchRow {
    pub(crate) id: MemoryId,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) score: f32,
    pub(crate) marker: Option<String>,
    pub(crate) snippet: Option<String>,
    pub(crate) occurred_at: Option<TemporalRef>,
    pub(crate) relations: Vec<SalientRelation>,
    pub(crate) more_relations: usize,
}

/// Run a `memory.search`: embed the query off every lock, read the search settings, then rank under a
/// brief graph + vector-index read lock (spec §Time → search scoring, §Visibility). The `Err` is the
/// agent-facing failure message — search is read-only, so a failure (no embedder, a transient embed or
/// backend error) terminates the block without corrupting anything.
pub(crate) async fn run_memory_search(
    engine: &Engine,
    present_set: &[MemoryId],
    query: &str,
    opts: &SearchOpts,
) -> Result<Vec<SearchRow>, MemorySearchError> {
    // An empty or whitespace query has nothing to match on — reject it before the embedder is called,
    // so a degenerate "list everything in a namespace" search fails fast and teachably rather than
    // embedding the empty string and grinding the whole memory through the ranker.
    if query.trim().is_empty() {
        return Err(MemorySearchError::EmptyQuery);
    }
    let Some(retrieval) = &engine.retrieval else {
        return Err(MemorySearchError::NoRetrieval);
    };
    let started = std::time::Instant::now();
    let embedding = retrieval
        .embedder
        .embed(&[query.to_owned()])
        .await
        .map_err(MemorySearchError::Embed)?
        .into_iter()
        .next()
        .ok_or(MemorySearchError::NoVector)?;
    let settings = Settings::from_store(engine.store.lock().as_ref())
        .map_err(MemorySearchError::Settings)?
        .search;
    let now = engine.clock.now();
    let tags: Vec<TagName> = opts.tags.iter().map(|t| TagName::new(t)).collect();
    let request = SearchQuery {
        text: query,
        embedding: &embedding,
        namespace: opts.namespace.as_deref(),
        tags: &tags,
        present_set,
    };
    let limit = opts.limit.unwrap_or(DEFAULT_SEARCH_LIMIT);
    let hits = {
        // Graph before the vector index — the lock order `memory.search` and the indexer share. Both
        // are held only across the synchronous ranking, never an `.await`.
        let graph = engine.graph.lock();
        let vectors = retrieval.vectors.lock();
        search(&graph, vectors.as_ref(), &request, &settings, now, limit)
            .map_err(MemorySearchError::Search)?
    };
    crate::metrics::observe_search(started.elapsed());
    Ok(hits
        .into_iter()
        .map(|hit| SearchRow {
            id: hit.memory.id,
            name: hit.memory.name.as_str().to_owned(),
            description: hit.memory.description,
            score: hit.score,
            marker: hit.marker,
            snippet: hit.snippet,
            occurred_at: hit.occurred_at,
            relations: hit.relations,
            more_relations: hit.more_relations,
        })
        .collect())
}
