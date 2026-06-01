//! Brute-force in-memory vector index: the test fake, and a usable implementation at personal-agent
//! scale. Swapped for [`SqliteVectorIndex`](super::SqliteVectorIndex) when persistence is needed.
//! Infallible, so every operation returns `Ok`.

use super::{ScoredHit, VectorError, VectorId, VectorIndex};
use crate::embed::Embedding;

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
    fn upsert(&mut self, id: VectorId, vector: Embedding) -> Result<(), VectorError> {
        match self
            .vectors
            .iter_mut()
            .find(|(existing, _)| *existing == id)
        {
            Some(slot) => slot.1 = vector,
            None => self.vectors.push((id, vector)),
        }
        Ok(())
    }

    fn remove(&mut self, id: &VectorId) -> Result<(), VectorError> {
        self.vectors.retain(|(existing, _)| existing != id);
        Ok(())
    }

    fn len(&self) -> Result<usize, VectorError> {
        Ok(self.vectors.len())
    }

    fn search(&self, query: &[f32], k: usize) -> Result<Vec<ScoredHit>, VectorError> {
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
        Ok(hits)
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
