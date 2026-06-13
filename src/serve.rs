//! The long-running HTTP server — `zuihitsu` with no subcommand boots this (spec §Clients and the
//! server boundary). It opens the instance the config selects (acquiring the single-writer log lock),
//! reconciles the graph, connects any configured MCP servers, runs the background scheduler driver,
//! and serves the API the CLI and a future web console drive. The HTTP layer lives here, in the
//! binary, so the library stays transport-agnostic; `/` is reserved for the web console, the
//! operator surface lives under `/control`, and the participant surface under `/platform`.

use std::{
    io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{ConnectInfo, Query, Request, State},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use zuihitsu::{
    Arbitration, ConfigError, ConversationLocator, EntryView, EnvConfig, Graph, GraphError,
    MemoryView, ModelCall, ModelClient, OpenAiClient, OpenAiEmbedder, Rollout, SeedSelf, Server,
    ServerError, SessionView, Settings, SnapshotSchedule, SqliteStore, SqliteVectorIndex,
    StdioHost, StoreError, SystemClock, TurnOutcome, VectorError, VectorIndex,
    genesis::GenesisStatus, model::embed::Embedder, snapshot, snapshot::SnapshotError,
};

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
}

/// How often the background indexer catches the vector index up to the log. Indexing is off the hot
/// path (spec §Storage → vector store), so a short poll keeps search fresh without a per-commit cost;
/// an idle tick is cheap (it reads the log from the index's cursor and finds nothing).
const INDEX_TICK_SECONDS: u64 = 5;

/// How often the background describer catches memory descriptions up to the log. Like indexing, it runs
/// off the hot path (spec §Write path → regenerate off the hot path), so a short poll keeps descriptions
/// fresh without a per-turn cost; an idle tick is cheap.
const DESCRIBE_TICK_SECONDS: u64 = 5;

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
        .route("/snapshot", post(snapshot))
        .route("/settings", get(settings).put(set_settings))
        .route("/imprint", post(imprint))
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

/// Operator-surface auth: a loopback peer passes without a key; a remote peer must present a valid
/// control key (spec §Trust model).
async fn require_control_key(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    authorize(&state.control_keys, peer, request, next).await
}

/// Participant-surface auth: the same rule against the platform key list.
async fn require_platform_key(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    authorize(&state.platform_keys, peer, request, next).await
}

