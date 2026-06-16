//! The long-running HTTP server — `zuihitsu` with no subcommand boots this (spec §Clients and the
//! server boundary). It opens the instance the config selects (acquiring the single-writer log lock),
//! reconciles the graph, connects any configured MCP servers, runs the background scheduler driver,
//! and serves the API the CLI and a future web console drive. The HTTP layer lives here, in the
//! binary, so the library stays transport-agnostic; `/` is reserved for the web console, the
//! operator surface lives under `/control`, and the participant surface under `/platform`.
//!
//! The module is split by surface: [`control`] holds the operator handlers, [`platform`] the
//! participant handlers, [`auth`] the per-surface bearer-key middleware, and [`error`] the request
//! error rendered as an HTTP response. This file owns the boot sequence ([`run_blocking`], [`serve`]),
//! the [`router`] wiring those surfaces together, the shared [`AppState`], and the startup
//! [`ServeError`].

mod auth;
mod control;
mod error;
mod platform;

use std::{
    io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use axum::{
    Router,
    response::IntoResponse,
    routing::{get, post},
};
use tokio::net::TcpListener;
use zuihitsu::{
    ConfigError, EnvConfig, Graph, GraphError, ModelClient, OpenAiClient, OpenAiEmbedder, Server,
    ServerError, SnapshotSchedule, SqliteStore, SqliteVectorIndex, StdioHost, StoreError,
    SystemClock, VectorError, VectorIndex, model::embed::Embedder, snapshot,
    snapshot::SnapshotError,
};

use auth::{require_control_key, require_platform_key};
use control::{
    arbitrations, create_agent, entries, env_config, events, genesis, health, imprint,
    interactions, lua_api, memories, memory, recurring, register_prompt, run_lua, sessions,
    set_settings, settings, snapshot as snapshot_handler,
};
use platform::{join, message};

/// Shared HTTP handler state: the agent server behind the `Arc` its facets are designed to share (so
/// each handler grabs a fresh `control()`/`platform()` per request), and the model client the
/// conversing endpoints (`imprint`, `route_message`) drive — `None` when no model endpoint is
/// configured, in which case those endpoints return `503`.
#[derive(Clone)]
struct AppState {
    server: Arc<Server>,
    model: Option<Arc<dyn ModelClient>>,
    /// Where an on-demand snapshot is written — `Some` when snapshotting is enabled, `None`
    /// otherwise (the snapshot endpoint then answers `409`).
    snapshot_dir: Option<PathBuf>,
    /// Valid API keys for the operator surface (`/control/*`); a remote peer must present one, a
    /// loopback peer is trusted without one. `Arc<[String]>` so the per-request state clone is a
    /// refcount bump, not a deep copy.
    control_keys: Arc<[String]>,
    /// Valid API keys for the participant surface (`/platform/*`); the same rule.
    platform_keys: Arc<[String]>,
    /// The environmental config this instance booted from, for the read-only config view. Serializing
    /// it redacts the secrets (the API keys serialize as counts, the MCP env as its variable names),
    /// so the view never exposes a key.
    config: Arc<EnvConfig>,
}

/// How often the background indexer catches the vector index up to the log. Indexing is off the hot
/// path (spec §Storage → vector store), so a short poll keeps search fresh without a per-commit cost;
/// an idle tick is cheap (it reads the log from the index's cursor and finds nothing).
const INDEX_TICK_SECONDS: u64 = 5;

/// How often the background describer catches memory descriptions up to the log. Like indexing, it runs
/// off the hot path (spec §Write path → regenerate off the hot path), so a short poll keeps descriptions
/// fresh without a per-turn cost; an idle tick is cheap.
const DESCRIBE_TICK_SECONDS: u64 = 5;

/// How often the background adjudicator weighs proposed merges (spec §Cross-platform identity →
/// adjudicated merge). Proposals are rare, so this polls less eagerly than the describer; an idle tick
/// is cheap (a head check against the cursor).
const ADJUDICATE_TICK_SECONDS: u64 = 7;

/// Build the multi-thread tokio runtime and run the server to completion — the synchronous entry the
/// CLI calls when invoked with no subcommand.
pub fn run_blocking(config_path: &Path) -> Result<(), ServeError> {
    let config = EnvConfig::load(config_path).map_err(ServeError::Config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(ServeError::Runtime)?;
    runtime.block_on(serve(config))
}

/// Open the instance, start the scheduler driver, and serve the HTTP API until interrupted, then shut
/// down gracefully (stop accepting, stop the driver, tear down live sessions).
async fn serve(config: EnvConfig) -> Result<(), ServeError> {
    let bind = config.serving.bind;
    // Capture the booted config for the read-only config view before its parts are moved out below
    // (the MCP table into `connect_mcp`, the API keys into the auth layers).
    let env_config = Arc::new(config.clone());
    ensure_parent_dir(&config.storage.event_log)?;
    ensure_parent_dir(&config.storage.graph)?;
    let store = SqliteStore::open(&config.storage.event_log).map_err(|source| {
        ServeError::OpenEventLog {
            path: config.storage.event_log.clone(),
            source,
        }
    })?;
    // Restore the graph from the latest snapshot when it leads the on-disk graph (a fresh, deleted, or
    // corrupt graph), before the file is opened — so boot replays only the tail (spec §Snapshots).
    let snapshot_dir = config.snapshots.effective_dir(&config.storage.graph);
    if config.snapshots.enabled
        && let Some(head) = snapshot::restore_if_stale(&config.storage.graph, &snapshot_dir)
            .map_err(ServeError::Snapshot)?
    {
        tracing::info!(head = head.0, "restored the graph from a snapshot");
    }
    let graph = Graph::open(&config.storage.graph).map_err(|source| ServeError::OpenGraph {
        path: config.storage.graph.clone(),
        source,
    })?;
    // Semantic retrieval is enabled when an embedding endpoint is configured: build the embedder and
    // open the sqlite-vec index sized to the embedding dimensionality, so `memory.search` and the
    // background indexer have an embedder and an index to work over (spec §Storage → vector store).
    let retrieval = if config.embedding.endpoint.is_empty() {
        tracing::warn!("no embedding endpoint configured; semantic search is unavailable");
        None
    } else {
        let embedder: Arc<dyn Embedder> = Arc::new(OpenAiEmbedder::new(&config.embedding));
        let vectors = SqliteVectorIndex::open(&config.storage.vectors, config.embedding.dimensions)
            .map_err(|source| ServeError::OpenVectors {
                path: config.storage.vectors.clone(),
                source,
            })?;
        Some((embedder, Box::new(vectors) as Box<dyn VectorIndex>))
    };
    let mut server = match retrieval {
        Some((embedder, vectors)) => Server::with_retrieval(
            Box::new(store),
            graph,
            Box::new(SystemClock),
            embedder,
            vectors,
        ),
        None => Server::new(Box::new(store), graph, Box::new(SystemClock)),
    };
    let status = server.boot()?;

    // Connect the configured MCP servers once, before the server is shared (`connect_mcp` is `&mut`).
    if !config.mcp.is_empty() {
        server.connect_mcp(Arc::new(StdioHost), config.mcp).await?;
    }

    let tick =
        Duration::from_secs(server.control().settings()?.scheduler.tick_seconds.max(1) as u64);

    // The model client the conversing endpoints use, built from config. Absent endpoint → no model →
    // those endpoints answer 503 rather than failing at call time.
    let model: Option<Arc<dyn ModelClient>> = if config.model.endpoint.is_empty() {
        tracing::warn!("no model endpoint configured; conversing endpoints will return 503");
        None
    } else {
        Some(Arc::new(OpenAiClient::new(&config.model)))
    };
    let server = Arc::new(server);

    // The background scheduler driver fires due wake-ups on its own timer (spec §Scheduled work),
    // stopping on the same Ctrl-C that ends the HTTP server.
    let driver = tokio::spawn({
        let server = server.clone();
        async move { server.run_scheduler(tick, shutdown_signal()).await }
    });

    // The background indexer catches the vector index up to the log off the hot path (spec §Storage →
    // vector store). A no-op (returns immediately) on a graph-only instance.
    let indexer = tokio::spawn({
        let server = server.clone();
        async move {
            server
                .run_indexer(Duration::from_secs(INDEX_TICK_SECONDS), shutdown_signal())
                .await
        }
    });

    // The background describer regenerates memory descriptions (and arbitration and temporal
    // extraction) off the hot path (spec §Write path → regenerate off the hot path). Spawned only when
    // a model is configured; without one there is nothing to run the synthesis call.
    let describer = model.as_ref().map(|model| {
        let server = server.clone();
        let model = model.clone();
        tokio::spawn(async move {
            server
                .run_describer(
                    model,
                    Duration::from_secs(DESCRIBE_TICK_SECONDS),
                    shutdown_signal(),
                )
                .await
        })
    });

    // The background adjudicator weighs proposed cross-platform merges off the hot path (spec
    // §Cross-platform identity → adjudicated merge). Spawned only when a model is configured; without
    // one there is no judge to run.
    let adjudicator = model.as_ref().map(|model| {
        let server = server.clone();
        let model = model.clone();
        tokio::spawn(async move {
            server
                .run_adjudicator(
                    model,
                    Duration::from_secs(ADJUDICATE_TICK_SECONDS),
                    shutdown_signal(),
                )
                .await
        })
    });

    // The background snapshotter checkpoints the graph on its own activity-gated cadence (spec
    // §Snapshots), when enabled. Stops on the same shutdown signal.
    let snapshotter = config.snapshots.enabled.then(|| {
        let server = server.clone();
        let schedule = SnapshotSchedule {
            dir: snapshot_dir.clone(),
            check_interval: Duration::from_secs(config.snapshots.check_interval_seconds.max(1)),
            min_new_events: config.snapshots.min_new_events,
            keep: config.snapshots.keep,
        };
        tokio::spawn(async move { server.run_snapshotter(schedule, shutdown_signal()).await })
    });

    let app = router(AppState {
        server: server.clone(),
        model,
        snapshot_dir: config.snapshots.enabled.then_some(snapshot_dir),
        control_keys: config.serving.control_keys.into(),
        platform_keys: config.serving.platform_keys.into(),
        config: env_config,
    });
    let listener = TcpListener::bind(bind).await.map_err(ServeError::Bind)?;
    tracing::info!(?status, %bind, "zuihitsu serving");
    // `into_make_service_with_connect_info` surfaces each connection's peer address to the auth
    // middleware (a bare `axum::serve(listener, app)` would not), so a loopback peer can be trusted.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .map_err(ServeError::Serve)?;

    tracing::info!("shutdown signal received; stopping");
    let _ = driver.await;
    let _ = indexer.await;
    if let Some(describer) = describer {
        let _ = describer.await;
    }
    if let Some(adjudicator) = adjudicator {
        let _ = adjudicator.await;
    }
    if let Some(snapshotter) = snapshotter {
        let _ = snapshotter.await;
    }
    server.shutdown().await;
    Ok(())
}

/// The API router. `/` is the reserved web-console root; the operator surface lives under `/control`
/// and the participant surface under `/platform`. Each surface is its own sub-router carrying its own
/// auth layer ([`require_control_key`] / [`require_platform_key`]), so a control key never authorizes
/// `/platform` and vice versa. The layer is applied per sub-router rather than once at the top, because
/// a top-level layer would also wrap the nested platform routes and force platform clients to present a
/// control key. Under `.nest`, the sub-router routes are spelled relative (`/agent`, not
/// `/control/agent`).
fn router(state: AppState) -> Router {
    // The operator surface: agent creation and read-only inspection (spec §Clients → control clients).
    // The CLI and the future web console drive these.
    let control = Router::new()
        .route("/health", get(health))
        .route("/agent", post(create_agent))
        .route("/genesis", get(genesis))
        .route("/memory", get(memory))
        .route("/memories", get(memories))
        .route("/entries", get(entries))
        .route("/sessions", get(sessions))
        .route("/recurring", get(recurring))
        .route("/arbitrations", get(arbitrations))
        .route("/interactions", get(interactions))
        .route("/events", get(events))
        .route("/snapshot", post(snapshot_handler))
        .route("/settings", get(settings).put(set_settings))
        .route("/config", get(env_config))
        .route("/imprint", post(imprint))
        .route("/lua", post(run_lua))
        .route("/lua-api", get(lua_api))
        .route("/prompt", post(register_prompt))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_control_key,
        ));
    // The participant surface: delivering turns and mid-session joins (spec §Clients → platform
    // clients). It carries platform identity in the payload, never operator authority.
    let platform = Router::new()
        .route("/message", post(message))
        .route("/join", post(join))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_platform_key,
        ));
    Router::new()
        // The reserved web-console root stays ungated for now (a static placeholder); the real UI
        // will move under the control surface when it lands.
        .route("/", get(root))
        .nest("/control", control)
        .nest("/platform", platform)
        .with_state(state)
}

