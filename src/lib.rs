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
pub mod index;
#[cfg(feature = "lua")]
pub mod lua;
pub mod model;
#[cfg(feature = "openai")]
pub mod openai;
#[cfg(feature = "sqlite")]
pub mod search;
#[cfg(feature = "sqlite")]
pub mod server;
pub mod settings;
pub mod store;
pub mod templates;
pub mod vector;

pub use clock::{Clock, ManualClock, SystemClock};
pub use config::{ConfigError, EmbeddingConfig, EnvConfig, ModelConfig};
pub use embed::{Embedder, Embedding, FakeEmbedder};
pub use event::{
    Cardinality, Event, EventPayload, EventSource, Initiation, LinkSource, PromptTemplateName,
    TerminalCause, TurnRole, Volatility,
};
pub use fetch::{CannedFetcher, FetchError, Fetcher};
pub use genesis::{GenesisStatus, Rollout, SeedSelf};
pub use ids::{
    ConversationId, EntryId, MemoryId, MemoryName, RelationName, Seq, TagName, Timestamp, TurnId,
};
pub use index::{IndexError, Indexer};
pub use model::{
    Completion, GenerateRequest, Message, ModelClient, ModelError, Role, ScriptedModel, ToolCall,
    ToolSpec,
};
pub use settings::{
    BriefSettings, CompactionSettings, RecencySettings, SearchSettings, Settings, TauDays,
    TurnSettings,
};
pub use store::{MemoryStore, Store, StoreError};
pub use templates::{PromptTemplate, latest_template};
pub use vector::{
    InMemoryVectorIndex, ScoredHit, VectorError, VectorId, VectorIndex, VectorRecord,
};

// The feature-gated re-exports are grouped per feature so the `#[cfg]` lives in one place rather
// than on every line; each private module is glob-re-exported into the crate root.
#[cfg(feature = "lua")]
mod __lua {
    pub use crate::{
        agent::{TurnError, TurnOutcome, run_turn},
        lua::{BlockOutcome, LuaError, Session},
    };
}
#[cfg(feature = "lua")]
pub use __lua::*;

#[cfg(feature = "sqlite")]
mod __sqlite {
    pub use crate::{
        graph::{EntryView, Graph, GraphError, LinkView, MemoryView, RelationView},
        search::{SearchError, SearchHit, SearchQuery, search},
        server::{Control, Server, ServerError},
        store::SqliteStore,
        vector::SqliteVectorIndex,
    };
}
#[cfg(feature = "sqlite")]
pub use __sqlite::*;

#[cfg(feature = "openai")]
mod __openai {
    pub use crate::openai::{OpenAiClient, OpenAiEmbedder};
}
#[cfg(feature = "openai")]
pub use __openai::*;
