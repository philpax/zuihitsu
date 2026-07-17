//! The in-memory event store. Always compiled; it is the backend for tests and for the no-I/O
//! build, and it implements exactly the same total-order contract as the SQLite backend.

use std::sync::mpsc::{Sender, channel};

use crate::{
    event::{Event, EventPayload, EventSource},
    ids::Seq,
    time::Timestamp,
};

use crate::store::{Store, StoreError, Subscription, notify};

#[derive(Default)]
pub struct MemoryStore {
    events: Vec<Event>,
    subscribers: Vec<Sender<Event>>,
}

impl MemoryStore {
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }

    /// Construct a store already holding `events` verbatim — a persisted log reloaded, the way a disk
    /// backend reopens a file. Each event keeps its recorded seq, timestamp, and source, so the total
    /// order is preserved exactly (unlike re-appending, which would re-stamp and re-seq). `events` must
    /// be the whole log in seq order. Lets a test carry the in-memory log across a simulated restart —
    /// a fresh instance over the same log, its runtime state reset — without a temp file.
    pub fn from_events(events: Vec<Event>) -> MemoryStore {
        MemoryStore {
            events,
            subscribers: Vec::new(),
        }
    }
}

impl Store for MemoryStore {
    fn append(
        &mut self,
        recorded_at: Timestamp,
        source: EventSource,
        payloads: Vec<EventPayload>,
    ) -> Result<Vec<Event>, StoreError> {
        let mut seq = self.head()?;
        let committed: Vec<Event> = payloads
            .into_iter()
            .map(|payload| {
                seq = seq.next();
                Event {
                    seq,
                    recorded_at,
                    source: source.clone(),
                    payload,
                }
            })
            .collect();

        self.events.extend(committed.iter().cloned());
        notify(&mut self.subscribers, &committed);
        Ok(committed)
    }

    fn read_from(&self, from: Seq) -> Result<Vec<Event>, StoreError> {
        Ok(self
            .events
            .iter()
            .filter(|event| event.seq >= from)
            .cloned()
            .collect())
    }

    fn head(&self) -> Result<Seq, StoreError> {
        Ok(self
            .events
            .last()
            .map(|event| event.seq)
            .unwrap_or(Seq::ZERO))
    }

    fn truncate_to(&mut self, to: Seq) -> Result<u64, StoreError> {
        let before = self.events.len();
        self.events.retain(|event| event.seq <= to);
        Ok((before - self.events.len()) as u64)
    }

    fn subscribe(&mut self) -> Subscription {
        let (sender, receiver) = channel();
        self.subscribers.push(sender);
        receiver
    }
}
