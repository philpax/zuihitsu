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

use std::collections::BTreeMap;

use ulid::Ulid;

use crate::{
    event::{Event, EventPayload},
    ids::{EntryId, MemoryId},
    store::{Store, StoreError, Subscription},
    vector::{VectorError, VectorId, VectorIndex, VectorRecord},
};

use super::{ModelError, embed::Embedder};

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

    /// Index a batch of committed events. Coalesces to one operation per vector (last event wins),
    /// so a description regenerated several times in the batch embeds once; entries are immutable, so
    /// each embeds once anyway.
    pub async fn apply(&mut self, events: &[Event]) -> Result<(), IndexError> {
        let mut ops: BTreeMap<VectorId, Op> = BTreeMap::new();
        for event in events {
            match &event.payload {
                EventPayload::MemoryContentAppended { entry_id, text, .. } => {
                    ops.insert(
                        VectorKey::Entry(*entry_id).to_vector_id(),
                        Op::Embed(text.clone()),
                    );
                }
                EventPayload::MemoryDescriptionRegenerated { id, new_text, .. } => {
                    ops.insert(
                        VectorKey::Description(*id).to_vector_id(),
                        Op::Embed(new_text.clone()),
                    );
                }
                EventPayload::MemoryDeleted { id } => {
                    ops.insert(VectorKey::Description(*id).to_vector_id(), Op::Remove);
                }
                _ => {}
            }
        }

        let to_embed: Vec<(VectorId, String)> = ops
            .iter()
            .filter_map(|(key, op)| match op {
                Op::Embed(text) => Some((key.clone(), text.clone())),
                Op::Remove => None,
            })
            .collect();

        if !to_embed.is_empty() {
            let texts: Vec<String> = to_embed.iter().map(|(_, text)| text.clone()).collect();
            let embeddings = self.embedder.embed(&texts).await?;
            let model_id = self.embedder.model_id();
            for ((id, _), embedding) in to_embed.into_iter().zip(embeddings) {
                self.vectors.upsert(VectorRecord {
                    id,
                    embedding,
                    model_id: model_id.into(),
                })?;
            }
        }

        for (key, op) in &ops {
            if matches!(op, Op::Remove) {
                self.vectors.remove(key)?;
            }
        }
        Ok(())
    }
}

/// The pending change for one vector in a batch.
enum Op {
    /// (Re)embed to this text.
    Embed(String),
    /// Drop the vector.
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

#[cfg(test)]
mod tests {
    //! The reactive projection embeds regenerated descriptions, drops vectors on delete, and can be
    //! driven from a full-log rebuild, a subscription drain, or a raw event batch. Uses the
    //! deterministic fake embedder, so the same text embeds identically and a query of a memory's own
    //! description retrieves it.
    use super::{Indexer, VectorKey};
    use crate::{
        event::{Event, EventPayload},
        ids::{MemoryId, MemoryName},
        model::embed::{Embedder, FakeEmbedder},
        store::{MemoryStore, Store},
        time::Timestamp,
        vector::{InMemoryVectorIndex, VectorIndex},
    };

    const DIMS: usize = 16;

    fn at(ms: i64) -> Timestamp {
        Timestamp::from_millis(ms)
    }

    #[tokio::test]
    async fn catch_up_embeds_each_memorys_description() {
        let mut store = MemoryStore::new();
        let dave = MemoryId::generate();
        store
            .append(
                at(1),
                vec![
                    // The indexer ignores the create and reacts only to the description.
                    EventPayload::MemoryCreated {
                        id: dave,
                        name: MemoryName::new("person/dave"),
                    },
                    EventPayload::MemoryDescriptionRegenerated {
                        id: dave,
                        new_text: "An avid rock climber".to_owned(),
                        produced_by: None,
                    },
                ],
            )
            .unwrap();

        let embedder = FakeEmbedder::new(DIMS);
        let mut vectors = InMemoryVectorIndex::new();
        Indexer::new(&embedder, &mut vectors)
            .catch_up(&store)
            .await
            .unwrap();

        assert_eq!(vectors.len().unwrap(), 1);
        // Querying Dave's own description retrieves his vector, keyed by memory id.
        let query = embedder
            .embed(&["An avid rock climber".to_owned()])
            .await
            .unwrap()
            .remove(0);
        let hits = vectors.search(&query, 1).unwrap();
        assert_eq!(hits[0].id, VectorKey::Description(dave).to_vector_id());
    }

