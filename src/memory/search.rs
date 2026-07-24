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

use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};

use crate::{
    decay,
    event::Volatility,
    graph::{Graph, GraphError, MemoryView},
    ids::{MemoryId, MemoryName, Namespace},
    memory::{memory_block::LinkDirection, visibility},
    model::index::VectorKey,
    settings::SearchSettings,
    time::{self, TemporalRef, Timestamp},
    vector::{VectorError, VectorIndex},
    vocabulary::{RelationName, TagName},
};

/// A ranked search result. `marker` is the inline teller-private marker when the memory surfaced via
/// a private entry, and `None` otherwise. `snippet` is the fragment of matched content that produced
/// the hit — an FTS5 extract for a lexical match, or the matched entry's text (clipped) for a
/// semantic entry match — so the result stays legible even when the memory's description is stale or
/// empty. Both snippet sources are visibility-safe: the FTS index is public-only, and an entry
/// snippet is only ever taken from an entry that has already passed the visibility predicate.
///
/// `occurred_at` is the resolved occurrence a hit carries so a scheduled or dated fact's *when* rides
/// on the result line, rather than surfacing only if the agent separately drills into `entries()`. A
/// hit is memory-level, so this is one representative date — the most recent visible dated entry's
/// occurrence (see [`visible_occurrence`]) — and the agent recalls the full set of occurrences through
/// `entries()`. Like the snippet, it is visibility-filtered: a date from an entry the present set
/// cannot see never leaks onto the hit.
///
/// `relations` are the memory's most salient out-of-class links (see [`salient_relations`]), so a hit
/// passively carries the cast that already participates in it — the recognition signal that lets an
/// agent searching "book club" pick the existing `event/book_club` its present people are on, rather
/// than minting a name-guessed duplicate. `more_relations` counts the salient links elided past the cap,
/// for the trailing `(+N more)` note. Unlike the snippet and occurrence, these are *not*
/// visibility-filtered — they mirror the link readers, which surface the agent's own whole graph.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    pub memory: MemoryView,
    pub score: f32,
    pub marker: Option<String>,
    pub snippet: Option<String>,
    pub occurred_at: Option<TemporalRef>,
    pub relations: Vec<SalientRelation>,
    pub more_relations: usize,
}

