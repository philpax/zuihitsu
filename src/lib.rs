//! Zuihitsu — a persistent memory system for a conversational agent.
//!
//! One instance hosts exactly one agent whose entire life is a single event log read from `seq 0`
//! (see `docs/spec.md`). Stage 1 establishes that log — the append-only source of truth — and the
//! abstraction seams (clock, store, and later the model/embedder/vector seams) that make a complete
//! agent constructible in memory for tests without a database, a network, or a wall clock.

pub mod agent;
pub mod clock;
pub mod config;
pub mod engine;
pub mod mcp;
pub mod memory;
pub mod model;
pub mod server;
pub mod snapshot;
pub mod store;
pub mod vector;

// The wasm-compatible core lives in `zuihitsu-core` and is re-exported here, so the rest of the
// codebase reaches these modules at their existing `crate::*` paths. `db` (rusqlite query helpers)
// is re-exported privately, since it is internal infrastructure rather than public API; `visibility`
// is re-exported under `memory::visibility`, its historical home (see `memory`).
use zuihitsu_core::db;
pub use zuihitsu_core::{event, graph, ids, settings, time, vocabulary};

// The agent-creation entry point, re-exported at the crate root so the operator CLI drives genesis
// as `zuihitsu::genesis::{rollout, status}` without reaching through the `agent` subsystem.
pub use agent::genesis;

pub use agent::{
    api_doc::{ApiEntry, ApiParam, ApiType, ObjectBuilder, enum_of, object},
    genesis::{GenesisStatus, Rollout, SeedSelf},
    templates::{PromptTemplate, latest_template},
};
pub use clock::{Clock, ManualClock, SystemClock};
pub use config::{ConfigError, EmbeddingConfig, EnvConfig, ModelConfig, SnapshotConfig};
pub use event::{
    Cardinality, Event, EventPayload, EventSource, Initiation, LinkSource, ModelPhase, ProducedBy,
    PromptTemplateName, RequestRecord, Teller, TerminalCause, TurnRole, Visibility, Volatility,
};
pub use ids::{
    ConversationId, ConversationLocator, EntryId, MemoryId, MemoryName, Seq, SessionId, TurnId,
};
pub use model::{
    Completion, GenerateRequest, GenerateResponse, Message, ModelClient, ModelError,
    ResponseSchema, Role, ScriptedModel, ToolCall, ToolChoice, ToolSpec, Usage,
    embed::{Embedder, Embedding, FakeEmbedder},
    extract_json_object,
    index::{IndexError, Indexer},
    parse_structured, schema_of,
};
pub use settings::{
    BriefSettings, CaptureLevel, CompactionSettings, ConcurrencySettings, ObservabilitySettings,
    RecencySettings, SchedulerSettings, SearchSettings, Settings, TauDays, TurnSettings,
};
pub use store::{MemoryStore, Store, StoreError};
pub use time::{
    BEFORE_AFTER_EPSILON_MILLIS, CivilDate, Direction, OccurrenceBounds, Rrule, TemporalRef,
    Timestamp,
};
pub use vector::{
    InMemoryVectorIndex, ScoredHit, VectorError, VectorId, VectorIndex, VectorRecord,
};
pub use vocabulary::{RelationName, TagName};

pub use agent::{
    BlockContext, McpCatalogue, Turn, TurnError, TurnOutcome, TurnReport, TurnView, buffer_turns,
    lua::{BlockOutcome, LuaError, Session, api_reference, render_api_reference},
    run_adjudicate_catch_up, run_describe_catch_up, run_turn, session_touched,
};
pub use engine::{Engine, Retrieval};
pub use graph::{EntryView, Graph, GraphError, LinkView, MemoryView, RelationView, SessionView};
pub use mcp::{
    ContentBlock, FakeMcpHost, FakeServer, McpError, McpHost, McpInstance, McpOutput,
    McpServerConfig, McpTool, StdioHost,
};
pub use memory::{
    brief::{BriefError, BriefRequest, compose, compose_participant},
    identity::{IdentityError, resolve_or_mint_conversation, resolve_or_mint_participant},
    memory_block::{
        AppendOptions, Authority, BlockEffects, EntryRef, MemoryBlock, MemoryError,
        VisibilityChoice,
    },
    search::{SearchError, SearchHit, SearchQuery, search},
    visibility::{
        MarkerRoom, default_visibility, default_visibility_named, room_display,
        teller_private_marker, visible,
    },
};
pub use model::openai::{OpenAiClient, OpenAiEmbedder};
pub use server::{
    Arbitration, Control, LuaConsoleOutcome, ModelCall, Server, ServerError, SnapshotSchedule,
};
pub use store::SqliteStore;
pub use vector::SqliteVectorIndex;