/// The reserved web-console root — a placeholder until the frontend lands.
async fn root() -> impl IntoResponse {
    (
        axum::http::StatusCode::OK,
        "zuihitsu is serving. The web console will live here; the API is under /control and \
         /platform.\n",
    )
}

/// Resolve on the next Ctrl-C. Driving both the HTTP server and the scheduler driver off independent
/// instances of this means a single interrupt stops both.
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

fn ensure_parent_dir(path: &Path) -> Result<(), ServeError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|source| ServeError::CreateDir {
            path: parent.to_owned(),
            source,
        })?;
    }
    Ok(())
}

/// A failure starting or running the server.
#[derive(Debug)]
pub enum ServeError {
    Config(ConfigError),
    Runtime(io::Error),
    CreateDir {
        path: PathBuf,
        source: io::Error,
    },
    OpenEventLog {
        path: PathBuf,
        source: StoreError,
    },
    OpenGraph {
        path: PathBuf,
        source: GraphError,
    },
    OpenVectors {
        path: PathBuf,
        source: VectorError,
    },
    /// Restoring the graph from a snapshot at boot failed (spec §Snapshots).
    Snapshot(SnapshotError),
    /// A server operation (boot, reading settings, connecting MCP) failed at startup.
    Server(ServerError),
    Bind(io::Error),
    Serve(io::Error),
}

