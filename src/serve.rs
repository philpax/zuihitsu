//! The long-running HTTP server — `zuihitsu` with no subcommand boots this (spec §Clients and the
//! server boundary). It opens the instance the config selects (acquiring the single-writer log lock),
//! reconciles the graph, connects any configured MCP servers, runs the background scheduler driver,
//! and serves the API the CLI and a future web debugger drive. The HTTP layer lives here, in the
//! binary, so the library stays transport-agnostic; `/` is reserved for the web debugger, the
//! operator surface lives under `/control`, and the participant surface under `/platform`.

use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use zuihitsu::{
    Arbitration, ConfigError, ConversationLocator, EntryView, EnvConfig, Graph, GraphError,
    MemoryView, ModelClient, OpenAiClient, Rollout, SeedSelf, Server, ServerError, SessionView,
    Settings, SqliteStore, StdioHost, StoreError, SystemClock, TurnOutcome, genesis::GenesisStatus,
};

/// Shared HTTP handler state: the agent server behind the `Arc` its facets are designed to share (so
/// each handler grabs a fresh `control()`/`platform()` per request), and the model client the
/// conversing endpoints (`imprint`, `route_message`) drive — `None` when no model endpoint is
/// configured, in which case those endpoints return `503`.
#[derive(Clone)]
struct AppState {
    server: Arc<Server>,
    model: Option<Arc<dyn ModelClient>>,
}

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
    let graph = Graph::open(&config.storage.graph).map_err(|source| ServeError::OpenGraph {
        path: config.storage.graph.clone(),
        source,
    })?;
    let mut server = Server::new(Box::new(store), graph, Box::new(SystemClock));
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

    let app = router(AppState {
        server: server.clone(),
        model,
    });
    let listener = TcpListener::bind(bind).await.map_err(ServeError::Bind)?;
    tracing::info!(?status, %bind, "zuihitsu serving");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(ServeError::Serve)?;

    tracing::info!("shutdown signal received; stopping");
    let _ = driver.await;
    server.shutdown().await;
    Ok(())
}

/// The API router. `/` is the reserved web-debugger root; the operator surface lives under `/control`
/// and the participant surface under `/platform` (filled in by later commits).
fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/control/health", get(health))
        // The operator surface: agent creation and read-only inspection (spec §Clients → control
        // clients). The CLI and the future web debugger drive these.
        .route("/control/agent", post(create_agent))
        .route("/control/genesis", get(genesis))
        .route("/control/memory", get(memory))
        .route("/control/memories", get(memories))
        .route("/control/entries", get(entries))
        .route("/control/sessions", get(sessions))
        .route("/control/recurring", get(recurring))
        .route("/control/arbitrations", get(arbitrations))
        .route("/control/settings", get(settings).put(set_settings))
        .route("/control/imprint", post(imprint))
        // The participant surface: delivering turns and mid-session joins (spec §Clients → platform
        // clients). It carries platform identity in the payload, never operator authority.
        .route("/platform/message", post(message))
        .route("/platform/join", post(join))
        .with_state(state)
}

/// The reserved web-debugger root — a placeholder until the frontend lands.
async fn root() -> impl IntoResponse {
    (
        StatusCode::OK,
        "zuihitsu is serving. The web debugger will live here; the API is under /control and \
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
        .imprint(&**model, &request.text)
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
            &**model,
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
        http::{Request, StatusCode},
    };
    use std::sync::Arc;
    use tower::ServiceExt;
    use zuihitsu::{Completion, ManualClock, ScriptedModel, Server, time::Timestamp};

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
        });
        let response = app
            .oneshot(
                Request::builder()
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
}
