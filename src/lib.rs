//! Zuihitsu — a persistent memory system for a conversational agent.
//!
//! One instance hosts exactly one agent whose entire life is a single event log read from `seq 0`
//! (see `docs/spec.md`). Stage 1 establishes that log — the append-only source of truth — and the
//! abstraction seams (clock, store, and later the model/embedder/vector seams) that make a complete
//! agent constructible in memory for tests without a database, a network, or a wall clock.

#[cfg(feature = "lua")]
pub mod agent;
pub mod api_doc;
#[cfg(feature = "sqlite")]
pub mod brief;
pub mod clock;
pub mod config;
pub mod embed;
pub mod event;
pub mod genesis;
#[cfg(feature = "sqlite")]
pub mod graph;
#[cfg(feature = "sqlite")]
pub mod identity;
pub mod ids;
pub mod index;
#[cfg(feature = "lua")]
pub mod lua;
#[cfg(feature = "sqlite")]
pub mod memory_block;
pub mod model;
#[cfg(feature = "openai")]
pub mod openai;
#[cfg(feature = "sqlite")]
pub mod search;
#[cfg(feature = "sqlite")]
pub mod server;
pub mod settings;
pub mod store;
#[cfg(feature = "sqlite")]
pub mod system_prompt;
pub mod templates;
pub mod time;
pub mod vector;
#[cfg(feature = "sqlite")]
pub mod visibility;

pub use api_doc::{ApiEntry, ApiParam, ApiType, ObjectBuilder, enum_of, object};
pub use clock::{Clock, ManualClock, SystemClock};
pub use config::{ConfigError, EmbeddingConfig, EnvConfig, ModelConfig};
pub use embed::{Embedder, Embedding, FakeEmbedder};
pub use event::{
    Cardinality, Event, EventPayload, EventSource, Initiation, LinkSource, ProducedBy,
    PromptTemplateName, Teller, TerminalCause, TurnRole, Visibility, Volatility,
};
pub use genesis::{GenesisStatus, Rollout, SeedSelf};
pub use ids::{
    ConversationId, ConversationLocator, EntryId, MemoryId, MemoryName, RelationName, Seq,
    SessionId, TagName, Timestamp, TurnId,
};
pub use index::{IndexError, Indexer};
pub use model::{
    Completion, GenerateRequest, GenerateResponse, Message, ModelClient, ModelError, Role,
    ScriptedModel, ToolCall, ToolChoice, ToolSpec, Usage,
};
pub use settings::{
    BriefSettings, CompactionSettings, RecencySettings, SearchSettings, Settings, TauDays,
    TurnSettings,
};
pub use store::{MemoryStore, Store, StoreError};
pub use templates::{PromptTemplate, latest_template};
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
    pub use crate::{
        agent::{
            BlockContext, Engine, Turn, TurnError, TurnOutcome, TurnReport, TurnView, buffer_turns,
            run_turn, session_touched,
        },
        lua::{BlockOutcome, LuaError, Session, api_reference, render_api_reference},
    };
}
#[cfg(feature = "lua")]
pub use __lua::*;

#[cfg(feature = "sqlite")]
mod __sqlite {
    pub use crate::{
        brief::{BriefError, BriefRequest, compose, compose_participant},
        graph::{EntryView, Graph, GraphError, LinkView, MemoryView, RelationView, SessionView},
        identity::{IdentityError, resolve_or_mint_conversation, resolve_or_mint_participant},
        memory_block::{
            AppendOptions, Authority, BlockEffects, MemoryBlock, MemoryError, VisibilityChoice,
        },
        search::{SearchError, SearchHit, SearchQuery, search},
        server::{Control, Server, ServerError},
        store::SqliteStore,
        vector::SqliteVectorIndex,
        visibility::{
            MarkerRoom, default_visibility, default_visibility_named, room_display,
            teller_private_marker, visible,
        },
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
