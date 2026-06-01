//! The vector-index seam: nearest-neighbour search over embeddings. [`InMemoryVectorIndex`] is both
//! the test fake and a usable small-scale implementation; [`SqliteVectorIndex`] is the real backend,
//! sqlite-vec over a `vec0` virtual table (spec §Storage → vector store, §Testability). Local, so
//! synchronous. The seam is fallible because the real backend can fail; the in-memory one never does.

mod in_memory;
#[cfg(feature = "sqlite")]
mod sqlite;

pub use in_memory::InMemoryVectorIndex;
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteVectorIndex;

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

/// A vector-index failure: a backend error, or an embedding whose dimensionality does not match the
/// index's.
#[derive(Debug)]
pub enum VectorError {
    Backend(String),
    Dimension { expected: usize, found: usize },
}

impl std::fmt::Display for VectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VectorError::Backend(message) => write!(f, "vector index: {message}"),
            VectorError::Dimension { expected, found } => write!(
                f,
                "vector index: expected a {expected}-dimensional embedding, got {found}"
            ),
        }
    }
}

impl std::error::Error for VectorError {}

/// Approximate (here, exact) nearest-neighbour search over embeddings.
pub trait VectorIndex {
    /// Insert or replace the vector for `id`.
    fn upsert(&mut self, id: VectorId, vector: Embedding) -> Result<(), VectorError>;

    /// Remove the vector for `id`, if present.
    fn remove(&mut self, id: &VectorId) -> Result<(), VectorError>;

    fn len(&self) -> Result<usize, VectorError>;

    fn is_empty(&self) -> Result<bool, VectorError> {
        Ok(self.len()? == 0)
    }

    /// The `k` stored vectors most similar to `query` by cosine similarity, best first.
    fn search(&self, query: &[f32], k: usize) -> Result<Vec<ScoredHit>, VectorError>;
}