impl From<ServerError> for ServeError {
    fn from(error: ServerError) -> Self {
        ServeError::Server(error)
    }
}

impl std::fmt::Display for ServeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServeError::Config(source) => write!(f, "serve: could not load config: {source}"),
            ServeError::Runtime(source) => {
                write!(f, "serve: could not start the runtime: {source}")
            }
            ServeError::CreateDir { path, source } => {
                write!(f, "serve: could not create {}: {source}", path.display())
            }
            ServeError::OpenEventLog { path, source } => {
                write!(
                    f,
                    "serve: could not open the event log at {}: {source}",
                    path.display()
                )
            }
            ServeError::OpenGraph { path, source } => {
                write!(
                    f,
                    "serve: could not open the graph at {}: {source}",
                    path.display()
                )
            }
            ServeError::OpenVectors { path, source } => {
                write!(
                    f,
                    "serve: could not open the vector index at {}: {source}",
                    path.display()
                )
            }
            ServeError::Snapshot(source) => {
                write!(
                    f,
                    "serve: could not restore the graph from a snapshot: {source}"
                )
            }
            ServeError::Server(source) => write!(f, "serve: {source}"),
            ServeError::Bind(source) => write!(f, "serve: could not bind the listener: {source}"),
            ServeError::Serve(source) => write!(f, "serve: the HTTP server failed: {source}"),
        }
    }
}

impl std::error::Error for ServeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ServeError::Config(source) => Some(source),
            ServeError::Runtime(source) => Some(source),
            ServeError::CreateDir { source, .. } => Some(source),
            ServeError::OpenEventLog { source, .. } => Some(source),
            ServeError::OpenGraph { source, .. } => Some(source),
            ServeError::OpenVectors { source, .. } => Some(source),
            ServeError::Snapshot(source) => Some(source),
            ServeError::Server(source) => Some(source),
            ServeError::Bind(source) | ServeError::Serve(source) => Some(source),
        }
    }
}

#[cfg(test)]
mod tests;