/// Trust a loopback peer; require a valid bearer key from every remote peer. Fail-closed — an empty
/// key list rejects every remote peer, so a routable bind with no keys is a silent lockout rather than
/// a silent exposure (spec §Trust model). A reverse proxy would make every peer appear loopback, so
/// this must not be fronted by one without re-checking auth.
async fn authorize(keys: &[String], peer: SocketAddr, request: Request, next: Next) -> Response {
    if peer.ip().is_loopback() {
        return next.run(request).await;
    }
    let presented = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        // The scheme match is deliberately case-sensitive: the clients are ours, not arbitrary agents.
        .and_then(|value| value.strip_prefix("Bearer "));
    match presented {
        Some(key) if key_is_valid(key, keys) => next.run(request).await,
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

/// Whether `presented` matches any configured key. Compares fixed-width SHA-256 digests rather than the
/// raw strings, so the comparison time does not depend on the key's length or a shared prefix (a plain
/// `==` on the strings would leak both through early exit); the whole list is scanned unconditionally
/// (`|=`, not an early `return`), so the number of configured keys and the matching position do not
/// leak through timing either.
fn key_is_valid(presented: &str, keys: &[String]) -> bool {
    let presented = Sha256::digest(presented.as_bytes());
    let mut matched = false;
    for key in keys {
        matched |= presented == Sha256::digest(key.as_bytes());
    }
    matched
}

/// The reserved web-console root — a placeholder until the frontend lands.
async fn root() -> impl IntoResponse {
    (
        StatusCode::OK,
        "zuihitsu is serving. The web console will live here; the API is under /control and \
         /platform.\n",
    )
}

/// The serving health/status: whether an agent exists yet.
#[derive(Serialize)]
struct Health {
    genesis: GenesisStatus,
}

async fn health(State(state): State<AppState>) -> Result<Json<Health>, ApiError> {
    let genesis = state.server.control().genesis_status()?;
    Ok(Json(Health { genesis }))
}

/// `POST /control/agent` — create the agent (or resume an interrupted genesis); idempotent.
async fn create_agent(
    State(state): State<AppState>,
    Json(seed): Json<SeedSelf>,
) -> Result<Json<Rollout>, ApiError> {
    Ok(Json(state.server.control().create_agent(&seed)?))
}

/// `GET /control/genesis` — whether an agent exists and is ready.
async fn genesis(State(state): State<AppState>) -> Result<Json<GenesisStatus>, ApiError> {
    Ok(Json(state.server.control().genesis_status()?))
}

/// A `?name=` query — a memory or entry name (which may contain `/` and `@`, so it rides as a query
/// parameter rather than a path segment).
#[derive(Deserialize)]
struct NameQuery {
    name: String,
}

/// `GET /control/memory?name=` — inspect a memory by name; `404` if it does not exist.
async fn memory(
    State(state): State<AppState>,
    Query(query): Query<NameQuery>,
) -> Result<Json<MemoryView>, ApiError> {
    match state.server.control().memory(&query.name)? {
        Some(view) => Ok(Json(view)),
        None => Err(ApiError::NotFound(format!(
            "no memory named {:?}",
            query.name
        ))),
    }
}

/// A `?prefix=` query — a namespace prefix (e.g. `person/`).
#[derive(Deserialize)]
struct PrefixQuery {
    prefix: String,
}

/// `GET /control/memories?prefix=` — the live memories in a namespace, ordered by name.
async fn memories(
    State(state): State<AppState>,
    Query(query): Query<PrefixQuery>,
) -> Result<Json<Vec<MemoryView>>, ApiError> {
    Ok(Json(state.server.control().memories(&query.prefix)?))
}

/// `GET /control/entries?name=` — a memory's local content entries (empty if the memory is unknown).
async fn entries(
    State(state): State<AppState>,
    Query(query): Query<NameQuery>,
) -> Result<Json<Vec<EntryView>>, ApiError> {
    Ok(Json(state.server.control().entries(&query.name)?))
}

/// A `?platform=&scope=` query addressing a conversation by its locator.
#[derive(Deserialize)]
struct LocatorQuery {
    platform: String,
    scope: String,
}

/// `GET /control/sessions?platform=&scope=` — the sessions of a conversation, oldest first.
async fn sessions(
    State(state): State<AppState>,
    Query(query): Query<LocatorQuery>,
) -> Result<Json<Vec<SessionView>>, ApiError> {
    let locator = ConversationLocator::new(query.platform, query.scope);
    Ok(Json(state.server.control().sessions(&locator)?))
}

/// `GET /control/recurring` — the memories carrying a recurring occurrence.
async fn recurring(State(state): State<AppState>) -> Result<Json<Vec<MemoryView>>, ApiError> {
    Ok(Json(state.server.control().recurring()?))
}

/// `GET /control/arbitrations` — the recorded belief arbitrations, oldest first.
async fn arbitrations(State(state): State<AppState>) -> Result<Json<Vec<Arbitration>>, ApiError> {
    Ok(Json(state.server.control().arbitrations()?))
}

/// `GET /control/interactions` — the recorded model interactions, oldest first (the deliberation
/// surface: per-call request, reasoning, token usage, and latency).
async fn interactions(State(state): State<AppState>) -> Result<Json<Vec<ModelCall>>, ApiError> {
    Ok(Json(state.server.control().model_calls()?))
}

/// `POST /control/snapshot` — write a graph snapshot now (the operator's take-one-before-an-experiment
/// trigger). `409` when snapshotting is disabled. The response names the file written, or reports that
/// the graph was already snapshotted at its current head.
async fn snapshot(State(state): State<AppState>) -> Result<Json<SnapshotResponse>, ApiError> {
    let dir = state
        .snapshot_dir
        .as_ref()
        .ok_or(ApiError::SnapshotsDisabled)?;
    let written = state.server.snapshot(dir)?;
    Ok(Json(SnapshotResponse {
        snapshot: written.map(|path| path.to_string_lossy().into_owned()),
    }))
}

/// The snapshot a `POST /control/snapshot` wrote, or `null` when the graph was already checkpointed at
/// its current head (no events since the last snapshot).
#[derive(Serialize)]
struct SnapshotResponse {
    snapshot: Option<String>,
}

/// `GET /control/settings` — the agent's current behavioral settings.
async fn settings(State(state): State<AppState>) -> Result<Json<Settings>, ApiError> {
    Ok(Json(state.server.control().settings()?))
}

/// `PUT /control/settings` — replace the behavioral settings (logged as an operator `ConfigSet`).
async fn set_settings(
    State(state): State<AppState>,
    Json(settings): Json<Settings>,
) -> Result<StatusCode, ApiError> {
    state.server.control().set_settings(settings)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /control/imprint` — one operator message of the imprint interview. Operator authority (the
/// only path that may write `self`); needs the model, so `503` if none is configured.
#[derive(Deserialize)]
struct ImprintRequest {
    text: String,
}

async fn imprint(
    State(state): State<AppState>,
    Json(request): Json<ImprintRequest>,
) -> Result<Json<TurnOutcome>, ApiError> {
    let model = state.model.as_ref().ok_or(ApiError::NoModel)?;
    let outcome = state
        .server
        .control()
        .imprint(model.as_ref(), &request.text)
        .await?;
    Ok(Json(outcome))
}

/// `POST /platform/message` — deliver a participant turn and run the agent's response cycle. Carries
/// the platform identity in the payload (the locator's platform, the sender, the present set); needs
/// the model, so `503` if none is configured.
#[derive(Deserialize)]
struct MessageRequest {
    locator: ConversationLocator,
    sender: String,
    text: String,
    present: Vec<String>,
}

async fn message(
    State(state): State<AppState>,
    Json(request): Json<MessageRequest>,
) -> Result<Json<TurnOutcome>, ApiError> {
    let model = state.model.as_ref().ok_or(ApiError::NoModel)?;
    let present: Vec<&str> = request.present.iter().map(String::as_str).collect();
    let outcome = state
        .server
        .platform()
        .route_message(
            model.as_ref(),
            &request.locator,
            &request.sender,
            &request.text,
            &present,
        )
        .await?;
    Ok(Json(outcome))
}

/// `POST /platform/join` — note a participant arriving mid-session (no model needed).
#[derive(Deserialize)]
struct JoinRequest {
    locator: ConversationLocator,
    participant: String,
}

async fn join(
    State(state): State<AppState>,
    Json(request): Json<JoinRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .server
        .platform()
        .note_join(&request.locator, &request.participant)?;
    Ok(StatusCode::NO_CONTENT)
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

/// An error rendered as an HTTP response. A [`ServerError`] is an infrastructure/processing failure →
/// `500`; a `NotFound` is a named resource that does not exist → `404`. Malformed request bodies are
/// rejected at the axum extractor (`400`) before a handler runs, so that case never reaches here.
enum ApiError {
    Server(ServerError),
    NotFound(String),
    /// A conversing endpoint was called but no model is configured.
    NoModel,
    /// The snapshot endpoint was called but snapshotting is disabled (`[snapshots] enabled = false`).
    SnapshotsDisabled,
}

impl From<ServerError> for ApiError {
    fn from(error: ServerError) -> Self {
        ApiError::Server(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            ApiError::Server(error) => {
                tracing::error!(%error, "request failed");
                (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
            }
            ApiError::NotFound(message) => (StatusCode::NOT_FOUND, message),
            ApiError::NoModel => (
                StatusCode::SERVICE_UNAVAILABLE,
                "no model endpoint is configured".to_owned(),
            ),
            ApiError::SnapshotsDisabled => (
                StatusCode::CONFLICT,
                "snapshots are disabled ([snapshots] enabled = false)".to_owned(),
            ),
        };
        (status, Json(ErrorBody { error: message })).into_response()
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
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
mod tests {
    use super::{AppState, router};
    use axum::{
        body::Body,
        extract::ConnectInfo,
        http::{Request, StatusCode},
    };
    use std::{net::SocketAddr, sync::Arc};
    use tower::ServiceExt;
    use zuihitsu::{Completion, ManualClock, ModelCall, ScriptedModel, Server, time::Timestamp};

    /// No configured keys — the existing tests run loopback, where keys are not consulted.
    fn no_keys() -> Arc<[String]> {
        Vec::new().into()
    }

    /// A loopback peer extension to inject into a `oneshot` request (real `axum::serve` sets this from
    /// the socket; `Request::builder()` does not). The auth middleware trusts a loopback peer, so the
    /// existing assertions are unaffected.
    fn loopback() -> ConnectInfo<SocketAddr> {
        ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 0)))
    }

    /// A non-loopback peer extension, for the auth tests — a remote peer must present a valid key.
    fn remote() -> ConnectInfo<SocketAddr> {
        ConnectInfo(SocketAddr::from(([203, 0, 113, 1], 1234)))
    }

    /// The router serves `/control/health` over an in-memory server, with no real socket — `oneshot`
    /// drives one request through the tower service.
    #[tokio::test]
    async fn health_reports_genesis_status() {
        let server = Arc::new(
            Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap(),
        );
        let app = router(AppState {
            server,
            model: None,
            snapshot_dir: None,
            control_keys: no_keys(),
            platform_keys: no_keys(),
        });
        let response = app
            .oneshot(
                Request::builder()
                    .extension(loopback())
                    .uri("/control/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        // No agent created, so genesis is Empty.
        assert_eq!(&bytes[..], br#"{"genesis":"Empty"}"#);
    }

    #[tokio::test]
    async fn create_then_inspect_over_the_control_api() {
        let server = Arc::new(
            Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap(),
        );
        let app = router(AppState {
            server,
            model: None,
            snapshot_dir: None,
            control_keys: no_keys(),
            platform_keys: no_keys(),
        });

        // Create the agent through the API.
        let seed = serde_json::json!({
            "agent_name": "Kestrel",
            "persona": "An assistant.",
            "seed_entries": [],
        });
        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .extension(loopback())
                    .method("POST")
                    .uri("/control/agent")
                    .header("content-type", "application/json")
                    .body(Body::from(seed.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::OK);

        // Genesis now reports Complete.
        let genesis = app
            .clone()
            .oneshot(
                Request::builder()
                    .extension(loopback())
                    .uri("/control/genesis")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(genesis.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&bytes[..], br#""Complete""#);

        // `self` exists; an unknown memory is a 404.
        let self_memory = app
            .clone()
            .oneshot(
                Request::builder()
                    .extension(loopback())
                    .uri("/control/memory?name=self")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(self_memory.status(), StatusCode::OK);

        let missing = app
            .oneshot(
                Request::builder()
                    .extension(loopback())
                    .uri("/control/memory?name=person/nobody")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_platform_message_runs_a_turn() {
        // A born agent with a scripted model in app state: a /platform/message delivers a participant
        // turn and returns the agent's reply.
        let server =
            Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
        server
            .control()
            .create_agent(&zuihitsu::SeedSelf {
                agent_name: "Kestrel".to_owned(),
                persona: "An assistant.".to_owned(),
                seed_entries: vec![],
            })
            .unwrap();
        let model: Arc<dyn zuihitsu::ModelClient> =
            Arc::new(ScriptedModel::new([Completion::Reply(
                "Hi there.".to_owned(),
            )]));
        let app = router(AppState {
            server: Arc::new(server),
            model: Some(model),
            snapshot_dir: None,
            control_keys: no_keys(),
            platform_keys: no_keys(),
        });

        let body = serde_json::json!({
            "locator": { "platform": "discord", "scope_path": "general" },
            "sender": "dave",
            "text": "hello",
            "present": ["dave"],
        });
        let response = app
            .oneshot(
                Request::builder()
                    .extension(loopback())
                    .method("POST")
                    .uri("/platform/message")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&bytes[..], br#"{"Reply":"Hi there."}"#);
    }

    #[tokio::test]
    async fn interactions_surface_the_recorded_model_calls() {
        // After a scripted turn, `/control/interactions` returns the model-interaction record.
        let server =
            Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
        server
            .control()
            .create_agent(&zuihitsu::SeedSelf {
                agent_name: "Kestrel".to_owned(),
                persona: "An assistant.".to_owned(),
                seed_entries: vec![],
            })
            .unwrap();
        let model: Arc<dyn zuihitsu::ModelClient> =
            Arc::new(ScriptedModel::new([Completion::Reply(
                "Hi there.".to_owned(),
            )]));
        let app = router(AppState {
            server: Arc::new(server),
            model: Some(model),
            snapshot_dir: None,
            control_keys: no_keys(),
            platform_keys: no_keys(),
        });

        let body = serde_json::json!({
            "locator": { "platform": "discord", "scope_path": "general" },
            "sender": "dave",
            "text": "hello",
            "present": ["dave"],
        });
        app.clone()
            .oneshot(
                Request::builder()
                    .extension(loopback())
                    .method("POST")
                    .uri("/platform/message")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .extension(loopback())
                    .uri("/control/interactions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let calls: Vec<ModelCall> = serde_json::from_slice(&bytes).unwrap();
        // The single reply step was recorded, with its completion and a non-empty digest.
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].completion,
            Completion::Reply("Hi there.".to_owned())
        );
        assert!(!calls[0].request_digest.is_empty());
    }

    #[tokio::test]
    async fn snapshot_endpoint_writes_a_file_or_409s_when_disabled() {
        let born = || {
            let server =
                Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
            server
                .control()
                .create_agent(&zuihitsu::SeedSelf {
                    agent_name: "Kestrel".to_owned(),
                    persona: "An assistant.".to_owned(),
                    seed_entries: vec![],
                })
                .unwrap();
            server
        };
        let post = || {
            Request::builder()
                .extension(loopback())
                .method("POST")
                .uri("/control/snapshot")
                .body(Body::empty())
                .unwrap()
        };

        // Enabled: the endpoint writes a snapshot into the configured directory.
        let dir = std::env::temp_dir().join(format!(
            "zuihitsu-snapep-{}",
            zuihitsu::MemoryId::generate().0
        ));
        let app = router(AppState {
            server: Arc::new(born()),
            model: None,
            snapshot_dir: Some(dir.clone()),
            control_keys: no_keys(),
            platform_keys: no_keys(),
        });
        let response = app.oneshot(post()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(zuihitsu::snapshot::latest(&dir).unwrap().is_some());
        std::fs::remove_dir_all(&dir).unwrap();

        // Disabled (no snapshot dir): the endpoint answers 409.
        let app = router(AppState {
            server: Arc::new(born()),
            model: None,
            snapshot_dir: None,
            control_keys: no_keys(),
            platform_keys: no_keys(),
        });
        let response = app.oneshot(post()).await.unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    /// A router over a fresh in-memory server with the given per-surface keys — for the auth tests.
    fn keyed_app(control: &[&str], platform: &[&str]) -> axum::Router {
        let server = Arc::new(
            Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap(),
        );
        let keys = |k: &[&str]| -> Arc<[String]> { k.iter().map(|s| s.to_string()).collect() };
        router(AppState {
            server,
            model: None,
            snapshot_dir: None,
            control_keys: keys(control),
            platform_keys: keys(platform),
        })
    }

    /// A GET request from `peer`, optionally bearing `key`.
    fn get(peer: ConnectInfo<SocketAddr>, uri: &str, key: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().extension(peer).uri(uri);
        if let Some(key) = key {
            builder = builder.header("authorization", format!("Bearer {key}"));
        }
        builder.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn a_remote_peer_without_a_valid_key_is_rejected() {
        let app = keyed_app(&["op-key"], &["pf-key"]);
        // No Authorization header → 401, on both surfaces.
        let response = app
            .clone()
            .oneshot(get(remote(), "/control/genesis", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .extension(remote())
                    .method("POST")
                    .uri("/platform/message")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        // A wrong key → 401.
        let response = app
            .oneshot(get(remote(), "/control/genesis", Some("nope")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_valid_key_authorizes_only_its_own_surface() {
        let app = keyed_app(&["op-key"], &["pf-key"]);
        // The control key opens a control route.
        let response = app
            .clone()
            .oneshot(get(remote(), "/control/genesis", Some("op-key")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        // ...but the same key does NOT open a platform route — the surfaces are isolated.
        let response = app
            .oneshot(
                Request::builder()
                    .extension(remote())
                    .method("POST")
                    .uri("/platform/message")
                    .header("authorization", "Bearer op-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_loopback_peer_is_trusted_without_a_key() {
        // Even with keys configured, a loopback peer needs none — the local CLI keeps working.
        let app = keyed_app(&["op-key"], &["pf-key"]);
        let response = app
            .oneshot(get(loopback(), "/control/genesis", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn an_empty_key_list_is_fail_closed_for_remote_peers() {
        // No keys configured + a remote peer → always rejected, so a wide bind with no keys is a
        // silent lockout, never a silent exposure.
        let app = keyed_app(&[], &[]);
        let response = app
            .oneshot(get(remote(), "/control/genesis", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
