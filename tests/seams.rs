//! Seam-fake tests: the model, embedder, fetcher, and vector-index seams behave deterministically,
//! so agent-level scenarios in later stages can be exercised entirely in memory (spec §Testability).

use zuihitsu::{
    CannedFetcher, Completion, Embedder, FakeEmbedder, FetchError, Fetcher, GenerateRequest,
    InMemoryVectorIndex, ModelClient, ModelError, ScriptedModel, ToolCall, VectorId, VectorIndex,
};

#[tokio::test]
async fn scripted_model_returns_programmed_steps_then_exhausts() {
    let model = ScriptedModel::new([
        Completion::ToolCalls(vec![ToolCall {
            id: "1".to_owned(),
            name: "run_lua".to_owned(),
            arguments: r#"{"script":"return 1"}"#.to_owned(),
        }]),
        Completion::Reply("done".to_owned()),
    ]);
    let request = GenerateRequest::default();

    assert!(matches!(
        model.generate(&request).await.unwrap(),
        Completion::ToolCalls(_)
    ));
    assert_eq!(
        model.generate(&request).await.unwrap(),
        Completion::Reply("done".to_owned())
    );
    assert!(matches!(
        model.generate(&request).await,
        Err(ModelError::Exhausted)
    ));
}

#[tokio::test]
async fn fake_embedder_is_deterministic_and_sized() {
    let embedder = FakeEmbedder::new(16);
    let hello_a = embedder.embed(&["hello".to_owned()]).await.unwrap();
    let hello_b = embedder.embed(&["hello".to_owned()]).await.unwrap();
    let world = embedder.embed(&["world".to_owned()]).await.unwrap();

    assert_eq!(hello_a[0].len(), 16);
    assert_eq!(hello_a, hello_b); // identical text embeds identically
    assert_ne!(hello_a, world); // distinct text embeds distinctly
}

#[tokio::test]
async fn canned_fetcher_serves_known_urls() {
    let fetcher = CannedFetcher::new().with_page("https://example.com", "# Hello");

    assert_eq!(
        fetcher.fetch_page("https://example.com").await.unwrap(),
        "# Hello"
    );
    assert!(matches!(
        fetcher.fetch_page("https://absent.example").await,
        Err(FetchError::NotFound)
    ));
}

#[tokio::test]
async fn vector_index_ranks_nearest_first() {
    let embedder = FakeEmbedder::new(32);
    let mut index = InMemoryVectorIndex::new();
    for text in ["climbing gym", "sourdough bread", "tax return"] {
        let vector = embedder.embed(&[text.to_owned()]).await.unwrap().remove(0);
        index.upsert(VectorId::new(text), vector);
    }
    assert_eq!(index.len(), 3);

    let query = embedder
        .embed(&["climbing gym".to_owned()])
        .await
        .unwrap()
        .remove(0);
    let hits = index.search(&query, 2);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, VectorId::new("climbing gym")); // exact match ranks first

    index.remove(&VectorId::new("climbing gym"));
    assert_eq!(index.len(), 2);
    assert!(!index.is_empty());
}
