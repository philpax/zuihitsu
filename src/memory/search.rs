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
    event::{Visibility, Volatility},
    graph::{Graph, GraphError, LexicalHit, MemoryView},
    ids::{MemoryId, MemoryName},
    model::index::VectorKey,
    settings::SearchSettings,
    time::{self, TemporalRef, Timestamp},
    vector::{VectorError, VectorIndex},
    vocabulary::TagName,
};

use super::visibility;

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
#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    pub memory: MemoryView,
    pub score: f32,
    pub marker: Option<String>,
    pub snippet: Option<String>,
    pub occurred_at: Option<TemporalRef>,
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
    // The matched-content snippet per memory, so a hit reads legibly even with a stale or empty
    // description. An entry-vector hit contributes its (already visibility-filtered) entry text; a
    // lexical hit's FTS extract is preferred below, as it marks the matched span precisely.
    let mut snippets: BTreeMap<MemoryId, String> = BTreeMap::new();
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
                // The matched entry survived the predicate, so its text is safe to quote as this
                // memory's snippet; the first surviving hit wins (best cosine, by search order).
                if let Entry::Vacant(slot) = snippets.entry(memory.id) {
                    slot.insert(clip_snippet(&entry.text));
                }
                // The first surviving hit for a memory sets its marker (visibility register and/or
                // staleness), via the vacant entry so the work and its `?` compose cleanly.
                if let Entry::Vacant(slot) = markers.entry(memory.id) {
                    let mut parts = Vec::new();
                    if entry.visibility != Visibility::Public {
                        let teller = graph.teller_display(&entry.told_by)?;
                        let room = graph.marker_room(entry.told_in)?;
                        if let Some(marker) =
                            visibility::entry_marker(&entry.visibility, &teller, room.as_ref())
                        {
                            parts.push(marker);
                        }
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
            None => {}
        }
    }

    // Lexical: normalized bm25 per memory. FTS holds only public content, so a lexical hit needs no
    // visibility filter — and neither does its snippet, an FTS extract of that public content. The
    // FTS extract marks the matched span, so it takes precedence over any entry-vector snippet.
    let lexical = graph.search_lexical(query.text, over_fetch)?;
    for hit in &lexical {
        if !hit.snippet.is_empty() {
            snippets.insert(hit.id, clip_snippet(&hit.snippet));
        }
    }
    let bm25 = normalize_bm25(&lexical);

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
        hits.push(SearchHit {
            memory,
            score,
            marker,
            snippet: snippets.get(&id).cloned(),
            occurred_at,
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

/// Normalize raw bm25 scores (more negative is a better match) to `[0, 1]`, best at 1.
fn normalize_bm25(lexical: &[LexicalHit]) -> BTreeMap<MemoryId, f32> {
    let min = lexical
        .iter()
        .map(|hit| hit.score)
        .fold(f32::INFINITY, f32::min);
    let max = lexical
        .iter()
        .map(|hit| hit.score)
        .fold(f32::NEG_INFINITY, f32::max);
    let range = max - min;
    lexical
        .iter()
        .map(|hit| {
            let normalized = if range > 0.0 {
                (max - hit.score) / range
            } else {
                1.0
            };
            (hit.id, normalized)
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
        .map(|entry| entry.occurred_sort.unwrap_or(entry.asserted_at).as_millis())
        .max()
        .unwrap_or_else(|| memory.created_at.as_millis());
    let delta_days =
        (now.as_millis() - latest_relevant).max(0) as f32 / time::MILLIS_PER_DAY as f32;
    let tau = match memory.volatility {
        Volatility::High => settings.recency.tau_days.high,
        Volatility::Medium => settings.recency.tau_days.medium,
        Volatility::Low => settings.recency.tau_days.low,
    };
    Ok((-delta_days / tau).exp())
}

#[cfg(test)]
mod tests {
    use super::{SearchHit, SearchQuery, recency_bonus, search};
    use crate::{
        InstanceFeatures,
        agent::genesis::{self, SeedSelf},
        clock::ManualClock,
        event::{Event, EventPayload, Teller, Visibility, Volatility},
        graph::Graph,
        ids::{EntryId, MemoryId, MemoryName, Namespace, Seq},
        model::{
            embed::{Embedder, FakeEmbedder},
            index::Indexer,
        },
        settings::{SearchSettings, Settings},
        store::{MemoryStore, Store},
        time::{CivilDate, TemporalRef, Timestamp},
        vector::InMemoryVectorIndex,
        vocabulary::TagName,
    };

    const DAY: i64 = 86_400_000;
    const DIMS: usize = 32;

    fn event(seq: u64, payload: EventPayload) -> Event {
        Event {
            seq: Seq(seq),
            recorded_at: Timestamp::from_millis(0),
            payload,
        }
    }

    /// A singleton memory with one entry at the given occurrence (or none) and assertion time.
    fn graph_with_entry(occurred_at: Option<TemporalRef>, asserted_ms: i64) -> (Graph, MemoryId) {
        let mut graph = Graph::open_in_memory().unwrap();
        let id = MemoryId::generate();
        graph
            .apply(&event(
                1,
                EventPayload::memory_created(id, Namespace::Topic.with_name("dated")),
            ))
            .unwrap();
        graph
            .apply(&event(
                2,
                EventPayload::MemoryContentAppended {
                    id,
                    entry_id: EntryId::generate(),
                    asserted_at: Timestamp::from_millis(asserted_ms),
                    occurred_at,
                    text: "fact".to_owned(),
                    told_by: Teller::Agent,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            ))
            .unwrap();
        (graph, id)
    }

    fn bonus(graph: &Graph, id: MemoryId, now_ms: i64) -> f32 {
        let memory = graph.memory_by_id(id).unwrap().unwrap();
        recency_bonus(
            &memory,
            graph,
            Timestamp::from_millis(now_ms),
            &SearchSettings::default(),
        )
        .unwrap()
    }

    #[test]
    fn occurrence_time_drives_decay_not_assertion_time() {
        let now = 20_000 * DAY;
        // Written "today" but about a decade ago: it must decay like a decade-old memory.
        let (about_past, past_id) = graph_with_entry(
            Some(TemporalRef::Instant(Timestamp::from_millis(
                now - 3650 * DAY,
            ))),
            now,
        );
        let (about_now, now_id) =
            graph_with_entry(Some(TemporalRef::Instant(Timestamp::from_millis(now))), now);
        assert!(
            bonus(&about_past, past_id, now) < 0.01,
            "a decade-old occurrence should decay sharply"
        );
        assert!(
            bonus(&about_now, now_id, now) > 0.99,
            "a present occurrence should not decay"
        );
    }

    #[test]
    fn falls_back_to_assertion_time_without_an_occurrence() {
        let now = 20_000 * DAY;
        let (graph, id) = graph_with_entry(None, now - 3650 * DAY);
        assert!(bonus(&graph, id, now) < 0.01);
    }

    #[test]
    fn higher_volatility_decays_faster_at_the_same_age() {
        // The volatility-aware part: at the *same* age, the decay rate is keyed by the memory's
        // volatility through τ — High (τ=90d) decays far faster than Medium (τ=365d) than Low (τ=3650d).
        let now = 20_000 * DAY;
        let one_year_ago = now - 365 * DAY;
        let bonus_for = |volatility| {
            let (mut graph, id) = graph_with_entry(
                Some(TemporalRef::Instant(Timestamp::from_millis(one_year_ago))),
                now,
            );
            graph
                .apply(&event(
                    3,
                    EventPayload::memory_volatility_set(id, volatility),
                ))
                .unwrap();
            bonus(&graph, id, now)
        };
        let (high, medium, low) = (
            bonus_for(Volatility::High),
            bonus_for(Volatility::Medium),
            bonus_for(Volatility::Low),
        );

        // Strictly ordered by volatility.
        assert!(
            high < medium,
            "High should decay faster than Medium ({high} vs {medium})"
        );
        assert!(
            medium < low,
            "Medium should decay faster than Low ({medium} vs {low})"
        );
        // Concrete anchors at one year, with the default τ: exp(-365/90) ≈ 0.017, exp(-1) ≈ 0.37,
        // exp(-365/3650) ≈ 0.90.
        assert!(
            high < 0.05,
            "high volatility is nearly fully decayed at a year: {high}"
        );
        assert!(
            (0.30..0.45).contains(&medium),
            "medium is ~0.37 at a year: {medium}"
        );
        assert!(low > 0.85, "low volatility barely decays at a year: {low}");
    }

    #[test]
    fn a_future_occurrence_does_not_decay() {
        let now = 20_000 * DAY;
        let (graph, id) = graph_with_entry(
            Some(TemporalRef::Instant(Timestamp::from_millis(
                now + 100 * DAY,
            ))),
            now,
        );
        assert!(bonus(&graph, id, now) > 0.99);
    }

    /// A write + index harness for the multi-signal blend. The fake embedder isn't semantic, but it
    /// is deterministic — the same text embeds to the same vector — so querying a memory's exact
    /// description gives it cosine 1, which exercises the semantic signal without a real model.
    struct Corpus {
        store: MemoryStore,
        graph: Graph,
        index: InMemoryVectorIndex,
        embedder: FakeEmbedder,
    }

    impl Corpus {
        fn new() -> Corpus {
            Corpus {
                store: MemoryStore::new(),
                graph: Graph::open_in_memory().unwrap(),
                index: InMemoryVectorIndex::new(),
                embedder: FakeEmbedder::new(DIMS),
            }
        }

        /// Commit `events`, bring the graph to head, and catch the vector index up — the real write +
        /// index path, so descriptions and entries are embedded with their proper keys.
        async fn commit(&mut self, at_ms: i64, events: Vec<EventPayload>) {
            self.store
                .append(Timestamp::from_millis(at_ms), events)
                .unwrap();
            self.graph.materialize_from(&self.store).unwrap();
            Indexer::new(&self.embedder, &mut self.index)
                .catch_up(&self.store)
                .await
                .unwrap();
        }

        /// Add a memory with a public description and one public content entry, asserted at `at_ms`.
        async fn add(
            &mut self,
            name: impl Into<MemoryName>,
            description: &str,
            content: &str,
            at_ms: i64,
        ) -> MemoryId {
            let id = MemoryId::generate();
            let at = Timestamp::from_millis(at_ms);
            self.commit(
                at_ms,
                vec![
                    EventPayload::memory_created(id, name),
                    EventPayload::MemoryContentAppended {
                        id,
                        entry_id: EntryId::generate(),
                        asserted_at: at,
                        occurred_at: None,
                        text: content.to_owned(),
                        told_by: Teller::Agent,
                        told_in: None,
                        visibility: Visibility::Public,
                    },
                    EventPayload::memory_description_regenerated(id, description, None),
                ],
            )
            .await;
            id
        }

        /// Record a participant's private aside about `memory` — a `PrivateToTeller` content entry.
        async fn tell_private(
            &mut self,
            memory: MemoryId,
            text: &str,
            teller: MemoryId,
            at_ms: i64,
        ) {
            self.commit(
                at_ms,
                vec![EventPayload::MemoryContentAppended {
                    id: memory,
                    entry_id: EntryId::generate(),
                    asserted_at: Timestamp::from_millis(at_ms),
                    occurred_at: None,
                    text: text.to_owned(),
                    told_by: Teller::Participant(teller),
                    told_in: None,
                    visibility: Visibility::PrivateToTeller,
                }],
            )
            .await;
        }

        /// As [`Corpus::tell_private`], but told in a specific room (`told_in`), so the surfaced
        /// marker can name it.
        async fn tell_private_in(
            &mut self,
            memory: MemoryId,
            text: &str,
            teller: MemoryId,
            told_in: MemoryId,
            at_ms: i64,
        ) {
            self.commit(
                at_ms,
                vec![EventPayload::MemoryContentAppended {
                    id: memory,
                    entry_id: EntryId::generate(),
                    asserted_at: Timestamp::from_millis(at_ms),
                    occurred_at: None,
                    text: text.to_owned(),
                    told_by: Teller::Participant(teller),
                    told_in: Some(told_in),
                    visibility: Visibility::PrivateToTeller,
                }],
            )
            .await;
        }

        /// As [`Corpus::add`], but the single content entry carries an occurrence — so the memory has
        /// a resolved date to surface on a hit.
        async fn add_dated(
            &mut self,
            name: impl Into<MemoryName>,
            description: &str,
            content: &str,
            occurred_at: TemporalRef,
            at_ms: i64,
        ) -> MemoryId {
            let id = MemoryId::generate();
            let at = Timestamp::from_millis(at_ms);
            self.commit(
                at_ms,
                vec![
                    EventPayload::memory_created(id, name),
                    EventPayload::MemoryContentAppended {
                        id,
                        entry_id: EntryId::generate(),
                        asserted_at: at,
                        occurred_at: Some(occurred_at),
                        text: content.to_owned(),
                        told_by: Teller::Agent,
                        told_in: None,
                        visibility: Visibility::Public,
                    },
                    EventPayload::memory_description_regenerated(id, description, None),
                ],
            )
            .await;
            id
        }

        /// As [`Corpus::tell_private`], but the private aside carries an occurrence — so a date lives
        /// on an entry that only surfaces while its teller is present.
        async fn tell_private_dated(
            &mut self,
            memory: MemoryId,
            text: &str,
            teller: MemoryId,
            occurred_at: TemporalRef,
            at_ms: i64,
        ) {
            self.commit(
                at_ms,
                vec![EventPayload::MemoryContentAppended {
                    id: memory,
                    entry_id: EntryId::generate(),
                    asserted_at: Timestamp::from_millis(at_ms),
                    occurred_at: Some(occurred_at),
                    text: text.to_owned(),
                    told_by: Teller::Participant(teller),
                    told_in: None,
                    visibility: Visibility::PrivateToTeller,
                }],
            )
            .await;
        }

        /// Create `tag` and apply it to `id`. Only one memory per tag in these tests, so the create is
        /// unconditional.
        fn tag(&mut self, id: MemoryId, tag: &str, at_ms: i64) {
            self.store
                .append(
                    Timestamp::from_millis(at_ms),
                    vec![
                        EventPayload::tag_created(TagName::new(tag), format!("about {tag}")),
                        EventPayload::tag_applied_to_memory(id, TagName::new(tag)),
                    ],
                )
                .unwrap();
            self.graph.materialize_from(&self.store).unwrap();
        }

        async fn query(&self, text: &str, now_ms: i64, limit: usize) -> Vec<MemoryId> {
            self.query_in(text, None, &[], &[], now_ms, limit)
                .await
                .into_iter()
                .map(|hit| hit.memory.id)
                .collect()
        }

        async fn query_in(
            &self,
            text: &str,
            namespace: Option<&str>,
            tags: &[TagName],
            present_set: &[MemoryId],
            now_ms: i64,
            limit: usize,
        ) -> Vec<SearchHit> {
            let embedding = self
                .embedder
                .embed(&[text.to_owned()])
                .await
                .unwrap()
                .remove(0);
            let query = SearchQuery {
                text,
                embedding: &embedding,
                namespace,
                tags,
                present_set,
            };
            search(
                &self.graph,
                &self.index,
                &query,
                &Settings::default().search,
                Timestamp::from_millis(now_ms),
                limit,
            )
            .unwrap()
        }
    }

    #[tokio::test]
    async fn the_matching_memory_ranks_first() {
        let mut corpus = Corpus::new();
        let dave = corpus
            .add(
                Namespace::Person.with_name("dave"),
                "An avid rock climber",
                "We met bouldering",
                1_000,
            )
            .await;
        corpus
            .add(
                Namespace::Person.with_name("erin"),
                "A tax accountant",
                "She filed my return",
                1_000,
            )
            .await;
        corpus
            .add(
                Namespace::Topic.with_name("sourdough"),
                "Naturally leavened bread",
                "Fed the starter",
                1_000,
            )
            .await;

        // Querying Dave's exact description gives him cosine 1 (and a lexical match), so he ranks first.
        let ranked = corpus.query("An avid rock climber", 1_000, 5).await;
        assert_eq!(ranked.first(), Some(&dave));
    }

    #[tokio::test]
    async fn recency_breaks_a_tie() {
        let mut corpus = Corpus::new();
        // Identical text → identical semantic and lexical scores; only recency differs.
        let stale = corpus
            .add(
                Namespace::Topic.with_name("stale"),
                "shared topic text",
                "shared topic text",
                0,
            )
            .await;
        let fresh = corpus
            .add(
                Namespace::Topic.with_name("fresh"),
                "shared topic text",
                "shared topic text",
                100 * DAY,
            )
            .await;

        let ranked = corpus.query("shared topic text", 100 * DAY, 5).await;
        assert_eq!(ranked.first(), Some(&fresh));
        assert!(ranked.contains(&stale));
    }

    #[tokio::test]
    async fn a_query_tag_boosts_a_carrier() {
        let mut corpus = Corpus::new();
        // Identical text → identical semantic, lexical, and recency scores; only the tag differs.
        let plain = corpus
            .add(
                Namespace::Topic.with_name("plain"),
                "shared topic text",
                "shared topic text",
                1_000,
            )
            .await;
        let tagged = corpus
            .add(
                Namespace::Topic.with_name("tagged"),
                "shared topic text",
                "shared topic text",
                1_000,
            )
            .await;
        corpus.tag(tagged, "climbing", 1_000);

        let ranked: Vec<MemoryId> = corpus
            .query_in(
                "shared topic text",
                None,
                &[TagName::new("climbing")],
                &[],
                1_000,
                5,
            )
            .await
            .into_iter()
            .map(|hit| hit.memory.id)
            .collect();
        assert_eq!(ranked.first(), Some(&tagged));
        assert!(ranked.contains(&plain));
    }

    #[tokio::test]
    async fn a_namespace_filters_out_other_kinds() {
        let mut corpus = Corpus::new();
        let dave = corpus
            .add(
                Namespace::Person.with_name("dave"),
                "shared marker text",
                "shared marker text",
                1_000,
            )
            .await;
        corpus
            .add(
                Namespace::Topic.with_name("marker"),
                "shared marker text",
                "shared marker text",
                1_000,
            )
            .await;

        // The topic matches lexically and semantically, but the person/ prefix excludes it.
        let ranked: Vec<MemoryId> = corpus
            .query_in(
                "shared marker text",
                Some(Namespace::Person.prefix()),
                &[],
                &[],
                1_000,
                5,
            )
            .await
            .into_iter()
            .map(|hit| hit.memory.id)
            .collect();
        assert_eq!(ranked, vec![dave]);
    }

    #[tokio::test]
    async fn an_empty_corpus_returns_nothing() {
        // No memories, no vectors: nothing to rank, whatever the query.
        let corpus = Corpus::new();
        let ranked = corpus.query("anything at all", 1_000, 5).await;
        assert!(ranked.is_empty());
    }

    #[tokio::test]
    async fn search_applies_the_predicate_to_entry_hits() {
        // Scenario 17: Erin's private aside about Marcus is embedded as an entry vector. The query matches
        // only that aside (the wording appears nowhere public), so Marcus surfaces solely through it.
        let mut corpus = Corpus::new();
        let erin_name = Namespace::Person.with_name("erin");
        let erin = corpus
            .add(&erin_name, "A colleague", "We work together", 1_000)
            .await;
        let marcus = corpus
            .add(
                Namespace::Person.with_name("marcus"),
                "A teammate",
                "On the same team",
                1_000,
            )
            .await;
        corpus
            .tell_private(marcus, "the quarterly review went badly", erin, 1_000)
            .await;

        // Erin present, Marcus absent: the aside surfaces Marcus, flagged teller-private.
        let hits = corpus
            .query_in(
                "the quarterly review went badly",
                None,
                &[],
                &[erin],
                1_000,
                5,
            )
            .await;
        let marcus_hit = hits
            .iter()
            .find(|hit| hit.memory.id == marcus)
            .expect("Marcus surfaces via the aside");
        let marker = marcus_hit
            .marker
            .as_deref()
            .expect("a teller-private marker");
        assert!(marker.contains("teller-private"));
        assert!(marker.contains(&erin_name.to_string()));

        // Marcus present too: the subject-guard suppresses the aside. It's the *same* predicate as the
        // brief, so the private entry survives in no hit — no result carries a teller-private marker.
        // (The fake embedder gives every text a faint nonzero cosine, so Marcus still appears via his
        // public vectors; the load-bearing fact is that the private aside no longer surfaces.)
        let hits = corpus
            .query_in(
                "the quarterly review went badly",
                None,
                &[],
                &[erin, marcus],
                1_000,
                5,
            )
            .await;
        assert!(hits.iter().all(|hit| hit.marker.is_none()));
    }

    #[tokio::test]
    async fn a_private_asides_marker_names_its_confidential_room() {
        // Scenario 13's mechanism: an aside told in a #confidential room surfaces flagged with the room
        // and its confidentiality — the cross-context signal the agent reasons over.
        let mut corpus = Corpus::new();
        let erin = corpus
            .add(
                Namespace::Person.with_name("erin"),
                "A colleague",
                "We work together",
                1_000,
            )
            .await;
        let marcus = corpus
            .add(
                Namespace::Person.with_name("marcus"),
                "A teammate",
                "On the same team",
                1_000,
            )
            .await;

        // A #confidential context — the #leads room.
        let leads = MemoryId::generate();
        corpus
            .commit(
                1_000,
                vec![EventPayload::memory_created(
                    leads,
                    Namespace::Context.with_name("leads"),
                )],
            )
            .await;
        corpus.tag(leads, "confidential", 1_000);

        // Erin, in #leads, says something private about Marcus.
        corpus
            .tell_private_in(marcus, "is being managed out", erin, leads, 1_000)
            .await;

        // Erin present, Marcus absent: Marcus surfaces, the marker naming the room and its confidentiality.
        let hits = corpus
            .query_in("is being managed out", None, &[], &[erin], 1_000, 5)
            .await;
        let marcus_hit = hits
            .iter()
            .find(|hit| hit.memory.id == marcus)
            .expect("Marcus surfaces via the aside");
        assert_eq!(
            marcus_hit.marker.as_deref(),
            Some("[teller-private, told by person/erin in #leads (confidential)]")
        );
    }

    #[tokio::test]
    async fn a_stale_description_still_yields_a_legible_snippet() {
        // The legibility guarantee: even when a memory's description is empty (the describer has not
        // caught up), a content match still carries a snippet of what matched — so the hit is
        // triageable rather than a bare name.
        let mut corpus = Corpus::new();
        let devin = corpus
            .add(
                Namespace::Person.with_name("devin"),
                "",
                "owns the rollback and cuts billing over to Stripe on July 20th",
                1_000,
            )
            .await;

        let hits = corpus
            .query_in("cut billing over to Stripe", None, &[], &[], 1_000, 5)
            .await;
        let hit = hits
            .iter()
            .find(|hit| hit.memory.id == devin)
            .expect("Devin surfaces on the content match");
        assert!(
            hit.memory.description.is_empty(),
            "the description is stale/empty, so it cannot carry the hit"
        );
        let snippet = hit
            .snippet
            .as_deref()
            .expect("a matched-content snippet stands in for the missing description");
        assert!(
            snippet.contains("Stripe"),
            "the snippet quotes the matched content: {snippet:?}"
        );
    }

    #[tokio::test]
    async fn a_private_entry_never_appears_in_a_snippet_for_an_excluded_present_set() {
        // The snippet must inherit the same visibility filter as the hit: a private aside's content
        // may never be quoted for a present set that excludes its audience, even though the subject
        // may still surface via public vectors.
        let mut corpus = Corpus::new();
        let erin = corpus
            .add(
                Namespace::Person.with_name("erin"),
                "A colleague",
                "We work together",
                1_000,
            )
            .await;
        let marcus = corpus
            .add(
                Namespace::Person.with_name("marcus"),
                "A teammate",
                "On the same team",
                1_000,
            )
            .await;
        corpus
            .tell_private(marcus, "the quarterly review went badly", erin, 1_000)
            .await;

        // Erin absent: the aside's teller is not present, so it never surfaces — and no snippet on any
        // hit may quote its content.
        let hits = corpus
            .query_in(
                "the quarterly review went badly",
                None,
                &[],
                &[marcus],
                1_000,
                5,
            )
            .await;
        assert!(
            hits.iter().all(|hit| hit
                .snippet
                .as_deref()
                .is_none_or(|snippet| !snippet.contains("quarterly review"))),
            "a private aside leaked into a snippet: {hits:?}"
        );

        // Positive control: with Erin present the aside surfaces, and its snippet is legible.
        let hits = corpus
            .query_in(
                "the quarterly review went badly",
                None,
                &[],
                &[erin],
                1_000,
                5,
            )
            .await;
        let marcus_hit = hits
            .iter()
            .find(|hit| hit.memory.id == marcus)
            .expect("Marcus surfaces via the aside");
        assert!(
            marcus_hit
                .snippet
                .as_deref()
                .expect("the surviving aside carries a snippet")
                .contains("quarterly review"),
            "the surfaced aside's snippet quotes its content: {marcus_hit:?}"
        );
    }

    #[tokio::test]
    async fn a_hit_carries_the_resolved_occurrence() {
        // The date-legibility guarantee: a scheduled fact's resolved occurrence rides on the hit, so a
        // recall that renders from the result line keeps the *when* — rather than the date surfacing
        // only if the agent separately drills into `entries()`.
        let mut corpus = Corpus::new();
        let ship = TemporalRef::Day(CivilDate("2026-07-17".into()));
        let migration = corpus
            .add_dated(
                Namespace::Event.with_name("billing-migration"),
                "The billing migration",
                "shipping the billing migration on Friday the 17th",
                ship.clone(),
                1_000,
            )
            .await;

        let hits = corpus
            .query_in("shipping the billing migration", None, &[], &[], 1_000, 5)
            .await;
        let hit = hits
            .iter()
            .find(|hit| hit.memory.id == migration)
            .expect("the migration surfaces on the content match");
        assert_eq!(
            hit.occurred_at.as_ref(),
            Some(&ship),
            "the hit carries the resolved occurrence: {hit:?}"
        );
    }

    #[tokio::test]
    async fn an_authored_date_outranks_a_newer_extracted_date_on_a_hit() {
        // Authored is ground truth; extracted is inference. An older authored July date must ride on
        // the hit over a *newer* extracted June date on a sibling entry — the exact shadowing the
        // temporal-fidelity defect produced, where "that weekend" was resolved against the clock and
        // the wrong June range shadowed the stated July cutover.
        let mut corpus = Corpus::new();
        let id = MemoryId::generate();
        let authored = EntryId::generate();
        let extracted = EntryId::generate();
        let july = TemporalRef::Day(CivilDate("2026-07-20".into()));
        let june = TemporalRef::Day(CivilDate("2026-06-08".into()));
        // Entry 1 carries the authored July cutover; entry 2 (newer) is appended untimed.
        corpus
            .commit(
                1_000,
                vec![
                    EventPayload::memory_created(id, Namespace::Event.with_name("billing-cutover")),
                    EventPayload::MemoryContentAppended {
                        id,
                        entry_id: authored,
                        asserted_at: Timestamp::from_millis(1_000),
                        occurred_at: Some(july.clone()),
                        text: "cut billing over to the new Stripe integration".to_owned(),
                        told_by: Teller::Agent,
                        told_in: None,
                        visibility: Visibility::Public,
                    },
                    EventPayload::MemoryContentAppended {
                        id,
                        entry_id: extracted,
                        asserted_at: Timestamp::from_millis(1_000),
                        occurred_at: None,
                        text: "Devin owns the rollback and makes the call that weekend".to_owned(),
                        told_by: Teller::Agent,
                        told_in: None,
                        visibility: Visibility::Public,
                    },
                ],
            )
            .await;
        // The extraction pass later (mis)resolves the second entry to a June date against the clock.
        corpus
            .commit(
                2_000,
                vec![EventPayload::entry_temporal_resolved(
                    id,
                    extracted,
                    june.clone(),
                    None,
                )],
            )
            .await;

        let hits = corpus
            .query_in(
                "cut billing over to Stripe rollback",
                None,
                &[],
                &[],
                2_000,
                5,
            )
            .await;
        let hit = hits
            .iter()
            .find(|hit| hit.memory.id == id)
            .expect("the cutover surfaces on the content match");
        assert_eq!(
            hit.occurred_at.as_ref(),
            Some(&july),
            "the authored July date must outrank the newer extracted June date: {hit:?}"
        );
    }

    #[tokio::test]
    async fn an_extracted_date_still_surfaces_when_no_authored_date_exists() {
        // The preference falls back rather than dropping the date: with no authored occurrence in the
        // class, the most recent visible extracted occurrence still rides on the hit.
        let mut corpus = Corpus::new();
        let id = MemoryId::generate();
        let entry = EntryId::generate();
        let june = TemporalRef::Day(CivilDate("2026-06-08".into()));
        corpus
            .commit(
                1_000,
                vec![
                    EventPayload::memory_created(id, Namespace::Event.with_name("rollback-call")),
                    EventPayload::MemoryContentAppended {
                        id,
                        entry_id: entry,
                        asserted_at: Timestamp::from_millis(1_000),
                        occurred_at: None,
                        text: "Devin makes the rollback call that weekend".to_owned(),
                        told_by: Teller::Agent,
                        told_in: None,
                        visibility: Visibility::Public,
                    },
                ],
            )
            .await;
        corpus
            .commit(
                2_000,
                vec![EventPayload::entry_temporal_resolved(
                    id,
                    entry,
                    june.clone(),
                    None,
                )],
            )
            .await;

        let hits = corpus
            .query_in("Devin makes the rollback call", None, &[], &[], 2_000, 5)
            .await;
        let hit = hits
            .iter()
            .find(|hit| hit.memory.id == id)
            .expect("the rollback call surfaces on the content match");
        assert_eq!(
            hit.occurred_at.as_ref(),
            Some(&june),
            "the extracted date surfaces when there is no authored one: {hit:?}"
        );
    }

    #[tokio::test]
    async fn a_private_entrys_date_never_leaks_into_a_hit() {
        // The occurrence inherits the snippet's visibility filter: a date on a private aside may never
        // ride on a hit for a present set that excludes its audience, even though the subject may still
        // surface via public vectors.
        let mut corpus = Corpus::new();
        let erin = corpus
            .add(
                Namespace::Person.with_name("erin"),
                "A colleague",
                "We work together",
                1_000,
            )
            .await;
        let marcus = corpus
            .add(
                Namespace::Person.with_name("marcus"),
                "A teammate",
                "On the same team",
                1_000,
            )
            .await;
        // The only dated entry on Marcus is Erin's private aside, so any date on his hit can come only
        // from it — an unambiguous probe for a leak.
        let review = TemporalRef::Day(CivilDate("2026-07-20".into()));
        corpus
            .tell_private_dated(
                marcus,
                "his review is on the 20th",
                erin,
                review.clone(),
                1_000,
            )
            .await;

        // Erin absent: the aside is not visible, so no hit may carry its date.
        let hits = corpus
            .query_in("his review is on the 20th", None, &[], &[marcus], 1_000, 5)
            .await;
        assert!(
            hits.iter().all(|hit| hit.occurred_at.is_none()),
            "a private aside's date leaked onto a hit: {hits:?}"
        );

        // Positive control: with Erin present the aside surfaces, so its date rides on Marcus's hit.
        let hits = corpus
            .query_in("his review is on the 20th", None, &[], &[erin], 1_000, 5)
            .await;
        let marcus_hit = hits
            .iter()
            .find(|hit| hit.memory.id == marcus)
            .expect("Marcus surfaces via the aside");
        assert_eq!(
            marcus_hit.occurred_at.as_ref(),
            Some(&review),
            "the surfaced aside's date rides on the hit: {marcus_hit:?}"
        );
    }

    #[test]
    fn settings_round_trip_through_the_log() {
        let mut store = MemoryStore::new();
        let seed = SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "A companion.".to_owned(),
            seed_entries: Vec::new(),
        };
        genesis::rollout(
            &mut store,
            &ManualClock::new(Timestamp::from_millis(1)),
            &seed,
            None,
            &InstanceFeatures::default(),
        )
        .unwrap();

        // Genesis seeds the default snapshot, so folding the log back yields exactly Settings::default().
        assert_eq!(Settings::from_store(&store).unwrap(), Settings::default());
    }
}
