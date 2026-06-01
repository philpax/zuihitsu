//! Zuihitsu — a persistent memory system for a conversational agent.
//!
//! One instance hosts exactly one agent whose entire life is a single event log read from `seq 0`
//! (see `docs/spec.md`). Stage 1 establishes that log — the append-only source of truth — and the
//! abstraction seams (clock, store, and later the model/fetcher/vector seams) that make a complete
//! agent constructible in memory for tests without a database, a network, or a wall clock.

#[cfg(feature = "lua")]
pub mod agent;
pub mod clock;
pub mod config;
pub mod embed;
pub mod event;
pub mod fetch;
pub mod genesis;
#[cfg(feature = "sqlite")]
pub mod graph;
pub mod ids;
#[cfg(feature = "lua")]
pub mod lua;
pub mod model;
#[cfg(feature = "openai")]
pub mod openai;
#[cfg(feature = "sqlite")]
pub mod server;
pub mod store;
pub mod vector;

#[cfg(feature = "lua")]
pub use agent::{TurnError, TurnOutcome, run_turn};
pub use clock::{Clock, ManualClock, SystemClock};
pub use config::{ConfigError, EmbeddingConfig, EnvConfig, ModelConfig};
pub use embed::{Embedder, Embedding, FakeEmbedder};
pub use event::{
    Cardinality, ConfigValue, Event, EventPayload, EventSource, Initiation, LinkSource,
    TerminalCause, TurnRole, Volatility,
};
pub use fetch::{CannedFetcher, FetchError, Fetcher};
pub use genesis::{GenesisStatus, Rollout, SeedSelf};
#[cfg(feature = "sqlite")]
pub use graph::{EntryView, Graph, GraphError, LinkView, MemoryView, RelationView};
pub use ids::{
    ConversationId, EntryId, MemoryId, MemoryName, RelationName, Seq, TagName, Timestamp, TurnId,
};
#[cfg(feature = "lua")]
pub use lua::{BlockOutcome, LuaError, Session};
pub use model::{
    Completion, GenerateRequest, Message, ModelClient, ModelError, Role, ScriptedModel, ToolCall,
    ToolSpec,
};
#[cfg(feature = "openai")]
pub use openai::OpenAiEmbedder;
#[cfg(feature = "sqlite")]
pub use server::{Control, Server, ServerError};
#[cfg(feature = "sqlite")]
pub use store::SqliteStore;
pub use store::{MemoryStore, Store, StoreError};
pub use vector::{InMemoryVectorIndex, ScoredHit, VectorId, VectorIndex};
