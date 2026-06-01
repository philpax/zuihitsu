//! Brute-force in-memory vector index: the test fake, and a usable implementation at personal-agent
//! scale. Swapped for [`SqliteVectorIndex`](super::SqliteVectorIndex) when persistence is needed.
//! Infallible, so every operation returns `Ok`.

use super::{ScoredHit, VectorError, VectorId, VectorIndex, VectorRecord};
use crate::ids::Seq;

#[derive(Default)]
pub struct InMemoryVectorIndex {
    records: Vec<VectorRecord>,
    cursor: Seq,
}

impl InMemoryVectorIndex {
    pub fn new() -> InMemoryVectorIndex {
        InMemoryVectorIndex::default()
    }
}

impl VectorIndex for InMemoryVectorIndex {
    fn upsert(&mut self, record: VectorRecord) -> Result<(), VectorError> {
        match self.records.iter_mut().find(|slot| slot.id == record.id) {
            Some(slot) => *slot = record,
            None => self.records.push(record),
        }
        Ok(())
    }

    fn remove(&mut self, id: &VectorId) -> Result<(), VectorError> {
        self.records.retain(|record| &record.id != id);
        Ok(())
    }

    fn len(&self) -> Result<usize, VectorError> {
        Ok(self.records.len())
    }

    fn search(&self, query: &[f32], k: usize) -> Result<Vec<ScoredHit>, VectorError> {
        let mut hits: Vec<ScoredHit> = self
            .records
            .iter()
            .map(|record| ScoredHit {
                id: record.id.clone(),
                score: cosine(query, &record.embedding),
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(k);
        Ok(hits)
    }

    fn cursor(&self) -> Result<Seq, VectorError> {
        Ok(self.cursor)
    }

    fn set_cursor(&mut self, seq: Seq) -> Result<(), VectorError> {
        self.cursor = seq;
        Ok(())
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
