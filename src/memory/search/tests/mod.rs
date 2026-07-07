use super::{SALIENCE_CAP, SearchHit, SearchQuery, recency_bonus, search};
use crate::{
    InstanceFeatures,
    agent::genesis::{self, SeedSelf},
    clock::ManualClock,
    event::{Cardinality, Event, EventPayload, LinkSource, Teller, Visibility, Volatility},
    graph::Graph,
    ids::{EntryId, MemoryId, MemoryName, Namespace, Seq},
    memory::memory_block::LinkDirection,
    model::{
        embed::{Embedder, FakeEmbedder},
        index::Indexer,
    },
    settings::{SearchSettings, Settings},
    store::{MemoryStore, Store},
    time::{CivilDate, TemporalRef, Timestamp},
    vector::InMemoryVectorIndex,
    vocabulary::{RelationName, TagName},
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
    async fn tell_private(&mut self, memory: MemoryId, text: &str, teller: MemoryId, at_ms: i64) {
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

    /// Register `relation` (idempotent — the materializer upserts) and create a `from -> to` edge
    /// under it, so a search hit's salient relations have something to carry.
    async fn link(&mut self, from: MemoryId, to: MemoryId, relation: &str, at_ms: i64) {
        self.commit(
            at_ms,
            vec![
                EventPayload::LinkTypeRegistered {
                    name: RelationName::new(relation),
                    inverse: RelationName::new(&format!("{relation}_of")),
                    from_card: Cardinality::Many,
                    to_card: Cardinality::Many,
                    symmetric: false,
                    reflexive: false,
                    description: format!("the {relation} relation"),
                },
                EventPayload::LinkCreated {
                    from,
                    to,
                    relation: RelationName::new(relation),
                    source: LinkSource::Agent,
                    told_by: Some(Teller::Agent),
                },
            ],
        )
        .await;
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

mod occurrences;
mod privacy;
mod ranking;
