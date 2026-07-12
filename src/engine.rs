//! The shared backends a turn threads as a unit: the append-only event log (`store`), the graph
//! projection it feeds (`graph`), and the clock that stamps writes (`clock`).
//!
//! They always travel together, so they ride as one value rather than three parallel arguments — the
//! shared shape behind [`crate::agent::Turn`], the pre-compaction flush, the step loop, and
//! [`crate::agent::lua::Session::execute`]. The whole bundle lives behind a single [`Arc`], so a turn
//! shares it with one pointer bump and the Lua block API can hold a `'static` handle to it across the
//! script's `eval_async` (the block API moved off mlua's borrowing `scope` to make that possible).
//!
//! Each backend is locked transiently for a single read or write; **nothing holds a guard across an
//! `.await`**. When two are needed at once — only `materialize_from`, which reads the store while
//! writing the graph, and the scheduler's `fire_due` — the **graph is locked before the store**, the
//! one ordering rule that keeps the (non-reentrant) locks deadlock-free once sessions run concurrently.

use std::{collections::HashMap, sync::Arc};

use parking_lot::Mutex;

use zuihitsu_core::progress::TurnProgress;

use crate::{
    clock::Clock, graph::Graph, ids::MemoryId, model::embed::Embedder, store::Store,
    vector::VectorIndex,
};

/// The store, graph, and clock a turn operates over, bundled behind one [`Arc`] (see the module docs
/// for the locking discipline). Built once per agent and cloned cheaply for each turn.
pub struct Engine {
    pub store: Mutex<Box<dyn Store>>,
    pub graph: Mutex<Graph>,
    pub clock: Box<dyn Clock>,
    /// The per-memory lock registry the Lua block API acquires from: a block holds the lock on each
    /// memory it touches until block end, so a concurrent block in another conversation serializes on
    /// the same memory (spec §Concurrency). Shared by every session through the one `Arc<Engine>`.
    pub memory_locks: Arc<MemoryLocks>,
    /// The semantic-retrieval backends, present when an embedding endpoint is configured. `None` on a
    /// graph-only instance (no embedding endpoint, and most tests), where `memory.search` reports
    /// itself unavailable rather than failing obscurely.
    pub retrieval: Option<Retrieval>,
    /// The live turn-progress feed the console's stream endpoint subscribes to (spec
    /// §Observability). Ephemeral by design: frames never touch the store, publishing to no
    /// subscriber is a no-op, and the recorded events are identical whether or not anyone watched.
    pub progress: ProgressFeed,
}

/// A lossy broadcast of [`TurnProgress`] frames. Publishing to no subscribers is free and dropped
/// frames cost only smoothness, so the turn loop publishes unconditionally.
pub struct ProgressFeed {
    sender: tokio::sync::broadcast::Sender<TurnProgress>,
}

impl ProgressFeed {
    fn new() -> ProgressFeed {
        // Deep enough that a briefly stalled SSE writer does not lag out mid-reply at token rate;
        // a receiver that still falls behind reconnects and simply misses cosmetic frames.
        let (sender, _) = tokio::sync::broadcast::channel(1024);
        ProgressFeed { sender }
    }

    /// Publish one frame; with no subscriber this is a no-op.
    pub fn publish(&self, frame: TurnProgress) {
        let _ = self.sender.send(frame);
    }

    /// A new subscription over frames published from now on.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<TurnProgress> {
        self.sender.subscribe()
    }
}

/// The embedder and vector index that back semantic search (spec §Storage → vector store). The vector
/// index is behind a `parking_lot` mutex like the store and graph, held only across the brief sync
/// index write or read — **never** across the slow `embedder.embed().await`. Both the background
/// indexer and `memory.search` embed *before* taking this lock (see
/// [`crate::model::index::embed_batch`] / [`crate::model::index::apply_batch`]), so the embedding never
/// blocks a concurrent search and no guard ever crosses a suspension point. The
/// embedder is immutable, shared as an `Arc`.
pub struct Retrieval {
    pub embedder: Arc<dyn Embedder>,
    pub vectors: Mutex<Box<dyn VectorIndex>>,
}

