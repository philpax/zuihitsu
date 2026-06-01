//! The vector indexer: a reactive projection of the log into the vector index.
//!
//! Like the materialized graph, the vector index is a rebuildable projection of the event log — but
//! maintaining it costs a model call, so it runs off the turn's hot path as background work (spec
//! §Storage → vector store, §Concurrency). The indexer consumes committed events and embeds the
//! content they record: a `MemoryDescriptionRegenerated` (re)embeds the description, a
//! `MemoryDeleted` drops its vectors. Each vector is stamped with the embedder's `model_id` at
//! creation so a mixed-embedding-space state stays detectable.
//!
//! This cut indexes the per-memory description vector; the per-entry vectors follow.

use std::collections::BTreeMap;

use crate::{
    embed::Embedder,
    event::{Event, EventPayload},
    ids::MemoryId,
    model::ModelError,
    store::{Store, StoreError, Subscription},
    vector::{VectorError, VectorId, VectorIndex, VectorRecord},
};

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
        self.apply_and_advance(&events).await?;
        Ok(count)
    }

    /// Drain every event currently available on `subscription`, index it, and advance the cursor.
    /// Returns how many were processed. Non-blocking: it stops when the channel is momentarily empty.
    pub async fn drain(&mut self, subscription: &Subscription) -> Result<usize, IndexError> {
        let events: Vec<Event> = subscription.try_iter().collect();
        let count = events.len();
        self.apply_and_advance(&events).await?;
        Ok(count)
    }

    /// Apply a batch, then advance the cursor to the batch's last `Seq` — done after the vectors are
    /// written so a crash re-processes rather than skips.
    async fn apply_and_advance(&mut self, events: &[Event]) -> Result<(), IndexError> {
        self.apply(events).await?;
        if let Some(last) = events.last() {
            self.vectors.set_cursor(last.seq)?;
        }
        Ok(())
    }

    /// Index a batch of committed events. Coalesces to one action per memory (last event wins), so a
    /// description regenerated several times in the batch embeds once.
    pub async fn apply(&mut self, events: &[Event]) -> Result<(), IndexError> {
        let mut actions: BTreeMap<MemoryId, Action> = BTreeMap::new();
        for event in events {
            match &event.payload {
                EventPayload::MemoryDescriptionRegenerated { id, new_text } => {
                    actions.insert(*id, Action::Embed(new_text.clone()));
                }
                EventPayload::MemoryDeleted { id } => {
                    actions.insert(*id, Action::Remove);
                }
                _ => {}
            }
        }

        let to_embed: Vec<(MemoryId, String)> = actions
            .iter()
            .filter_map(|(id, action)| match action {
                Action::Embed(text) => Some((*id, text.clone())),
                Action::Remove => None,
            })
            .collect();

        if !to_embed.is_empty() {
            let texts: Vec<String> = to_embed.iter().map(|(_, text)| text.clone()).collect();
            let embeddings = self.embedder.embed(&texts).await?;
            let model_id = self.embedder.model_id();
            for ((id, _), embedding) in to_embed.into_iter().zip(embeddings) {
                self.vectors.upsert(VectorRecord {
                    id: VectorId::new(id.0.to_string()),
                    embedding,
                    model_id: model_id.into(),
                })?;
            }
        }

        for (id, action) in &actions {
            if matches!(action, Action::Remove) {
                self.vectors.remove(&VectorId::new(id.0.to_string()))?;
            }
        }
        Ok(())
    }
}

/// The pending index change for one memory in a batch.
enum Action {
    /// (Re)embed the description to this text.
    Embed(String),
    /// Drop the memory's vectors.
    Remove,
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