/// One salient relation on a hit: the relation, which way it runs relative to the hit's `same_as`
/// class (`Incoming` when the far end points at the identity), and the far memory's name — enough for
/// the agent to recognize the neighbourhood and `memory.get` a far end by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SalientRelation {
    pub relation: RelationName,
    pub direction: LinkDirection,
    pub other_name: MemoryName,
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
    // Resolve identity over the `same_as` class: collapse a raw hit id to its class primary, so every
    // signal is keyed by the primary and a merged class surfaces once, under its primary. Also serves
    // as the visibility predicate's `class_of`. A lone or unknown memory is its own key.
    let class_of = |id| graph.class_id(id).map(|class| class.unwrap_or(id));

    // Semantic: cosine per class — the best over any member's description hit and any visible entry
    // hits (negative similarity clamped away). A description vector is public-safe; an entry vector
    // must pass the predicate, and a surviving private one contributes a marker.
    let mut cosine: BTreeMap<MemoryId, f32> = BTreeMap::new();
    let mut markers: BTreeMap<MemoryId, String> = BTreeMap::new();
    // The matched-content snippet per memory, so a hit reads legibly even with a stale or empty
    // description. An entry-vector hit contributes its (already visibility-filtered) entry text; a
    // lexical hit's FTS extract is preferred below, as it marks the matched span precisely.
    let mut snippets: BTreeMap<MemoryId, String> = BTreeMap::new();
    for hit in vectors.search(query.embedding, over_fetch)? {
        let score = hit.score.max(0.0);
        match VectorKey::parse(&hit.id) {
            Some(VectorKey::Description(id)) => raise(&mut cosine, class_of(id)?, score),
            Some(VectorKey::Entry(entry_id)) => {
                let Some((memory, entry)) = graph.entry_by_id(entry_id)? else {
                    continue;
                };
                if !visibility::visible(&entry, &memory, query.present_set, &class_of)? {
                    continue;
                }
                let primary = class_of(memory.id)?;
                raise(&mut cosine, primary, score);
                // The matched entry survived the predicate, so its text is safe to quote as the
                // class's snippet; the first surviving hit wins (best cosine, by search order).
                if let Entry::Vacant(slot) = snippets.entry(primary) {
                    slot.insert(clip_snippet(&entry.text));
                }
                // The first surviving hit for a class sets its marker (visibility register and/or
                // staleness), via the vacant entry so the work and its `?` compose cleanly.
                if let Entry::Vacant(slot) = markers.entry(primary) {
                    let mut parts = Vec::new();
                    if let Some(marker_text) =
                        graph.entry_provenance_marker(&entry, &memory, query.present_set)?
                    {
                        parts.push(marker_text);
                    }
                    let effective = entry.occurred_sort.unwrap_or(entry.asserted_at);
                    if decay::is_stale(memory.volatility, effective, now) {
                        parts.push(decay::STALE_MARKER.to_owned());
                    }
                    if !parts.is_empty() {
                        slot.insert(parts.join(" "));
                    }
                }
            }
            // Contextual vectors are for dedup/consolidation, not search.
            Some(VectorKey::EntryContextual(_)) => {}
            None => {}
        }
    }

    // Lexical: normalized bm25 per class. FTS holds only public content, so a lexical hit needs no
    // visibility filter — and neither does its snippet, an FTS extract of that public content. Each
    // hit is collapsed to its class primary, keeping the strongest (most negative) bm25 member and
    // that member's snippet, so a merged class contributes one score and one extract, under its
    // primary. Normalization then sees one score per class. The FTS extract marks the matched span,
    // so it takes precedence over any entry-vector snippet.
    let lexical = graph.search_lexical(query.text, over_fetch)?;
    let mut lexical_best: BTreeMap<MemoryId, (f32, String)> = BTreeMap::new();
    for hit in &lexical {
        let primary = class_of(hit.id)?;
        match lexical_best.entry(primary) {
            Entry::Vacant(slot) => {
                slot.insert((hit.score, hit.snippet.clone()));
            }
            Entry::Occupied(mut slot) if hit.score < slot.get().0 => {
                slot.insert((hit.score, hit.snippet.clone()));
            }
            Entry::Occupied(_) => {}
        }
    }
    // The best-bm25 member's extract wins for the class; a non-empty extract takes precedence over any
    // entry-vector snippet already set, while an empty one leaves that fallback in place.
    for (primary, (_, snippet)) in &lexical_best {
        if !snippet.is_empty() {
            snippets.insert(*primary, clip_snippet(snippet));
        }
    }
    let bm25 = normalize_bm25(&lexical_best);

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
        // A renamed memory carries a "formerly …" marker, so a hit reached by an old name (or whose
        // older content still uses one) reads as the same person under their current handle, rather
        // than a second one (spec §Identity → Renaming).
        let marker = combine_marker(markers.get(&id).cloned(), graph.former_names(id)?);
        // Surface the memory's representative occurrence, so a recall that renders from the hit line
        // (rather than drilling into `entries()`) still carries a scheduled or dated fact's date.
        // Filtered by the same predicate as the snippet: a date on an entry the present set cannot see
        // never leaks.
        let occurred_at = visible_occurrence(&memory, graph, query.present_set, &class_of)?;
        // The most salient out-of-class links, so the hit line reveals the cast already on this memory
        // (spec §Search → informed creation). One class-traversing read per hit, bounded by `limit`.
        let (relations, more_relations) =
            salient_relations(&memory, graph, query.present_set, &class_of)?;
        hits.push(SearchHit {
            memory,
            score,
            marker,
            snippet: snippets.get(&id).cloned(),
            occurred_at,
            relations,
            more_relations,
        });
    }
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    hits.truncate(limit);
    Ok(hits)
}

