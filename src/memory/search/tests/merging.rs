use super::*;

/// Merge two stubs into one `same_as` class (operator-adjudicated), mirroring the graph merge tests'
/// payload pattern: register the symmetric `same_as` relation, then link the pair.
async fn merge(corpus: &mut Corpus, a: MemoryId, b: MemoryId, at_ms: i64) {
    corpus
        .commit(
            at_ms,
            vec![
                EventPayload::LinkTypeRegistered {
                    name: RelationName::SameAs,
                    inverse: RelationName::SameAs,
                    from_card: Cardinality::Many,
                    to_card: Cardinality::Many,
                    symmetric: true,
                    reflexive: false,
                    description: String::new(),
                },
                EventPayload::link_created(
                    a,
                    b,
                    RelationName::SameAs,
                    LinkPosture {
                        source: LinkSource::Operator,
                        told_by: None,
                        told_in: None,
                        visibility: Visibility::Public,
                    },
                ),
            ],
        )
        .await;
}

/// Two stubs of one person, each with distinct public content the query matches; the chat stub matches
/// "kelp" more strongly, so its extract is the evidence.
async fn rowan_stubs(corpus: &mut Corpus) -> (MemoryId, MemoryId) {
    let direct = corpus
        .add(
            Namespace::Person.with_name("rowan@direct"),
            "Coordinates the harbour survey",
            "Kelp planning notes from Rowan at the harbour.",
            1_000,
        )
        .await;
    let chat = corpus
        .add(
            Namespace::Person.with_name("rowan@chat"),
            "Pings about logistics",
            "Kelp kelp kelp raccoon logistics from the night shift.",
            1_000,
        )
        .await;
    (direct, chat)
}

#[tokio::test]
async fn a_merged_class_surfaces_once_under_its_primary() {
    let mut corpus = Corpus::new();
    let (direct, chat) = rowan_stubs(&mut corpus).await;
    merge(&mut corpus, direct, chat, 1_000).await;

    let hits = corpus.query_in("kelp", None, &[], &[], 1_000, 5).await;
    assert_eq!(hits.len(), 1, "the merged class surfaces as one hit");
    assert_eq!(
        hits[0].memory.id,
        direct.min(chat),
        "the hit is keyed by the class primary"
    );
    // The stronger-matching member (chat, which repeats the term) supplies the snippet, so the class
    // carries its strongest evidence even though it surfaces under the primary.
    let snippet = hits[0].snippet.as_deref().unwrap_or_default();
    assert!(
        snippet.contains("raccoon"),
        "the hit carries the strongest member's snippet: {snippet:?}"
    );
}

#[tokio::test]
async fn an_unmerged_pair_still_yields_two_hits() {
    let mut corpus = Corpus::new();
    let (direct, chat) = rowan_stubs(&mut corpus).await;

    let hits = corpus.query_in("kelp", None, &[], &[], 1_000, 5).await;
    let ids: Vec<MemoryId> = hits.iter().map(|hit| hit.memory.id).collect();
    assert_eq!(
        hits.len(),
        2,
        "without a merge, both stubs surface: {ids:?}"
    );
    assert!(
        ids.contains(&direct) && ids.contains(&chat),
        "each stub is its own hit: {ids:?}"
    );
}
