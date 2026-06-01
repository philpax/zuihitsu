//! The sqlite-vec backend: nearest-neighbour ranking, upsert-replaces semantics, and the
//! dimension-mismatch guard. Uses the deterministic fake embedder so no model is needed.

#![cfg(feature = "sqlite")]

use zuihitsu::{Embedder, FakeEmbedder, SqliteVectorIndex, VectorError, VectorId, VectorIndex};

const DIMS: usize = 32;

async fn vector(embedder: &FakeEmbedder, text: &str) -> Vec<f32> {
    embedder.embed(&[text.to_owned()]).await.unwrap().remove(0)
}

#[tokio::test]
async fn ranks_nearest_first_and_replaces_on_reinsert() {
    let embedder = FakeEmbedder::new(DIMS);
    let mut index = SqliteVectorIndex::open_in_memory(DIMS).unwrap();
    assert!(index.is_empty().unwrap());

    for text in ["climbing gym", "sourdough bread", "tax return"] {
        index
            .upsert(VectorId::new(text), vector(&embedder, text).await)
            .unwrap();
    }
    assert_eq!(index.len().unwrap(), 3);

    let query = vector(&embedder, "climbing gym").await;
    let hits = index.search(&query, 2).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, VectorId::new("climbing gym"));
    // The exact match is cosine similarity ~1 (within f32 round-trip error through the JSON binding).
    assert!((hits[0].score - 1.0).abs() < 1e-3);
    assert!(hits[0].score >= hits[1].score);

    // Re-inserting an existing id replaces rather than duplicates.
    index
        .upsert(
            VectorId::new("climbing gym"),
            vector(&embedder, "climbing gym").await,
        )
        .unwrap();
    assert_eq!(index.len().unwrap(), 3);

    index.remove(&VectorId::new("climbing gym")).unwrap();
    assert_eq!(index.len().unwrap(), 2);
    assert!(
        index
            .search(&query, 5)
            .unwrap()
            .iter()
            .all(|hit| hit.id != VectorId::new("climbing gym"))
    );
}

#[tokio::test]
async fn rejects_a_wrong_dimensioned_embedding() {
    let mut index = SqliteVectorIndex::open_in_memory(DIMS).unwrap();
    let wrong = vec![0.0_f32; DIMS + 1];
    assert!(matches!(
        index.upsert(VectorId::new("x"), wrong.clone()),
        Err(VectorError::Dimension {
            expected: DIMS,
            found,
        }) if found == DIMS + 1
    ));
    assert!(matches!(
        index.search(&wrong, 3),
        Err(VectorError::Dimension { .. })
    ));
}
