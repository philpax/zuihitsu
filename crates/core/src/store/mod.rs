//! The event-log seam: the durable, append-only source of truth.
//!
//! The backend is swappable as long as it preserves a single total order over `Seq` (spec
//! §Storage). The in-memory backend (here) serves tests, the no-I/O path, and the wasm replica; the
//! durable file-backed SQLite backend lives in the main crate, since it needs the host filesystem.
//! Faithful replay falls out of this seam: read from `Seq::ZERO` and the events come back in the
//! exact order they were committed. The system's behavior is therefore a pure function of the event
//! log modulo declared nondeterminism (ULID minting, wall-clock stamps), and the in-memory backends
//! (`MemoryStore` here, `Graph::open_in_memory`, `SqliteVectorIndex::open_in_memory`) exist so tests
//! exercise exactly that function without touching the host filesystem; the file-backed backends stay
//! for production and for the handful of tests that assert the persistence path itself.

mod memory;

pub use memory::MemoryStore;

use std::sync::mpsc::{Receiver, Sender};

use crate::{
    event::{Event, EventPayload, EventSource},
    ids::Seq,
    time::Timestamp,
};

/// A live feed of events committed after the subscription was taken. The console and other
/// read-side clients use this for incremental updates (spec §Observability).
pub type Subscription = Receiver<Event>;

/// The single writer's view of the log. One process holds the writable store; everything else is a
/// reader (spec principle 10, "one writer, many clients").
///
/// `Send` so the store can ride behind the shared `Arc<Mutex<Box<dyn Store>>>` the turn engine
/// threads; both backends (`MemoryStore`, `SqliteStore`) are `Send`.
pub trait Store: Send {
    /// Append a batch atomically, stamping every payload with `recorded_at` and `source` and
    /// assigning consecutive sequence numbers. Returns the committed events in order. A batch is the
    /// unit of atomicity — it maps onto a block's buffered effects in the eventual commit path (Stage
    /// 4) — and shares one authority: `source` is required so a writer must name the authority it
    /// commits under rather than defaulting one silently (spec §Trust model).
    fn append(
        &mut self,
        recorded_at: Timestamp,
        source: EventSource,
        payloads: Vec<EventPayload>,
    ) -> Result<Vec<Event>, StoreError>;

    /// Read every event with `seq >= from`, in `Seq` order. `read_from(Seq::ZERO)` is a full
    /// replay. (Returns a `Vec` for now; this becomes a stream once logs are large enough to care.)
    fn read_from(&self, from: Seq) -> Result<Vec<Event>, StoreError>;

    /// The highest committed sequence number, or `Seq::ZERO` if the log is empty.
    fn head(&self) -> Result<Seq, StoreError>;

    /// The wall-clock stamp of the event at `seq`, or `None` if no event holds it. Dates a single
    /// committed event without replaying the tail — used to age a watermark (the describer's oldest
    /// pending content change) against the clock. The default scans from `seq`; a backend that can
    /// index a single row overrides it (the file-backed store does).
    fn recorded_at(&self, seq: Seq) -> Result<Option<Timestamp>, StoreError> {
        Ok(self
            .read_from(seq)?
            .first()
            .filter(|event| event.seq == seq)
            .map(|event| event.recorded_at))
    }

    /// Remove every event with `seq > to`, leaving `to` as the new head; returns the number removed.
    /// The inverse of `append`, and the sole exception to the append-only log: it exists only for the
    /// operator's revert path. The derived stores (the materialized graph, the vector index, and any
    /// snapshots) sit ahead of the truncated log afterward, and the caller is responsible for resetting
    /// them so they rebuild from the shortened log on the next boot.
    fn truncate_to(&mut self, to: Seq) -> Result<u64, StoreError>;

    /// Subscribe to events committed from now on. Multiple subscribers are independent.
    fn subscribe(&mut self) -> Subscription;
}

/// An event-store failure.
#[derive(Debug)]
pub enum StoreError {
    /// The underlying backend (e.g. SQLite) reported an error.
    Backend(String),
    /// An event payload could not be (de)serialized.
    Serialize(serde_json::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Backend(message) => write!(f, "event store: {message}"),
            StoreError::Serialize(error) => {
                write!(f, "event store: could not serialize a payload: {error}")
            }
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Serialize(error) => Some(error),
            StoreError::Backend(_) => None,
        }
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(error: serde_json::Error) -> Self {
        StoreError::Serialize(error)
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        StoreError::Backend(error.to_string())
    }
}

/// Fan committed events out to live subscribers, dropping any whose receiver has hung up. Shared by
/// both backends (the in-memory one here and the file-backed one in the main crate), since the
/// subscriber set is an in-process concern independent of durability.
pub fn notify(subscribers: &mut Vec<Sender<Event>>, committed: &[Event]) {
    subscribers.retain(|sender| {
        committed
            .iter()
            .all(|event| sender.send(event.clone()).is_ok())
    });
}

