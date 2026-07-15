//! The in-memory event store. Always compiled; it is the backend for tests and for the no-I/O
//! build, and it implements exactly the same total-order contract as the SQLite backend.

use std::sync::mpsc::{Sender, channel};

use crate::{
    event::{Event, EventPayload, EventSource},
    ids::Seq,
    time::Timestamp,
};

use super::{Store, StoreError, Subscription, notify};

#[derive(Default)]
pub struct MemoryStore {
    events: Vec<Event>,
    subscribers: Vec<Sender<Event>>,
}

impl MemoryStore {
    pub fn new() -> MemoryStore {
        MemoryStore::default()
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
