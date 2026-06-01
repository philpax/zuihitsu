//! Zuihitsu — a persistent memory system for a conversational agent.
//!
//! One instance hosts exactly one agent whose entire life is a single event log read from `seq 0`
//! (see `docs/spec.md`). Stage 1 establishes that log — the append-only source of truth — and the
//! abstraction seams (clock, store, and later the model/fetcher/vector seams) that make a complete
//! agent constructible in memory for tests without a database, a network, or a wall clock.

pub mod clock;
pub mod event;
pub mod ids;
pub mod store;

pub use clock::{Clock, ManualClock, SystemClock};
pub use event::{Event, EventPayload};
pub use ids::{EntryId, MemoryId, MemoryName, Seq, TagName, Timestamp};
#[cfg(feature = "sqlite")]
pub use store::SqliteStore;
pub use store::{MemoryStore, Store, StoreError};
