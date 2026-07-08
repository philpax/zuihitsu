use super::*;
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