/// The `Store` seam contract, written once against the trait so every backend is held to the same
/// total-order and faithful-replay guarantees. Exposed under the `test-support` feature so the main
/// crate's `SqliteStore` tests run the exact same checks the in-memory backend does here, rather
/// than re-deriving them (spec §Testability; CONTRIBUTING → reuse testing facilities).
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::{Seq, Store, Timestamp};
    use crate::{
        event::{EventPayload, EventSource},
        ids::{MemoryId, Namespace},
        vocabulary::TagName,
    };

    /// A small, representative batch: a tag, a memory, and a rename of it.
    pub fn sample_payloads() -> Vec<EventPayload> {
        let id = MemoryId::generate();
        vec![
            EventPayload::tag_created(
                TagName::new("hobbies"),
                "Recreational activities and interests".to_owned(),
            ),
            EventPayload::memory_created(id, Namespace::Person.with_name("dave")),
            EventPayload::memory_renamed(
                id,
                Namespace::Person.with_name("dave"),
                Namespace::Person.with_name("dave-chen"),
            ),
        ]
    }

    /// Appending stamps consecutive sequence numbers, and a full read returns the exact payloads in
    /// the exact order they were committed — faithful replay at the log layer.
    pub fn append_is_ordered_and_faithful<S: Store>(store: &mut S) {
        assert_eq!(store.head().unwrap(), Seq::ZERO);

        let payloads = sample_payloads();
        let committed = store
            .append(
                Timestamp::from_millis(1_000),
                EventSource::Agent,
                payloads.clone(),
            )
            .unwrap();

        assert_eq!(committed.len(), 3);
        assert_eq!(committed[0].seq, Seq(1));
        assert_eq!(committed[2].seq, Seq(3));
        assert_eq!(store.head().unwrap(), Seq(3));

        let replayed = store.read_from(Seq::ZERO).unwrap();
        let replayed_payloads: Vec<EventPayload> =
            replayed.iter().map(|e| e.payload.clone()).collect();
        assert_eq!(replayed_payloads, payloads);
        assert!(replayed.windows(2).all(|w| w[0].seq < w[1].seq));
        assert!(
            replayed
                .iter()
                .all(|e| e.recorded_at == Timestamp::from_millis(1_000))
        );
    }

    /// Every event in a batch is stamped with the batch's `source`, both on the committed events the
    /// append returns and on a later replay — the envelope's author axis survives the round trip
    /// through the backend.
    pub fn append_stamps_the_source<S: Store>(store: &mut S) {
        let sources = [
            EventSource::Bootstrap,
            EventSource::Agent,
            EventSource::Operator,
            EventSource::Orchestration,
        ];
        for (index, source) in sources.into_iter().enumerate() {
            let committed = store
                .append(
                    Timestamp::from_millis(1_000 + index as i64),
                    source,
                    vec![EventPayload::memory_deleted(MemoryId::generate())],
                )
                .unwrap();
            assert_eq!(committed[0].source, source);
        }

        let replayed = store.read_from(Seq::ZERO).unwrap();
        let replayed_sources: Vec<EventSource> = replayed.iter().map(|e| e.source).collect();
        assert_eq!(replayed_sources, sources);
    }

    /// Two appends continue the sequence, and `read_from` returns only the requested tail.
    pub fn read_from_returns_tail<S: Store>(store: &mut S) {
        store
            .append(
                Timestamp::from_millis(1),
                EventSource::Agent,
                vec![EventPayload::memory_deleted(MemoryId::generate())],
            )
            .unwrap();
        store
            .append(
                Timestamp::from_millis(2),
                EventSource::Agent,
                vec![EventPayload::memory_deleted(MemoryId::generate())],
            )
            .unwrap();

        let tail = store.read_from(Seq(2)).unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].seq, Seq(2));
    }

    /// Truncating to a seq drops every later event, leaves the head at that seq, reports how many it
    /// removed, and is a no-op once the log is already at or below the target.
    pub fn truncate_removes_the_tail<S: Store>(store: &mut S) {
        store
            .append(
                Timestamp::from_millis(1),
                EventSource::Agent,
                vec![
                    EventPayload::memory_deleted(MemoryId::generate()),
                    EventPayload::memory_deleted(MemoryId::generate()),
                    EventPayload::memory_deleted(MemoryId::generate()),
                ],
            )
            .unwrap();
        assert_eq!(store.head().unwrap(), Seq(3));

        let removed = store.truncate_to(Seq(1)).unwrap();
        assert_eq!(removed, 2);
        assert_eq!(store.head().unwrap(), Seq(1));
        assert_eq!(store.read_from(Seq::ZERO).unwrap().len(), 1);

        // Truncating at or past the head removes nothing.
        assert_eq!(store.truncate_to(Seq(1)).unwrap(), 0);
        assert_eq!(store.truncate_to(Seq(5)).unwrap(), 0);
        assert_eq!(store.head().unwrap(), Seq(1));
    }

    /// A subscriber taken before an append receives the committed events.
    pub fn subscriber_sees_appends<S: Store>(store: &mut S) {
        let subscription = store.subscribe();
        store
            .append(
                Timestamp::from_millis(5),
                EventSource::Agent,
                vec![EventPayload::tag_created(
                    TagName::new("colleagues"),
                    "People worked with".to_owned(),
                )],
            )
            .unwrap();

        let event = subscription.recv().unwrap();
        assert_eq!(event.seq, Seq(1));
        assert_eq!(event.payload.kind(), "TagCreated");
    }
}

#[cfg(test)]
mod tests {
    //! The seam contract run against the in-memory backend; the SQLite backend runs the same
    //! `test_support` helpers from the main crate.
    use super::{
        MemoryStore,
        test_support::{
            append_is_ordered_and_faithful, append_stamps_the_source, read_from_returns_tail,
            subscriber_sees_appends, truncate_removes_the_tail,
        },
    };

    #[test]
    fn memory_truncate_removes_the_tail() {
        truncate_removes_the_tail(&mut MemoryStore::new());
    }

    #[test]
    fn memory_append_stamps_the_source() {
        append_stamps_the_source(&mut MemoryStore::new());
    }

    #[test]
    fn memory_append_is_ordered_and_faithful() {
        append_is_ordered_and_faithful(&mut MemoryStore::new());
    }

    #[test]
    fn memory_read_from_returns_tail() {
        read_from_returns_tail(&mut MemoryStore::new());
    }

    #[test]
    fn memory_subscriber_sees_appends() {
        subscriber_sees_appends(&mut MemoryStore::new());
    }
}
