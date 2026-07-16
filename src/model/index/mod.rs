//! The vector indexer: a reactive projection of the log into the vector index.
//!
//! Like the materialized graph, the vector index is a rebuildable projection of the event log — but
//! maintaining it costs a model call, so it runs off the turn's hot path as background work (spec
//! §Storage → vector store, §Concurrency). The indexer consumes committed events and embeds the
//! content they record: a `MemoryContentAppended` embeds that entry, a
//! `MemoryDescriptionRegenerated` (re)embeds the description, and a `MemoryDeleted` drops the
//! memory's description vector (its entry vectors are left, but become unreachable once the memory
//! is soft-deleted, since search resolves an entry hit through its now-absent memory). Each vector
//! is stamped with the embedder's `model_id` at creation so a mixed-embedding-space state stays
//! detectable.
//!
//! Vectors are keyed by [`VectorKey`] so a hit identifies what it is: `mem:<id>` for a description,
//! `entry:<id>` for an entry. Both granularities are embedded (spec §Storage → vector store); the
//! visibility predicate filters entry hits at search time.

mod batch;

#[cfg(test)]
mod tests;

use crate::{
    event::Event,
    ids::{EntryId, MemoryId},
    store::{Store, StoreError, Subscription},
    vector::{VectorError, VectorId, VectorIndex},
};

use crate::model::{ModelError, embed::Embedder};

pub use batch::{Batch, apply_batch, embed_batch};

use ulid::Ulid;

/// What a vector represents, encoded in its [`VectorId`] prefix so a search hit can be mapped back to
/// the memory or entry it came from.
pub enum VectorKey {
    /// A memory's description vector, `mem:<memory-ulid>`.
    Description(MemoryId),
    /// A content entry's vector, `entry:<entry-ulid>`.
    Entry(EntryId),
}

/// The `VectorId` prefixes, named once so the write (`to_vector_id`) and read (`parse`) directions
/// cannot drift.
const DESCRIPTION_PREFIX: &str = "mem:";
const ENTRY_PREFIX: &str = "entry:";

impl VectorKey {
    pub fn to_vector_id(&self) -> VectorId {
        match self {
            VectorKey::Description(id) => VectorId::new(format!("{DESCRIPTION_PREFIX}{}", id.0)),
            VectorKey::Entry(id) => VectorId::new(format!("{ENTRY_PREFIX}{}", id.0)),
        }
    }

    /// Recover the key from a stored [`VectorId`], or `None` for an unrecognized prefix.
    pub fn parse(id: &VectorId) -> Option<VectorKey> {
        let raw = id.0.as_str();
        if let Some(ulid) = raw.strip_prefix(DESCRIPTION_PREFIX) {
            return Ulid::from_string(ulid)
                .ok()
                .map(|ulid| VectorKey::Description(MemoryId(ulid)));
        }
        if let Some(ulid) = raw.strip_prefix(ENTRY_PREFIX) {
            return Ulid::from_string(ulid)
                .ok()
                .map(|ulid| VectorKey::Entry(EntryId(ulid)));
        }
        None
    }
}

/// Maintains the vector index from the event log. Borrows the embedder and the index for the span of
/// a batch; the server owns both and drives the indexer off a [`Subscription`].
pub struct Indexer<'a> {
    embedder: &'a dyn Embedder,
    vectors: &'a mut dyn VectorIndex,
}

impl<'a> Indexer<'a> {
    pub fn new(embedder: &'a dyn Embedder, vectors: &'a mut dyn VectorIndex) -> Indexer<'a> {
        Indexer { embedder, vectors }
    }

    /// Catch the index up to the log: process every event after its cursor, then advance the cursor.
    /// On a fresh (or ephemeral) index the cursor is `Seq::ZERO`, so this embeds the whole log;
    /// after a clean run it resumes from where it left off rather than re-embedding. Returns the
    /// number of events processed.
    pub async fn catch_up(&mut self, store: &dyn Store) -> Result<usize, IndexError> {
        let events = store.read_from(self.vectors.cursor()?.next())?;
        let count = events.len();
        self.index_batch(&events).await?;
        Ok(count)
    }

    /// Drain every event currently available on `subscription`, index it, and advance the cursor.
    /// Returns how many were processed. Non-blocking: it stops when the channel is momentarily empty.
    pub async fn drain(&mut self, subscription: &Subscription) -> Result<usize, IndexError> {
        let events: Vec<Event> = subscription.try_iter().collect();
        let count = events.len();
        self.index_batch(&events).await?;
        Ok(count)
    }

    /// Embed a batch and apply it to the index, advancing the cursor. The convenience path for callers
    /// that hold the embedder and index together (the tests, the rebuild). The live server instead uses
    /// [`embed_batch`] then [`apply_batch`] separately, so the slow embedding holds no index lock.
    pub async fn index_batch(&mut self, events: &[Event]) -> Result<(), IndexError> {
        let batch = embed_batch(self.embedder, events).await?;
        apply_batch(self.vectors, batch)?;
        Ok(())
    }
}

/// A failure indexing the log: embedding the text, or writing the vector index, or reading the log.
#[derive(Debug)]
pub enum IndexError {
    Embed(ModelError),
    Vector(VectorError),
    Store(StoreError),
}

impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IndexError::Embed(error) => write!(f, "index (embed): {error}"),
            IndexError::Vector(error) => write!(f, "index (vector): {error}"),
            IndexError::Store(error) => write!(f, "index (store): {error}"),
        }
    }
}

impl std::error::Error for IndexError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IndexError::Embed(error) => Some(error),
            IndexError::Vector(error) => Some(error),
            IndexError::Store(error) => Some(error),
        }
    }
}

impl From<ModelError> for IndexError {
    fn from(error: ModelError) -> IndexError {
        IndexError::Embed(error)
    }
}

impl From<VectorError> for IndexError {
    fn from(error: VectorError) -> IndexError {
        IndexError::Vector(error)
    }
}

impl From<StoreError> for IndexError {
    fn from(error: StoreError) -> IndexError {
        IndexError::Store(error)
    }
}
