//! Multi-signal search tests. The fake embedder isn't semantic, but it is deterministic — the same
//! text embeds to the same vector — so querying a memory's exact description gives it cosine 1,
//! which lets us exercise the semantic signal and the blend without a real model.

#![cfg(feature = "sqlite")]

use zuihitsu::{
    Embedder, EntryId, FakeEmbedder, Graph, InMemoryVectorIndex, ManualClock, MemoryId, MemoryName,
    MemoryStore, SeedSelf, Settings, Store, TagName, Timestamp, VectorId, VectorIndex,
    VectorRecord,
    event::EventPayload,
    genesis::{self},
    search,
    search::SearchQuery,
};

const DIMS: usize = 32;
const DAY_MS: i64 = 86_400_000;

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

    /// Add a memory with a description (embedded into the index) and one content entry, asserted at
    /// `asserted_at_ms`, and bring the graph up to date.
    async fn add(
        &mut self,
        name: &str,
        description: &str,
        content: &str,
        asserted_at_ms: i64,
    ) -> MemoryId {
        let id = MemoryId::generate();
        let at = Timestamp::from_millis(asserted_at_ms);
        self.store
            .append(
                at,
                vec![
                    EventPayload::MemoryCreated {
                        id,
                        name: MemoryName::new(name),
                    },
                    EventPayload::MemoryContentAppended {
                        id,
                        entry_id: EntryId::generate(),
                        asserted_at: at,
                        text: content.to_owned(),
                    },
                    EventPayload::MemoryDescriptionRegenerated {
                        id,
                        new_text: description.to_owned(),
                        produced_by: None,
                    },
                ],
            )
            .unwrap();
        self.graph.materialize_from(&self.store).unwrap();
        let embedding = self
            .embedder
            .embed(&[description.to_owned()])
            .await
            .unwrap()
            .remove(0);
        self.index
            .upsert(VectorRecord {
                id: VectorId::new(id.0.to_string()),
                embedding,
                model_id: self.embedder.model_id().into(),
            })
            .unwrap();
        id
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
        self.query_in(text, None, &[], now_ms, limit).await
    }

    async fn query_in(
        &self,
        text: &str,
        namespace: Option<&str>,
        tags: &[TagName],
        now_ms: i64,
        limit: usize,
    ) -> Vec<MemoryId> {
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
        .into_iter()
        .map(|hit| hit.memory.id)
        .collect()
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
            100 * DAY_MS,
        )
        .await;

    let ranked = corpus.query("shared topic text", 100 * DAY_MS, 5).await;
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

    let ranked = corpus
        .query_in(
            "shared topic text",
            None,
            &[TagName::new("climbing")],
            1_000,
            5,
        )
        .await;
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
    let ranked = corpus
        .query_in("shared marker text", Some("person/"), &[], 1_000, 5)
        .await;
    assert_eq!(ranked, vec![dave]);
}

#[tokio::test]
async fn an_empty_corpus_returns_nothing() {
    // No memories, no vectors: nothing to rank, whatever the query.
    let corpus = Corpus::new();
    let ranked = corpus.query("anything at all", 1_000, 5).await;
    assert!(ranked.is_empty());
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
