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
    event::{Visibility, Volatility},
    graph::{Graph, GraphError, MemoryView},
    ids::MemoryId,
    model::index::VectorKey,
    settings::SearchSettings,
    time::{self, Timestamp},
    vector::{VectorError, VectorIndex},
    vocabulary::TagName,
};

use super::visibility;

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
                    let teller = graph.teller_display(&entry.told_by)?;
                    let room = graph.marker_room(entry.told_in)?;
                    markers.insert(
                        memory.id,
                        visibility::teller_private_marker(&teller, room.as_ref()),
                    );
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
        agent::genesis::{self, SeedSelf},
        clock::ManualClock,
        event::{Event, EventPayload, Teller, Visibility, Volatility},
        graph::Graph,
        ids::{EntryId, MemoryId, MemoryName, Seq},
        model::{
            embed::{Embedder, FakeEmbedder},
            index::Indexer,
        },
        settings::{SearchSettings, Settings},
        store::{MemoryStore, Store},
        time::{TemporalRef, Timestamp},
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
                EventPayload::MemoryCreated {
                    id,
                    name: MemoryName::new("topic/dated"),
                },
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
                    EventPayload::MemoryVolatilitySet { id, volatility },
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
            name: &str,
            description: &str,
            content: &str,
            at_ms: i64,
        ) -> MemoryId {
            let id = MemoryId::generate();
            let at = Timestamp::from_millis(at_ms);
            self.commit(
                at_ms,
                vec![
                    EventPayload::MemoryCreated {
                        id,
                        name: MemoryName::new(name),
                    },
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
                    EventPayload::MemoryDescriptionRegenerated {
                        id,
                        new_text: description.to_owned(),
                        produced_by: None,
                    },
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

        /// Create `tag` and apply it to `id`. Only one memory per tag in these tests, so the create is
        /// unconditional.
        fn tag(&mut self, id: MemoryId, tag: &str, at_ms: i64) {
            self.store
                .append(
                    Timestamp::from_millis(at_ms),
                    vec![
                        EventPayload::TagCreated {
                            name: TagName::new(tag),
                            description: format!("about {tag}"),
                        },
                        EventPayload::TagAppliedToMemory {
                            memory: id,
                            tag: TagName::new(tag),
                        },
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
                "person/dave",
                "An avid rock climber",
                "We met bouldering",
                1_000,
            )
            .await;
        corpus
            .add(
                "person/erin",
                "A tax accountant",
                "She filed my return",
                1_000,
            )
            .await;
        corpus
            .add(
                "topic/sourdough",
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
            .add("topic/stale", "shared topic text", "shared topic text", 0)
            .await;
        let fresh = corpus
            .add(
                "topic/fresh",
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
                "topic/plain",
                "shared topic text",
                "shared topic text",
                1_000,
            )
            .await;
        let tagged = corpus
            .add(
                "topic/tagged",
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
                "person/dave",
                "shared marker text",
                "shared marker text",
                1_000,
            )
            .await;
        corpus
            .add(
                "topic/marker",
                "shared marker text",
                "shared marker text",
                1_000,
            )
            .await;

        // The topic matches lexically and semantically, but the person/ prefix excludes it.
        let ranked: Vec<MemoryId> = corpus
            .query_in("shared marker text", Some("person/"), &[], &[], 1_000, 5)
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
        // Scenario 17: Erin's private aside about Phil is embedded as an entry vector. The query matches
        // only that aside (the wording appears nowhere public), so Phil surfaces solely through it.
        let mut corpus = Corpus::new();
        let erin = corpus
            .add("person/erin", "A colleague", "We work together", 1_000)
            .await;
        let phil = corpus
            .add("person/phil", "A teammate", "On the same team", 1_000)
            .await;
        corpus
            .tell_private(phil, "the quarterly review went badly", erin, 1_000)
            .await;

        // Erin present, Phil absent: the aside surfaces Phil, flagged teller-private.
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
        let phil_hit = hits
            .iter()
            .find(|hit| hit.memory.id == phil)
            .expect("Phil surfaces via the aside");
        let marker = phil_hit.marker.as_deref().expect("a teller-private marker");
        assert!(marker.contains("teller-private"));
        assert!(marker.contains("person/erin"));

        // Phil present too: the subject-guard suppresses the aside. It's the *same* predicate as the
        // brief, so the private entry survives in no hit — no result carries a teller-private marker.
        // (The fake embedder gives every text a faint nonzero cosine, so Phil still appears via his
        // public vectors; the load-bearing fact is that the private aside no longer surfaces.)
        let hits = corpus
            .query_in(
                "the quarterly review went badly",
                None,
                &[],
                &[erin, phil],
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
            .add("person/erin", "A colleague", "We work together", 1_000)
            .await;
        let phil = corpus
            .add("person/phil", "A teammate", "On the same team", 1_000)
            .await;

        // A #confidential context — the #leads room.
        let leads = MemoryId::generate();
        corpus
            .commit(
                1_000,
                vec![EventPayload::MemoryCreated {
                    id: leads,
                    name: MemoryName::new("context/leads"),
                }],
            )
            .await;
        corpus.tag(leads, "confidential", 1_000);

        // Erin, in #leads, says something private about Phil.
        corpus
            .tell_private_in(phil, "is being managed out", erin, leads, 1_000)
            .await;

        // Erin present, Phil absent: Phil surfaces, the marker naming the room and its confidentiality.
        let hits = corpus
            .query_in("is being managed out", None, &[], &[erin], 1_000, 5)
            .await;
        let phil_hit = hits
            .iter()
            .find(|hit| hit.memory.id == phil)
            .expect("Phil surfaces via the aside");
        assert_eq!(
            phil_hit.marker.as_deref(),
            Some("[teller-private, told by person/erin in #leads (confidential)]")
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
        )
        .unwrap();

        // Genesis seeds the default snapshot, so folding the log back yields exactly Settings::default().
        assert_eq!(Settings::from_store(&store).unwrap(), Settings::default());
    }
}
