//! Zuihitsu — a persistent memory system for a conversational agent.
//!
//! One instance hosts exactly one agent whose entire life is a single event log read from `seq 0`
//! (see `docs/spec.md`). Stage 1 establishes that log — the append-only source of truth — and the
//! abstraction seams (clock, store, and later the model/embedder/vector seams) that make a complete
//! agent constructible in memory for tests without a database, a network, or a wall clock.

pub mod agent;
pub mod clock;
pub mod config;
#[cfg(feature = "sqlite")]
mod db;
pub mod event;
#[cfg(feature = "sqlite")]
pub mod graph;
pub mod ids;
#[cfg(feature = "sqlite")]
pub mod memory;
pub mod model;
#[cfg(feature = "sqlite")]
pub mod server;
pub mod settings;
pub mod store;
pub mod time;
pub mod vector;

// Transitional module re-exports: the integration tests reference these by module *path* (e.g.
// `zuihitsu::genesis::…`). Keep the old paths alive until the test-colocation commit moves those
// tests in-module and these can be dropped.
pub use agent::genesis;
#[cfg(feature = "sqlite")]
pub use agent::system_prompt;
#[cfg(feature = "sqlite")]
pub use memory::{brief, search};
pub use model::index;

pub use agent::{
    api_doc::{ApiEntry, ApiParam, ApiType, ObjectBuilder, enum_of, object},
    genesis::{GenesisStatus, Rollout, SeedSelf},
    templates::{PromptTemplate, latest_template},
};
pub use clock::{Clock, ManualClock, SystemClock};
pub use config::{ConfigError, EmbeddingConfig, EnvConfig, ModelConfig};
pub use event::{
    Cardinality, Event, EventPayload, EventSource, Initiation, LinkSource, ProducedBy,
    PromptTemplateName, Teller, TerminalCause, TurnRole, Visibility, Volatility,
};
pub use ids::{
    ConversationId, ConversationLocator, EntryId, MemoryId, MemoryName, RelationName, Seq,
    SessionId, TagName, Timestamp, TurnId,
};
pub use model::{
    Completion, GenerateRequest, GenerateResponse, Message, ModelClient, ModelError, Role,
    ScriptedModel, ToolCall, ToolChoice, ToolSpec, Usage,
    embed::{Embedder, Embedding, FakeEmbedder},
    index::{IndexError, Indexer},
};
pub use settings::{
    BriefSettings, CompactionSettings, RecencySettings, SearchSettings, Settings, TauDays,
    TurnSettings,
};
pub use store::{MemoryStore, Store, StoreError};
pub use time::{
    BEFORE_AFTER_EPSILON_MILLIS, CivilDate, Direction, OccurrenceBounds, Rrule, TemporalRef,
};
pub use vector::{
    InMemoryVectorIndex, ScoredHit, VectorError, VectorId, VectorIndex, VectorRecord,
};

// The feature-gated re-exports are grouped per feature so the `#[cfg]` lives in one place rather
// than on every line; each private module is glob-re-exported into the crate root.
#[cfg(feature = "lua")]
mod __lua {
    pub use crate::agent::{
        BlockContext, Engine, Turn, TurnError, TurnOutcome, TurnReport, TurnView, buffer_turns,
        lua::{BlockOutcome, LuaError, Session, api_reference, render_api_reference},
        run_turn, session_touched,
    };
}
#[cfg(feature = "lua")]
pub use __lua::*;

#[cfg(feature = "sqlite")]
mod __sqlite {
    pub use crate::{
        graph::{EntryView, Graph, GraphError, LinkView, MemoryView, RelationView, SessionView},
        memory::{
            brief::{BriefError, BriefRequest, compose, compose_participant},
            identity::{IdentityError, resolve_or_mint_conversation, resolve_or_mint_participant},
            memory_block::{
                AppendOptions, Authority, BlockEffects, MemoryBlock, MemoryError, VisibilityChoice,
            },
            search::{SearchError, SearchHit, SearchQuery, search},
            visibility::{
                MarkerRoom, default_visibility, default_visibility_named, room_display,
                teller_private_marker, visible,
            },
        },
        server::{Control, Server, ServerError},
        store::SqliteStore,
        vector::SqliteVectorIndex,
    };
}
#[cfg(feature = "sqlite")]
pub use __sqlite::*;

#[cfg(feature = "openai")]
mod __openai {
    pub use crate::model::openai::{OpenAiClient, OpenAiEmbedder};
}
#[cfg(feature = "openai")]
pub use __openai::*;