impl Engine {
    /// Bundle the three backends behind a shared [`Arc`], graph-only (no semantic retrieval).
    pub fn new(store: Box<dyn Store>, graph: Graph, clock: Box<dyn Clock>) -> Arc<Engine> {
        Arc::new(Engine {
            store: Mutex::new(store),
            graph: Mutex::new(graph),
            clock,
            memory_locks: Arc::new(MemoryLocks::new()),
            retrieval: None,
            progress: ProgressFeed::new(),
        })
    }

    /// As [`Engine::new`], with the semantic-retrieval backends attached — the configuration the live
    /// server uses when an embedding endpoint is set.
    pub fn with_retrieval(
        store: Box<dyn Store>,
        graph: Graph,
        clock: Box<dyn Clock>,
        embedder: Arc<dyn Embedder>,
        vectors: Box<dyn VectorIndex>,
    ) -> Arc<Engine> {
        Arc::new(Engine {
            store: Mutex::new(store),
            graph: Mutex::new(graph),
            clock,
            memory_locks: Arc::new(MemoryLocks::new()),
            progress: ProgressFeed::new(),
            retrieval: Some(Retrieval {
                embedder,
                vectors: Mutex::new(vectors),
            }),
        })
    }
}

/// The per-memory mutual-exclusion registry (spec §Concurrency → per-memory mutual exclusion): one
/// async mutex per [`MemoryId`], minted on first contention. A block acquires the lock for each memory
/// it touches and holds the owned guard until block end, so concurrent access to the same memory from
/// another conversation blocks until the holding block finishes.
///
/// Entries persist for the registry's lifetime — one small `Arc<Mutex<()>>` per memory ever touched,
/// which is negligible at this deployment's scale. This is deliberate, not a leak: a periodic sweep of
/// uncontended entries is the standard fix if it ever matters, and is deferred.
pub struct MemoryLocks {
    map: Mutex<HashMap<MemoryId, Arc<tokio::sync::Mutex<()>>>>,
}

impl MemoryLocks {
    fn new() -> MemoryLocks {
        MemoryLocks {
            map: Mutex::new(HashMap::new()),
        }
    }

    /// Acquire the lock for `id`, awaiting any current holder, and return the owned guard (released
    /// when dropped). The registry map is locked only to fetch-or-mint the per-memory mutex and is
    /// released before the `.await`, so no `parking_lot` guard ever crosses the suspension point.
    pub async fn acquire(&self, id: MemoryId) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = self.map.lock().entry(id).or_default().clone();
        lock.lock_owned().await
    }
}

#[cfg(test)]
mod tests {
    use super::{MemoryId, MemoryLocks};
    use std::time::Duration;

    #[tokio::test]
    async fn a_second_acquire_of_the_same_memory_waits_for_the_first() {
        let locks = MemoryLocks::new();
        let id = MemoryId::generate();
        let held = locks.acquire(id).await;

        // A second acquire of the same memory cannot complete while the first guard is held.
        let blocked = tokio::time::timeout(Duration::from_millis(50), locks.acquire(id)).await;
        assert!(blocked.is_err(), "the second acquire should wait");

        // Once the first guard drops, the second acquire proceeds.
        drop(held);
        let _second = tokio::time::timeout(Duration::from_millis(50), locks.acquire(id))
            .await
            .expect("the second acquire proceeds once the lock frees");
    }

    #[tokio::test]
    async fn distinct_memories_do_not_contend() {
        let locks = MemoryLocks::new();
        let (a, b) = (MemoryId::generate(), MemoryId::generate());
        let _held = locks.acquire(a).await;
        // Holding `a` does not block acquiring a different memory `b`.
        let _other = tokio::time::timeout(Duration::from_millis(50), locks.acquire(b))
            .await
            .expect("a distinct memory's lock is free");
    }
}
