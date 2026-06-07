//! Multi-signal search tests. The fake embedder isn't semantic, but it is deterministic — the same
//! text embeds to the same vector — so querying a memory's exact description gives it cosine 1,
//! which lets us exercise the semantic signal and the blend without a real model.

#![cfg(feature = "sqlite")]

use zuihitsu::{
    Embedder, EntryId, FakeEmbedder, Graph, InMemoryVectorIndex, Indexer, ManualClock, MemoryId,
    MemoryName, MemoryStore, SeedSelf, Settings, Store, TagName, Teller, Timestamp, Visibility,
    event::EventPayload,
    genesis::{self},
    search,
    search::{SearchHit, SearchQuery},
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
    async fn add(&mut self, name: &str, description: &str, content: &str, at_ms: i64) -> MemoryId {
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

    /// As [`tell_private`], but told in a specific room (`told_in`), so the surfaced marker can name
    /// it.
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