/// Append a `[formerly …]` note to a hit's marker when the memory has been renamed, so an old-name
/// match — or a hit whose older content still uses an old name — reads as the same person under their
/// current handle rather than a second one (spec §Identity → Renaming).
fn combine_marker(marker: Option<String>, former_names: Vec<MemoryName>) -> Option<String> {
    if former_names.is_empty() {
        return marker;
    }
    let names: Vec<&str> = former_names.iter().map(MemoryName::as_str).collect();
    let note = format!("[formerly {}]", names.join(", "));
    Some(match marker {
        Some(existing) => format!("{existing} {note}"),
        None => note,
    })
}

/// The occurrence to surface on a hit: the most recent visible dated entry's `occurred_at`, over the
/// memory's whole `same_as` class, preferring an authored occurrence over an extracted one. An
/// authored date is ground truth (the agent stamped it at append); an extracted one is inference the
/// turn-end temporal extraction resolved, which can misfire (anaphora like "that weekend" resolved
/// against the clock). So the freshest visible authored date wins, and a visible extracted date
/// surfaces only when the class holds no authored date at all — a guess never shadows a stated fact.
/// Within each tier, entries are scanned in commit order, so the last one wins — the freshest dated
/// fact, which for a recall is the scheduled event or decision the agent is most likely relaying. The
/// visibility predicate gates each entry against the present set, mirroring the snippet, so a date on
/// a teller-private aside the present set cannot see never leaks onto the hit. `None` when the memory
/// holds no visible dated entry.
fn visible_occurrence(
    memory: &MemoryView,
    graph: &Graph,
    present_set: &[MemoryId],
    class_of: &visibility::ClassOf,
) -> Result<Option<TemporalRef>, GraphError> {
    let mut latest_authored = None;
    let mut latest_extracted = None;
    for entry in graph.class_entries(memory.id)? {
        if entry.occurred_at.is_some()
            && visibility::visible(&entry, memory, present_set, class_of)?
        {
            if entry.occurred_authored {
                latest_authored = entry.occurred_at;
            } else {
                latest_extracted = entry.occurred_at;
            }
        }
    }
    Ok(latest_authored.or(latest_extracted))
}

/// How many salient relations a hit carries before the rest are elided behind a `(+N more)` note —
/// small on purpose, enough to reveal the memory's cast (its people, its recent links) without
/// flooding a result line.
const SALIENCE_CAP: usize = 3;

/// The salient relations to surface on a hit, with the count of any elided past the cap. Salience is
/// deliberately simple and legible: of the memory's out-of-class links, a far end that is a
/// [`Namespace::Person`] memory comes first — people anchor identity, so seeing who participates is
/// what lets the agent recognize the event or topic it already holds — and within that, the most
/// recently created links come first. The set is capped at [`SALIENCE_CAP`]; the remainder feeds the
/// `(+N more)` note. Visibility-filtered through `link_visible` when an audience is present, mirroring
/// the content entry reads. One class-traversing read per hit, so the cost is bounded by the search
/// `limit`, as [`visible_occurrence`] is.
fn salient_relations(
    memory: &MemoryView,
    graph: &Graph,
    present_set: &[MemoryId],
    class_of: &visibility::ClassOf,
) -> Result<(Vec<SalientRelation>, usize), GraphError> {
    let person = Namespace::Person.prefix();
    let mut neighbors = graph.class_neighbor_links(memory.id)?;
    // Filter private links when an audience is present. With no one present (a solo flush or
    // maintenance pass), nothing is filtered — mirroring the content direct-read carve-out.
    if !present_set.is_empty() {
        neighbors.retain(|neighbor| -> bool {
            let symmetric = graph
                .relation(neighbor.relation.as_str())
                .ok()
                .flatten()
                .map(|r| r.symmetric)
                .unwrap_or(false);
            visibility::link_visible(&neighbor.link_vis(), symmetric, present_set, class_of)
                .unwrap_or(false)
        });
    }
    // `class_neighbor_links` already orders most-recently-created first; a *stable* sort then floats
    // person far ends ahead without disturbing that recency order within each group.
    neighbors.sort_by_key(|neighbor| !neighbor.other_name.as_str().starts_with(person));
    let total = neighbors.len();
    let relations: Vec<SalientRelation> = neighbors
        .into_iter()
        .take(SALIENCE_CAP)
        .map(|neighbor| SalientRelation {
            relation: neighbor.relation,
            direction: if neighbor.incoming {
                LinkDirection::Incoming
            } else {
                LinkDirection::Outgoing
            },
            other_name: neighbor.other_name,
        })
        .collect();
    let more = total.saturating_sub(relations.len());
    Ok((relations, more))
}

