//! The event-log seam: the durable, append-only source of truth.
//!
//! The backend is swappable as long as it preserves a single total order over `Seq` (spec
//! §Storage). The in-memory backend is always available, for tests and the no-I/O build; the
//! SQLite backend ships behind the `sqlite` feature. Faithful replay falls out of this seam: read
//! from `Seq::ZERO` and the events come back in the exact order they were committed.

mod memory;
#[cfg(feature = "sqlite")]
mod sqlite;

pub use memory::MemoryStore;
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteStore;

use std::sync::mpsc::{Receiver, Sender};

use crate::{
    event::{Event, EventPayload},
    ids::Seq,
    time::Timestamp,
};

/// A live feed of events committed after the subscription was taken. The debugger and other
/// read-side clients use this for incremental updates (spec §Observability).
pub type Subscription = Receiver<Event>;

/// The single writer's view of the log. One process holds the writable store; everything else is a
/// reader (spec principle 10, "one writer, many clients").
pub trait Store {
    /// Append a batch atomically, stamping every payload with `recorded_at` and assigning
    /// consecutive sequence numbers. Returns the committed events in order. A batch is the unit of
    /// atomicity — it maps onto a block's buffered effects in the eventual commit path (Stage 4).
    fn append(
        &mut self,
        recorded_at: Timestamp,
        payloads: Vec<EventPayload>,
    ) -> Result<Vec<Event>, StoreError>;

    /// Read every event with `seq >= from`, in `Seq` order. `read_from(Seq::ZERO)` is a full
    /// replay. (Returns a `Vec` for now; this becomes a stream once logs are large enough to care.)
    fn read_from(&self, from: Seq) -> Result<Vec<Event>, StoreError>;

    /// The highest committed sequence number, or `Seq::ZERO` if the log is empty.
    fn head(&self) -> Result<Seq, StoreError>;

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

#[cfg(feature = "sqlite")]
impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        StoreError::Backend(error.to_string())
    }
}

/// Fan committed events out to live subscribers, dropping any whose receiver has hung up. Shared by
/// both backends, since the subscriber set is an in-process concern independent of durability.
fn notify(subscribers: &mut Vec<Sender<Event>>, committed: &[Event]) {
    subscribers.retain(|sender| {
        committed
            .iter()
            .all(|event| sender.send(event.clone()).is_ok())
    });
}

#[cfg(test)]
mod tests {
    //! The seam contract, written once against the `Store` trait and run against every backend, so
    //! the in-memory and SQLite stores are held to the same total-order and faithful-replay contract.
    use super::{MemoryStore, Store};
    use crate::{
        event::EventPayload,
        ids::{MemoryId, MemoryName, Seq},
        time::Timestamp,
        vocabulary::TagName,
    };

    fn sample_payloads() -> Vec<EventPayload> {
        let id = MemoryId::generate();
        vec![
            EventPayload::TagCreated {
                name: TagName::new("hobbies"),
                description: "Recreational activities and interests".to_owned(),
            },
            EventPayload::MemoryCreated {
                id,
                name: MemoryName::new("person/dave"),
            },
            EventPayload::MemoryRenamed {
                id,
                old_name: MemoryName::new("person/dave"),
                new_name: MemoryName::new("person/dave-chen"),
            },
        ]
    }

    /// Appending stamps consecutive sequence numbers, and a full read returns the exact payloads in
    /// the exact order they were committed — faithful replay at the log layer.
    fn append_is_ordered_and_faithful<S: Store>(store: &mut S) {
        assert_eq!(store.head().unwrap(), Seq::ZERO);

        let payloads = sample_payloads();
        let committed = store
            .append(Timestamp::from_millis(1_000), payloads.clone())
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

    /// Two appends continue the sequence, and `read_from` returns only the requested tail.
    fn read_from_returns_tail<S: Store>(store: &mut S) {
        store
            .append(
                Timestamp::from_millis(1),
                vec![EventPayload::MemoryDeleted {
                    id: MemoryId::generate(),
                }],
            )
            .unwrap();
        store
            .append(
                Timestamp::from_millis(2),
                vec![EventPayload::MemoryDeleted {
                    id: MemoryId::generate(),
                }],
            )
            .unwrap();

        let tail = store.read_from(Seq(2)).unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].seq, Seq(2));
    }

    /// A subscriber taken before an append receives the committed events.
    fn subscriber_sees_appends<S: Store>(store: &mut S) {
        let subscription = store.subscribe();
        store
            .append(
                Timestamp::from_millis(5),
                vec![EventPayload::TagCreated {
                    name: TagName::new("colleagues"),
                    description: "People worked with".to_owned(),
                }],
            )
            .unwrap();

        let event = subscription.recv().unwrap();
        assert_eq!(event.seq, Seq(1));
        assert_eq!(event.payload.kind(), "TagCreated");
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

    #[cfg(feature = "sqlite")]
    mod sqlite {
        use super::*;
        use crate::store::SqliteStore;

        #[test]
        fn append_is_ordered_and_faithful() {
            super::append_is_ordered_and_faithful(&mut SqliteStore::open_in_memory().unwrap());
        }

        #[test]
        fn read_from_returns_tail() {
            super::read_from_returns_tail(&mut SqliteStore::open_in_memory().unwrap());
        }

        #[test]
        fn subscriber_sees_appends() {
            super::subscriber_sees_appends(&mut SqliteStore::open_in_memory().unwrap());
        }

        /// The log survives a process boundary: append, drop, reopen, and the events are still there
        /// in order — the property the whole "rebuild from the log" story rests on.
        #[test]
        fn persists_across_reopen() {
            let path = std::env::temp_dir()
                .join(format!("zuihitsu-test-{}.sqlite", MemoryId::generate().0));

            {
                let mut store = SqliteStore::open(&path).unwrap();
                store
                    .append(Timestamp::from_millis(1_000), sample_payloads())
                    .unwrap();
            }
            {
                let store = SqliteStore::open(&path).unwrap();
                assert_eq!(store.head().unwrap(), Seq(3));
                assert_eq!(store.read_from(Seq::ZERO).unwrap().len(), 3);
            }

            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
            let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        }

        /// One log, one writer: a second open of the same file is refused while the first is held,
        /// and succeeds once it is released.
        #[test]
        fn exclusive_lock_blocks_a_second_writer() {
            let path = std::env::temp_dir()
                .join(format!("zuihitsu-lock-{}.sqlite", MemoryId::generate().0));

            let first = SqliteStore::open(&path).unwrap();
            assert!(SqliteStore::open(&path).is_err()); // already open
            drop(first);
            assert!(SqliteStore::open(&path).is_ok()); // lock released

            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
            let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        }
    }
}
