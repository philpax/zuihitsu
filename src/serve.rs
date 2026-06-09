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

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use serde::Serialize;
use tokio::net::TcpListener;
use zuihitsu::{
    ConfigError, EnvConfig, Graph, GraphError, Server, ServerError, SqliteStore, StoreError,
    SystemClock, genesis::GenesisStatus,
};

use zuihitsu::StdioHost;

/// Shared HTTP handler state: the agent server behind the `Arc` its facets are designed to share, so
/// each handler grabs a fresh `control()`/`platform()` per request.
#[derive(Clone)]
struct AppState {
    server: Arc<Server>,
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
    let server = Arc::new(server);

    // The background scheduler driver fires due wake-ups on its own timer (spec §Scheduled work),
    // stopping on the same Ctrl-C that ends the HTTP server.
    let driver = tokio::spawn({
        let server = server.clone();
        async move { server.run_scheduler(tick, shutdown_signal()).await }
    });

    let app = router(AppState {
        server: server.clone(),
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

/// A server-side error rendered as an HTTP response. Every [`ServerError`] is an
/// infrastructure/processing failure, so it maps to `500` with a JSON body; bad input is rejected at
/// the extractor (400) and missing resources are an empty/`404` shape in the handler, so neither
/// reaches here.
struct ApiError(ServerError);

impl From<ServerError> for ApiError {
    fn from(error: ServerError) -> Self {
        ApiError(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        tracing::error!(error = %self.0, "request failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: self.0.to_string(),
            }),
        )
            .into_response()
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
    use zuihitsu::{ManualClock, Server, time::Timestamp};

    /// The router serves `/control/health` over an in-memory server, with no real socket — `oneshot`
    /// drives one request through the tower service.
    #[tokio::test]
    async fn health_reports_genesis_status() {
        let server = Arc::new(
            Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap(),
        );
        let app = router(AppState { server });
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
}
