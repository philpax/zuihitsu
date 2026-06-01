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

use crate::event::{Event, EventPayload};
use crate::ids::{Seq, Timestamp};

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

/// An event-store failure. Display messages are lowercase fragments suitable for "failed to {…}".
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
            StoreError::Backend(message) => write!(f, "access the event store: {message}"),
            StoreError::Serialize(error) => write!(f, "serialize an event payload: {error}"),
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

/// Fan committed events out to live subscribers, dropping any whose receiver has hung up. Shared by
/// both backends, since the subscriber set is an in-process concern independent of durability.
fn notify(subscribers: &mut Vec<Sender<Event>>, committed: &[Event]) {
    subscribers.retain(|sender| {
        committed
            .iter()
            .all(|event| sender.send(event.clone()).is_ok())
    });
}