/// Keep the best (highest) cosine seen for a memory.
fn raise(cosine: &mut BTreeMap<MemoryId, f32>, id: MemoryId, score: f32) {
    let best = cosine.entry(id).or_insert(0.0);
    *best = best.max(score);
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

/// Normalize the per-class raw bm25 scores (more negative is a better match) to `[0, 1]`, best at 1.
/// The input is already collapsed to one score per class primary, so the normalization spans classes,
/// not raw stubs.
fn normalize_bm25(lexical: &BTreeMap<MemoryId, (f32, String)>) -> BTreeMap<MemoryId, f32> {
    let min = lexical
        .values()
        .map(|(score, _)| *score)
        .fold(f32::INFINITY, f32::min);
    let max = lexical
        .values()
        .map(|(score, _)| *score)
        .fold(f32::NEG_INFINITY, f32::max);
    let range = max - min;
    lexical
        .iter()
        .map(|(id, (score, _))| {
            let normalized = if range > 0.0 {
                (max - score) / range
            } else {
                1.0
            };
            (*id, normalized)
        })
        .collect()
}

/// Clip a matched-content snippet to a legible length, appending an ellipsis when it is cut. The FTS5
/// extract is already short, so this mainly bounds an entry-vector snippet (a whole entry's text) to a
/// phrase-sized preview. Cuts on a `char` boundary, not a byte offset, so multi-byte text stays valid.
fn clip_snippet(text: &str) -> String {
    const MAX_CHARS: usize = 160;
    let trimmed = text.trim();
    let mut clipped: String = trimmed.chars().take(MAX_CHARS).collect();
    if trimmed.chars().count() > MAX_CHARS {
        clipped.push('…');
    }
    clipped
}

/// `exp(-Δt / τ(volatility))` over the memory's most recent *occurrence* time — each entry's
/// `occurred_sort` when present, else its assertion time — falling back to the memory's creation time
/// (spec §Time → recency). So an entry written today *about* 2019 retrieves like a 2019 memory.
/// Bounded to `[0, 1]`; a future-dated occurrence (a calendar item) counts as no decay.
fn recency_bonus(
    memory: &MemoryView,
    graph: &Graph,
    now: Timestamp,
    settings: &SearchSettings,
) -> Result<f32, GraphError> {
    let latest_relevant = graph
        .class_entries(memory.id)?
        .iter()
        .map(|entry| {
            entry
                .occurred_sort
                .unwrap_or(entry.asserted_at)
                .as_millisecond()
        })
        .max()
        .unwrap_or_else(|| memory.created_at.as_millisecond());
    let delta_days =
        (now.as_millisecond() - latest_relevant).max(0) as f32 / time::MILLIS_PER_DAY as f32;
    let tau = match memory.volatility {
        Volatility::High => settings.recency.tau_days.high,
        Volatility::Medium => settings.recency.tau_days.medium,
        Volatility::Low => settings.recency.tau_days.low,
    };
    Ok((-delta_days / tau).exp())
}

#[cfg(test)]
mod tests;
