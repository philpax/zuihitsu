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

    // The topic matches lexically and semantically, but the [`Namespace::Person`] prefix excludes
    // it.
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
    // public vectors; the load-bearing fact is that the private aside does not surface.)
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
    // the hit over a *newer* extracted June date on a sibling entry — the exact shadowing that
    // occurs when a relative phrase like "that weekend" is resolved against the clock and the
    // wrong June range shadows the stated July cutover.
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

#[tokio::test]
async fn a_hit_carries_its_salient_relations_people_first() {
    // The informed-creation surface: a hit for a linked memory passively carries its most salient
    // relations, people first, so a search for the book club shows the cast already on it — the
    // recognition signal that steers a recall toward reuse over a name-guessed duplicate.
    let mut corpus = Corpus::new();
    let club = corpus
        .add(
            Namespace::Event.with_name("book_club"),
            "The monthly book club",
            "we discussed the book",
            1_000,
        )
        .await;
    let maya = corpus
        .add(
            Namespace::Person.with_name("maya"),
            "A reader",
            "reads a lot",
            1_000,
        )
        .await;
    let nadia = corpus
        .add(
            Namespace::Person.with_name("nadia"),
            "A reader",
            "reads a lot",
            1_000,
        )
        .await;
    let venue = corpus
        .add(
            Namespace::Topic.with_name("library"),
            "The venue",
            "meets there",
            1_000,
        )
        .await;
    let snacks = corpus
        .add(
            Namespace::Topic.with_name("snacks"),
            "The snacks",
            "brings snacks",
            1_000,
        )
        .await;

    // Link the two non-person memories first (older rows), then the two people (newest rows). With
    // person-first salience the people float ahead of the more-recent non-person, and the cap of 3
    // elides the last non-person behind a `(+1 more)` note.
    corpus.link(venue, club, "hosts", 1_000).await;
    corpus.link(snacks, club, "supplies", 1_000).await;
    corpus.link(maya, club, "participates_in", 1_000).await;
    corpus.link(nadia, club, "participates_in", 1_000).await;

    let hits = corpus
        .query_in("The monthly book club", None, &[], &[], 1_000, 8)
        .await;
    let hit = hits
        .iter()
        .find(|hit| hit.memory.id == club)
        .expect("the book club surfaces on its description");

    assert_eq!(hit.relations.len(), SALIENCE_CAP);
    assert_eq!(
        hit.more_relations, 1,
        "one salient link elided past the cap"
    );
    let person = Namespace::Person.prefix();
    assert!(
        hit.relations[0].other_name.as_str().starts_with(person)
            && hit.relations[1].other_name.as_str().starts_with(person),
        "people anchor identity, so they come first: {:?}",
        hit.relations
    );
    // The two people participate in the club — the edge runs into the club's class, so it reads as
    // incoming, which the hit line renders with a `←`.
    assert!(
        hit.relations
            .iter()
            .take(2)
            .all(|relation| relation.direction == LinkDirection::Incoming
                && relation.relation == RelationName::new("participates_in")),
    );
    let names: Vec<&str> = hit
        .relations
        .iter()
        .map(|relation| relation.other_name.as_str())
        .collect();
    assert!(names.contains(&MemoryName::from(Namespace::Person.with_name("maya")).as_str()));
    assert!(names.contains(&MemoryName::from(Namespace::Person.with_name("nadia")).as_str()));
    // The third salient link is the most-recently created non-person (snacks over library).
    assert_eq!(
        hit.relations[2].other_name.as_str(),
        MemoryName::from(Namespace::Topic.with_name("snacks")).as_str(),
    );
}

#[tokio::test]
async fn an_unlinked_hit_carries_no_relations() {
    // A memory with no out-of-class links carries no salient relations, so the hit line stays bare
    // rather than trailing an empty `— ` segment.
    let mut corpus = Corpus::new();
    let solo = corpus
        .add(
            Namespace::Topic.with_name("sourdough"),
            "Naturally leavened bread",
            "fed the starter",
            1_000,
        )
        .await;

    let hits = corpus
        .query_in("Naturally leavened bread", None, &[], &[], 1_000, 8)
        .await;
    let hit = hits
        .iter()
        .find(|hit| hit.memory.id == solo)
        .expect("the topic surfaces on its description");
    assert!(hit.relations.is_empty());
    assert_eq!(hit.more_relations, 0);
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
