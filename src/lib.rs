//! Zuihitsu — a persistent memory system for a conversational agent.
//!
//! One instance hosts exactly one agent whose entire life is a single event log read from `seq 0`
//! (see `docs/spec.md`). Stage 1 establishes that log — the append-only source of truth — and the
//! abstraction seams (clock, store, and later the model/fetcher/vector seams) that make a complete
//! agent constructible in memory for tests without a database, a network, or a wall clock.

pub mod clock;
pub mod embed;
pub mod event;
pub mod fetch;
pub mod ids;
pub mod model;
pub mod store;
pub mod vector;

pub use clock::{Clock, ManualClock, SystemClock};
pub use embed::{Embedder, Embedding, FakeEmbedder};
pub use event::{Event, EventPayload};
pub use fetch::{CannedFetcher, FetchError, Fetcher};
pub use ids::{EntryId, MemoryId, MemoryName, Seq, TagName, Timestamp};
pub use model::{
    Completion, GenerateRequest, Message, ModelClient, ModelError, Role, ScriptedModel, ToolCall,
    ToolSpec,
};
#[cfg(feature = "sqlite")]
pub use store::SqliteStore;
pub use store::{MemoryStore, Store, StoreError};
pub use vector::{InMemoryVectorIndex, ScoredHit, VectorId, VectorIndex};
