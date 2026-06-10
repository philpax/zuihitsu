//! The vector-index seam: nearest-neighbour search over embeddings. [`InMemoryVectorIndex`] is both
//! the test fake and a usable small-scale implementation; [`SqliteVectorIndex`] is the real backend,
//! sqlite-vec over a `vec0` virtual table (spec §Storage → vector store, §Testability). Local, so
//! synchronous. The seam is fallible because the real backend can fail; the in-memory one never does.

mod in_memory;
mod sqlite;

pub use in_memory::InMemoryVectorIndex;
pub use sqlite::SqliteVectorIndex;

use smol_str::SmolStr;

use crate::{ids::Seq, model::embed::Embedding};

/// A stored vector's key. A string, so both entry and description vectors can share one index; the
/// entry-vs-description distinction and visibility metadata arrive when search becomes
/// visibility-aware (Stage 5/6).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

/// A vector to store, with the provenance the index keeps alongside it. `model_id` is the embedding
/// model that produced the vector, recorded **at creation** so a mixed-embedding-space state is
/// detectable rather than silent (spec §Storage → vector store); retrofitting it would itself be a
/// full re-embed. Visibility metadata joins this struct when search becomes visibility-aware.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorRecord {
    pub id: VectorId,
    pub embedding: Embedding,
    pub model_id: SmolStr,
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

impl From<rusqlite::Error> for VectorError {
    fn from(error: rusqlite::Error) -> Self {
        VectorError::Backend(error.to_string())
    }
}

/// Approximate (here, exact) nearest-neighbour search over embeddings. `Send` so the index can live
/// behind the shared [`Engine`](crate::engine::Engine)'s mutex and be driven from the background
/// indexer task; its backends (a sqlite `Connection`, an in-memory map) are `Send`.
pub trait VectorIndex: Send {
    /// Insert or replace a vector and its provenance.
    fn upsert(&mut self, record: VectorRecord) -> Result<(), VectorError>;

    /// Remove the vector for `id`, if present.
    fn remove(&mut self, id: &VectorId) -> Result<(), VectorError>;

    fn len(&self) -> Result<usize, VectorError>;

    fn is_empty(&self) -> Result<bool, VectorError> {
        Ok(self.len()? == 0)
    }

    /// The `k` stored vectors most similar to `query` by cosine similarity, best first.
    fn search(&self, query: &[f32], k: usize) -> Result<Vec<ScoredHit>, VectorError>;

    /// The highest log `Seq` the indexer has processed into this index, or `Seq::ZERO` if none.
    /// Catch-up resumes from `cursor().next()`, so a persistent index doesn't re-embed the log on
    /// every boot; an ephemeral one reports `ZERO` and is rebuilt from the log.
    fn cursor(&self) -> Result<Seq, VectorError>;

    /// Record that the index has processed the log through `seq`. Written after the batch's vectors
    /// are durable, so a crash in between re-processes that batch (an idempotent re-embed) rather
    /// than skipping it.
    fn set_cursor(&mut self, seq: Seq) -> Result<(), VectorError>;
}