    #[tokio::test]
    async fn catch_up_resumes_from_the_cursor() {
        let mut store = MemoryStore::new();
        let dave = MemoryId::generate();
        store
            .append(
                at(1),
                vec![EventPayload::MemoryDescriptionRegenerated {
                    id: dave,
                    new_text: "An avid rock climber".to_owned(),
                    produced_by: None,
                }],
            )
            .unwrap();

        let embedder = FakeEmbedder::new(DIMS);
        let mut vectors = InMemoryVectorIndex::new();

        // First catch-up processes the one event and advances the cursor to its seq.
        assert_eq!(
            Indexer::new(&embedder, &mut vectors)
                .catch_up(&store)
                .await
                .unwrap(),
            1
        );
        assert_eq!(vectors.cursor().unwrap(), store.head().unwrap());

        // With nothing new in the log, a second catch-up is a no-op (doesn't re-embed).
        assert_eq!(
            Indexer::new(&embedder, &mut vectors)
                .catch_up(&store)
                .await
                .unwrap(),
            0
        );

        // A new event is the only thing the next catch-up processes.
        let erin = MemoryId::generate();
        store
            .append(
                at(2),
                vec![EventPayload::MemoryDescriptionRegenerated {
                    id: erin,
                    new_text: "A tax accountant".to_owned(),
                    produced_by: None,
                }],
            )
            .unwrap();
        assert_eq!(
            Indexer::new(&embedder, &mut vectors)
                .catch_up(&store)
                .await
                .unwrap(),
            1
        );
        assert_eq!(vectors.len().unwrap(), 2);
        assert_eq!(vectors.cursor().unwrap(), store.head().unwrap());
    }

    #[tokio::test]
    async fn drain_indexes_subscribed_events() {
        let mut store = MemoryStore::new();
        let subscription = store.subscribe();
        let dave = MemoryId::generate();
        store
            .append(
                at(1),
                vec![EventPayload::MemoryDescriptionRegenerated {
                    id: dave,
                    new_text: "An avid rock climber".to_owned(),
                    produced_by: None,
                }],
            )
            .unwrap();

        let embedder = FakeEmbedder::new(DIMS);
        let mut vectors = InMemoryVectorIndex::new();
        let processed = Indexer::new(&embedder, &mut vectors)
            .drain(&subscription)
            .await
            .unwrap();

        assert_eq!(processed, 1);
        assert_eq!(vectors.len().unwrap(), 1);
    }

    #[tokio::test]
    async fn a_later_regeneration_replaces_and_a_delete_removes() {
        let dave = MemoryId::generate();
        let embedder = FakeEmbedder::new(DIMS);
        let mut vectors = InMemoryVectorIndex::new();
        let key = VectorKey::Description(dave).to_vector_id();

        {
            let mut indexer = Indexer::new(&embedder, &mut vectors);
            // A re-description replaces in place rather than adding a second vector.
            indexer
                .apply(&events(&mut MemoryStore::new(), dave, "old description"))
                .await
                .unwrap();
            indexer
                .apply(&events(&mut MemoryStore::new(), dave, "new description"))
                .await
                .unwrap();
        }
        assert_eq!(vectors.len().unwrap(), 1);
        let new_query = embedder
            .embed(&["new description".to_owned()])
            .await
            .unwrap()
            .remove(0);
        assert!((vectors.search(&new_query, 1).unwrap()[0].score - 1.0).abs() < 1e-3);

        // A delete drops the vector.
        let mut store = MemoryStore::new();
        let deletion = store
            .append(at(2), vec![EventPayload::MemoryDeleted { id: dave }])
            .unwrap();
        Indexer::new(&embedder, &mut vectors)
            .apply(&deletion)
            .await
            .unwrap();
        assert!(vectors.is_empty().unwrap());
        assert!(
            vectors
                .search(&new_query, 5)
                .unwrap()
                .iter()
                .all(|hit| hit.id != key)
        );
    }

    /// Commit a description regeneration for `id` and return the resulting events to feed the indexer.
    fn events(store: &mut MemoryStore, id: MemoryId, description: &str) -> Vec<Event> {
        store
            .append(
                at(1),
                vec![EventPayload::MemoryDescriptionRegenerated {
                    id,
                    new_text: description.to_owned(),
                    produced_by: None,
                }],
            )
            .unwrap()
    }
}
