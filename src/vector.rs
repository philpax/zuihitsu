//! The vector-index seam: nearest-neighbour search over embeddings. The real index (sqlite-vec)
//! lands in Stage 5/11; the in-memory brute-force index here is both the test fake and a usable
//! small-scale implementation (spec §Storage → vector store, §Testability). Local, so synchronous.

use smol_str::SmolStr;

use crate::embed::Embedding;

/// A stored vector's key. A string, so both entry and description vectors can share one index; the
/// entry-vs-description distinction and visibility metadata arrive when search becomes
/// visibility-aware (Stage 5/6).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VectorId(pub SmolStr);

impl VectorId {
    pub fn new(id: impl Into<SmolStr>) -> VectorId {
        VectorId(id.into())
    }
}

/// A search result: a stored vector and its cosine similarity to the query, in `[-1, 1]`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoredHit {
    pub id: VectorId,
    pub score: f32,
}

/// Approximate (here, exact) nearest-neighbour search over embeddings.
pub trait VectorIndex {
    /// Insert or replace the vector for `id`.
    fn upsert(&mut self, id: VectorId, vector: Embedding);

    /// Remove the vector for `id`, if present.
    fn remove(&mut self, id: &VectorId);

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The `k` stored vectors most similar to `query` by cosine similarity, best first.
    fn search(&self, query: &[f32], k: usize) -> Vec<ScoredHit>;
}

/// Brute-force in-memory index. Fine at personal-agent scale; swapped for sqlite-vec when needed.
#[derive(Default)]
pub struct InMemoryVectorIndex {
    vectors: Vec<(VectorId, Embedding)>,
}

impl InMemoryVectorIndex {
    pub fn new() -> InMemoryVectorIndex {
        InMemoryVectorIndex::default()
    }
}

impl VectorIndex for InMemoryVectorIndex {
    fn upsert(&mut self, id: VectorId, vector: Embedding) {
        match self
            .vectors
            .iter_mut()
            .find(|(existing, _)| *existing == id)
        {
            Some(slot) => slot.1 = vector,
            None => self.vectors.push((id, vector)),
        }
    }

    fn remove(&mut self, id: &VectorId) {
        self.vectors.retain(|(existing, _)| existing != id);
    }

    fn len(&self) -> usize {
        self.vectors.len()
    }

    fn search(&self, query: &[f32], k: usize) -> Vec<ScoredHit> {
        let mut hits: Vec<ScoredHit> = self
            .vectors
            .iter()
            .map(|(id, vector)| ScoredHit {
                id: id.clone(),
                score: cosine(query, vector),
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(k);
        hits
    }
}

/// Cosine similarity. Mismatched lengths or a zero-magnitude vector score 0.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let magnitude =
        a.iter().map(|x| x * x).sum::<f32>().sqrt() * b.iter().map(|y| y * y).sum::<f32>().sqrt();
    if magnitude > 0.0 {
        dot / magnitude
    } else {
        0.0
    }
}
