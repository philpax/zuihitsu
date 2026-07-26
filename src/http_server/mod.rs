//! The long-running HTTP server — `zuihitsu` with no subcommand boots this (spec §Clients and the
//! server boundary). It opens the instance the config selects (acquiring the single-writer log lock),
//! reconciles the graph, connects any configured MCP servers, runs the background scheduler driver,
//! and serves the API the CLI and a future web console drive. The HTTP layer lives here, in the
//! binary, so the library stays transport-agnostic.
//!
//! Split by surface: [`control`] (operator handlers), [`platform`] (participant handlers), [`auth`]
//! (bearer-key middleware), [`error`] (HTTP error responses), [`console`] (embedded web console),
//! [`serve_error`] (startup [`ServeError`]). This file owns the boot sequence, the [`router`], and
//! the shared [`AppState`].

mod auth;
mod console;
mod control;
mod error;
mod platform;
mod read_only;
mod serve_error;
mod stream;

pub use serve_error::ServeError;

use std::{net::SocketAddr, path::Path, sync::Arc, time::Duration};

use axum::{
    Router,
    routing::{get, post},
};
use tokio::net::TcpListener;
use zuihitsu::{
    EnvConfig, Graph, HttpFetcher, HttpFetcherConfig, ModelArbiter, ModelClient, OpenAiClient,
    OpenAiEmbedder, RetryingModel, RmcpHost, Server, SnapshotSchedule, SqliteStore,
    SqliteVectorIndex, SystemClock, VectorIndex,
    metrics::{LATENCY_BUCKETS, describe},
    model::embed::Embedder,
    snapshot,
};

use auth::{require_control_key, require_platform_key};
use console::{ShutdownFlag, console, ensure_parent_dir};
use control::{
    arbitrations, confirm_merge, create_agent, designate_primary, edit_self, entries, env_config,
    events, genesis, health, imprint, interactions, lua_api, maintenance_canonicalize,
    maintenance_consolidate, maintenance_link_cleanup, memories, memory, merge_proposals, metrics,
    prompt_status, recurring, register_prompt, retract_attestation, retract_entry, run_lua,
    sessions, set_settings, settings, snapshot as snapshot_handler, unmerge,
};
use platform::{join, link, message, message_stream, project, roster, self_memory, write_context};
use read_only::refuse_mutations_when_read_only;

/// Shared HTTP handler state: the agent server behind an `Arc`, and the model client the conversing
/// endpoints (`imprint`, `route_message`) drive — `None` when no model endpoint is configured, in
/// which case those endpoints return `503`.
#[derive(Clone)]
struct AppState {
    server: Arc<Server>,
    model: Option<Arc<dyn ModelClient>>,
    /// The same client as `model`, as its concrete resilience wrapper — the handle
    /// `/control/health` reads the circuit state and last failure from. `None` when no model is
    /// configured (and in router tests that inject a bare fake as `model`).
    backend: Option<Arc<RetryingModel>>,
    /// Where an on-demand snapshot is written — `Some` when snapshotting is enabled, `None`
    /// otherwise (the snapshot endpoint then answers `409`).
    snapshot_dir: Option<std::path::PathBuf>,
    /// The committed-event fan-out behind `GET /control/events/stream` — one store subscription,
    /// shared by every connected viewer.
    live: Arc<stream::LiveEvents>,
    /// The shared shutdown flag ([`ShutdownFlag`]). The long-lived SSE handler watches its own clone
    /// so it breaks its otherwise-unbounded loop when Ctrl-C fires, letting the connection drain — the
    /// same signal `with_graceful_shutdown` and the background drivers observe.
    shutdown: ShutdownFlag,
    /// The Prometheus metrics handle — `render()` produces the `/control/metrics` text. `None` when
    /// the recorder could not be installed (a boot failure leaves the server up but the metrics
    /// endpoint answers `503`).
    metrics: Option<metrics_exporter_prometheus::PrometheusHandle>,
    /// When the server booted, for the `uptime_seconds` gauge.
    boot: std::time::Instant,
    /// Valid API keys for `/control/*`; a remote peer must present one, a loopback peer is trusted
    /// without one. `Arc<[String]>` so the per-request state clone is a refcount bump.
    control_keys: Arc<[String]>,
    /// The registered platform connectors as `(platform, key)` pairs — the platform surface's
    /// credentials. A `/platform/*` request's bearer key resolves to exactly one platform, which scopes
    /// every operation to that platform (a loopback request is the `direct` interface). `Arc<[…]>` so the
    /// per-request state clone is a refcount bump.
    platform_connectors: Arc<[(String, String)]>,
    /// The environmental config this instance booted from, for the read-only config view. Serializing
    /// it redacts the secrets (API keys serialize as counts, MCP env and HTTP headers as their names).
    config: Arc<EnvConfig>,
    /// Whether the server is booted in read-only mode — mutating handlers return `409` when true.
    read_only: bool,
}

/// How often the background indexer catches the vector index up to the log. Indexing is off the hot
/// path (spec §Storage → vector store), so a short poll keeps search fresh without a per-commit cost;
/// an idle tick is cheap (it reads the log from the index's cursor and finds nothing).
const INDEX_TICK_SECONDS: u64 = 5;

/// How often the background describer catches memory descriptions up to the log. Like indexing, it runs
/// off the hot path (spec §Write path → regenerate off the hot path), so a short poll keeps descriptions
/// fresh without a per-turn cost; an idle tick is cheap.
const DESCRIBE_TICK_SECONDS: u64 = 5;

/// How often the background link-inference pass extracts relationships implicit in memory content
/// (spec §Write path → link inference). It runs off the hot path and an idle tick is cheap (a head
/// check against the cursor).
const LINK_INFERENCE_TICK_SECONDS: u64 = 7;

/// How often the background idle sweep closes-with-flush sessions idle past the gap, so a conversation
/// never messaged again still has its working state consolidated. Coarse — the idle gap is measured in
/// minutes — and an idle tick is cheap (a query for open sessions, then a per-session activity check).
const SWEEP_TICK_SECONDS: u64 = 60;

/// How often the checkpoint sweep evaluates live sessions for a mid-session flush (spec §Compaction →
/// checkpoint flush). The gates (substance, cooldown, audience) do the real rate-limiting; the tick
/// only bounds how quickly an eligible session is noticed, and an idle tick is cheap (per-live-session
/// buffer reads, no model call).
const CHECKPOINT_TICK_SECONDS: u64 = 30;

/// How often the maintenance driver ticks (spec §Write path → maintenance passes). Each pass's own
/// activity gate does the real rate-limiting; the tick only bounds how quickly a pass notices it
/// has work, and an idle tick is cheap (a cursor compare, no model call). The actual tick interval
/// is read from `MaintenanceSettings::tick_seconds` at runtime; this is the default.
const MAINTENANCE_TICK_SECONDS: u64 = 60;

/// Build the multi-thread tokio runtime and run the server to completion — the synchronous entry the
/// CLI calls when invoked with no subcommand.
pub fn run_blocking(config_path: &Path, cli_read_only: bool) -> Result<(), ServeError> {
    let config = EnvConfig::load(config_path).map_err(ServeError::Config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(ServeError::Runtime)?;
    runtime.block_on(serve(config, cli_read_only))
}

/// Open the instance, start the scheduler driver, and serve the HTTP API until interrupted, then shut
/// down gracefully (stop accepting, stop the driver, tear down live sessions).
async fn serve(config: EnvConfig, cli_read_only: bool) -> Result<(), ServeError> {
    // The CLI flag overrides the config: `--read-only` forces read-only even when
    // `[serving] read_only = false`.
    let read_only = config.serving.read_only || cli_read_only;
    if read_only {
        return serve_read_only(config).await;
    }
    let bind = config.serving.bind;
    // Capture the booted config for the read-only config view before its parts are moved out below
    // (the MCP table into `connect_mcp`, the API keys into the auth layers).
    let env_config = Arc::new(config.clone());
    // Capture the boot-log fields early, before `config.mcp` is moved into `connect_mcp`.
    let (storage_dir, model_endpoint, embedding_endpoint) = boot_log_fields(&config);
    // The three databases share one directory (the storage config); ensuring the event log's parent
    // creates that directory for all of them.
    let event_log = config.storage.event_log();
    let graph_path = config.storage.graph();
    let vectors_path = config.storage.vectors();
    ensure_parent_dir(&event_log)?;
    let store = SqliteStore::open(&event_log).map_err(|source| ServeError::OpenEventLog {
        path: event_log.clone(),
        source,
    })?;
    // Restore the graph from the latest snapshot when it leads the on-disk graph (a fresh, deleted, or
    // corrupt graph), before the file is opened — so boot replays only the tail (spec §Snapshots).
    let snapshot_dir = config.snapshots.effective_dir(&graph_path);
    if config.snapshots.enabled
        && let Some(head) =
            snapshot::restore_if_stale(&graph_path, &snapshot_dir).map_err(ServeError::Snapshot)?
    {
        tracing::info!(head = head.0, "restored the graph from a snapshot");
    }
    let graph = Graph::open(&graph_path).map_err(|source| ServeError::OpenGraph {
        path: graph_path.clone(),
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
        let vectors = SqliteVectorIndex::open(&vectors_path, config.embedding.dimensions).map_err(
            |source| ServeError::OpenVectors {
                path: vectors_path.clone(),
                source,
            },
        )?;
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
    // A configured model must declare its context window: the agent's compaction budget derives from
    // it, and the OpenAI-style API does not report it (see `docs/model_management.md`).
    if !config.model.endpoint.is_empty() {
        let context_length = config
            .model
            .context_length
            .ok_or(ServeError::MissingContextLength)?;
        server.set_model_context_length(context_length);
    }
    let status = server.boot()?;

    // Install the Prometheus metrics recorder and declare every metric's help/type (spec
    // §Observability → metrics). The recorder is process-global (one agent per process); the handle
    // renders the `/control/metrics` text. A failure to install is non-fatal — the server serves on
    // without the metrics endpoint.
    let metrics = install_metrics();
    let boot = std::time::Instant::now();

    // If the embedding model changed since the vectors were last built, re-embed the whole log before
    // serving — blocking here so requests are refused until the index is rebuilt in the new model's
    // space, rather than answered from a silently-incompatible one (spec §Storage → vector store).
    server.reembed_if_embedding_model_changed().await?;

    // Connect the configured MCP servers once, before the server is shared (`connect_mcp` is `&mut`).
    if !config.mcp.is_empty() {
        server.connect_mcp(Arc::new(RmcpHost), config.mcp).await?;
    }

    let settings = server.control().settings()?;

    // Attach the in-house web fetcher backing `web.markdown`, built from the web settings. The
    // transport-shaping values are read here at construction, so a change to them takes effect on
    // restart; the SSRF guard refuses private and loopback addresses unless the operator opts in.
    let web = &settings.web;
    let fetcher = HttpFetcher::new(HttpFetcherConfig {
        timeout: Duration::from_secs(web.fetch_timeout_seconds.max(1) as u64),
        max_response_bytes: web.max_response_bytes.max(0) as u64,
        user_agent: web.user_agent.clone(),
        allow_private_addresses: web.allow_private_addresses,
    })
    .map_err(ServeError::Web)?;
    server.connect_web(Arc::new(fetcher), web.max_markdown_chars.max(0) as usize);

    let tick = Duration::from_secs(settings.scheduler.tick_seconds.max(1) as u64);

    // The model client the conversing endpoints use, built from config. Absent endpoint → no model →
    // those endpoints answer 503 rather than failing at call time. The real client is wrapped in
    // the transport-resilience decorator here, at serving construction, so every caller — the turn
    // loop and the background workers — shares one retry policy and one circuit breaker; the
    // concrete handle is kept alongside the trait object so `/control/health` can read the circuit.
    let backend: Option<Arc<RetryingModel>> = if config.model.endpoint.is_empty() {
        tracing::warn!("no model endpoint configured; conversing endpoints will return 503");
        None
    } else {
        Some(Arc::new(RetryingModel::new(
            Arc::new(OpenAiClient::new(&config.model)),
            &config.model.resilience,
        )))
    };
    // The shared client is arbitrated so a waiting conversation turn dispatches ahead of queued
    // background synthesis (spec §Write path → model sharing). The conversation path holds the
    // turn-priority handle; each background worker holds a background handle minted from the same
    // arbiter, so a background pass structurally cannot dispatch at turn priority.
    let arbiter: Option<Arc<ModelArbiter>> = backend
        .clone()
        .map(|backend| ModelArbiter::new(backend as Arc<dyn ModelClient>));
    let model: Option<Arc<dyn ModelClient>> = arbiter.as_ref().map(|arbiter| arbiter.turn());
    let server = Arc::new(server);

    // The process's single shutdown source: one Ctrl-C listener latches a flag that graceful
    // shutdown, every background driver, and the streaming handlers observe through their own clones,
    // so one interrupt stops them all without each registering its own signal.
    let shutdown = ShutdownFlag::install();

    // The background scheduler driver fires due wake-ups on its own timer (spec §Scheduled work),
    // stopping on the same Ctrl-C that ends the HTTP server.
    let driver = tokio::spawn({
        let server = server.clone();
        let shutdown = shutdown.clone();
        async move { server.run_scheduler(tick, shutdown.wait()).await }
    });

    // The background indexer catches the vector index up to the log off the hot path (spec §Storage →
    // vector store). A no-op (returns immediately) on a graph-only instance.
    let indexer = tokio::spawn({
        let server = server.clone();
        let shutdown = shutdown.clone();
        async move {
            server
                .run_indexer(Duration::from_secs(INDEX_TICK_SECONDS), shutdown.wait())
                .await
        }
    });

    // The background describer regenerates memory descriptions (and arbitration and temporal
    // extraction) off the hot path (spec §Write path → regenerate off the hot path). Spawned only when
    // a model is configured; without one there is nothing to run the synthesis call.
    let describer = arbiter.as_ref().map(|arbiter| {
        tokio::spawn({
            let server = server.clone();
            let arbiter = arbiter.clone();
            let shutdown = shutdown.clone();
            async move {
                server
                    .run_describer(
                        arbiter,
                        Duration::from_secs(DESCRIBE_TICK_SECONDS),
                        shutdown.wait(),
                    )
                    .await
            }
        })
    });

    // The background link-inference pass extracts relationships implicit in memory content off the hot
    // path (spec §Write path → link inference). Spawned only when a model is configured; without one
    // there is nothing to run the extraction call.
    let link_inference = arbiter.as_ref().map(|arbiter| {
        tokio::spawn({
            let server = server.clone();
            let model = arbiter.background();
            let shutdown = shutdown.clone();
            async move {
                server
                    .run_link_inference(
                        model,
                        Duration::from_secs(LINK_INFERENCE_TICK_SECONDS),
                        shutdown.wait(),
                    )
                    .await
            }
        })
    });

    // The background maintenance driver runs consolidation, canonical-profile, and link-cleanup passes
    // off the hot path (spec §Write path → maintenance passes). Spawned only when a model is
    // configured; the synthesis calls need one.
    let maintenance = arbiter.as_ref().map(|arbiter| {
        tokio::spawn({
            let server = server.clone();
            let model = arbiter.background();
            let shutdown = shutdown.clone();
            async move {
                server
                    .run_maintenance(
                        model,
                        Duration::from_secs(MAINTENANCE_TICK_SECONDS),
                        shutdown.wait(),
                    )
                    .await
            }
        })
    });

    // The background idle sweep closes-with-flush sessions idle past the gap, so a conversation never
    // messaged again still has its working state consolidated (spec §Compaction → pre-compaction
    // flush). Spawned only when a model is configured; the flush turn needs one.
    let sweeper = arbiter.as_ref().map(|arbiter| {
        tokio::spawn({
            let server = server.clone();
            let model = arbiter.background();
            let shutdown = shutdown.clone();
            async move {
                server
                    .run_sweeper(
                        model,
                        Duration::from_secs(SWEEP_TICK_SECONDS),
                        shutdown.wait(),
                    )
                    .await
            }
        })
    });

    // The background checkpoint sweep flushes a live session's working state to memory mid-session
    // (spec §Compaction → checkpoint flush), so a parallel conversation can read it before this one
    // goes idle. Spawned only when a model is configured; the flush turn needs one.
    let checkpoint_sweeper = arbiter.as_ref().map(|arbiter| {
        tokio::spawn({
            let server = server.clone();
            let model = arbiter.background();
            let shutdown = shutdown.clone();
            async move {
                server
                    .run_checkpoint_sweeper(
                        model,
                        Duration::from_secs(CHECKPOINT_TICK_SECONDS),
                        shutdown.wait(),
                    )
                    .await
            }
        })
    });

    // The background snapshotter checkpoints the graph on its own activity-gated cadence (spec
    // §Snapshots), when enabled. Stops on the same shutdown signal.
    let snapshotter = config.snapshots.enabled.then(|| {
        tokio::spawn({
            let server = server.clone();
            let shutdown = shutdown.clone();
            let schedule = SnapshotSchedule {
                dir: snapshot_dir.clone(),
                check_interval: Duration::from_secs(config.snapshots.check_interval_seconds.max(1)),
                min_new_events: config.snapshots.min_new_events,
                keep: config.snapshots.keep,
            };
            async move { server.run_snapshotter(schedule, shutdown.wait()).await }
        })
    });

    let app = router(AppState {
        model,
        backend,
        snapshot_dir: config.snapshots.enabled.then_some(snapshot_dir),
        boot,
        ..shared_app_state(server.clone(), shutdown.clone(), metrics, env_config)
    });
    let mcp_summary = server.mcp_summary();
    tracing::info!(
        %bind,
        %storage_dir,
        %model_endpoint,
        %embedding_endpoint,
        ?status,
        read_only = false,
        mcp_servers = mcp_summary.len(),
        "zuihitsu serving"
    );
    for (name, tools) in mcp_summary {
        tracing::info!(mcp = %name, tools, "mcp server up");
    }
    serve_until_shutdown(app, bind, &shutdown).await?;
    let _ = driver.await;
    let _ = indexer.await;
    if let Some(describer) = describer {
        let _ = describer.await;
    }
    if let Some(link_inference) = link_inference {
        let _ = link_inference.await;
    }
    if let Some(maintenance) = maintenance {
        let _ = maintenance.await;
    }
    if let Some(sweeper) = sweeper {
        let _ = sweeper.await;
    }
    if let Some(checkpoint_sweeper) = checkpoint_sweeper {
        let _ = checkpoint_sweeper.await;
    }
    if let Some(snapshotter) = snapshotter {
        let _ = snapshotter.await;
    }
    server.shutdown().await;
    Ok(())
}

/// Install the Prometheus metrics recorder and declare every metric's help/type (spec
/// §Observability → metrics). The recorder is process-global (one agent per process); the handle
/// renders the `/control/metrics` text. A failure to install is non-fatal — the server serves on
/// without the metrics endpoint.
fn install_metrics() -> Option<metrics_exporter_prometheus::PrometheusHandle> {
    match metrics_exporter_prometheus::PrometheusBuilder::new().set_buckets(LATENCY_BUCKETS) {
        Ok(builder) => match builder.install_recorder() {
            Ok(handle) => {
                describe();
                Some(handle)
            }
            Err(error) => {
                tracing::warn!(%error, "could not install the metrics recorder; /control/metrics is disabled");
                None
            }
        },
        Err(error) => {
            tracing::warn!(%error, "could not configure the metrics recorder; /control/metrics is disabled");
            None
        }
    }
}

/// The boot-log fields shared by both boot paths: the resolved storage directory and the model
/// and embedding endpoints (host + model id, no secrets — keys are never logged).
fn boot_log_fields(config: &EnvConfig) -> (String, String, String) {
    let storage_dir = config.storage.dir.display().to_string();
    let model_endpoint = if config.model.endpoint.is_empty() {
        "(none)".to_owned()
    } else {
        format!("{} [{}]", config.model.endpoint, config.model.llm)
    };
    let embedding_endpoint = if config.embedding.endpoint.is_empty() {
        "(none)".to_owned()
    } else {
        format!(
            "{} [{} · {}d]",
            config.embedding.endpoint, config.embedding.model, config.embedding.dimensions
        )
    };
    (storage_dir, model_endpoint, embedding_endpoint)
}

/// Bind the listener, serve the app until interrupted, then shut down gracefully. The driver-join
/// tail is the caller's responsibility — the live path joins its spawned drivers, the read-only
/// path has none.
async fn serve_until_shutdown(
    app: axum::Router,
    bind: SocketAddr,
    shutdown: &ShutdownFlag,
) -> Result<(), ServeError> {
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|source| ServeError::Bind { addr: bind, source })?;
    // `into_make_service_with_connect_info` surfaces each connection's peer address to the auth
    // middleware (a bare `axum::serve(listener, app)` would not), so a loopback peer can be trusted.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown.clone().wait())
    .await
    .map_err(ServeError::Serve)?;
    tracing::info!("shutdown signal received; stopping");
    Ok(())
}

/// The [`AppState`] fields both boot paths share. The read-only path takes it whole; the live path
/// takes it with `..` and overrides the four fields it alone has (`model`, `backend`, `snapshot_dir`,
/// and a `boot` instant taken before its longer startup). `config` arrives already `Arc`-wrapped
/// because the live path captures it before the MCP table and the API keys are moved out of the owned
/// config, and it is the config *as booted*, so `serving.read_only` reads true under `--read-only`
/// even when the file said otherwise.
fn shared_app_state(
    server: Arc<Server>,
    shutdown: ShutdownFlag,
    metrics: Option<metrics_exporter_prometheus::PrometheusHandle>,
    config: Arc<EnvConfig>,
) -> AppState {
    AppState {
        live: Arc::new(stream::LiveEvents::start(&server)),
        shutdown,
        server,
        model: None,
        backend: None,
        snapshot_dir: None,
        metrics,
        boot: std::time::Instant::now(),
        control_keys: config.serving.control_keys.clone().into(),
        platform_connectors: config
            .platform_connectors
            .iter()
            .map(|(platform, connector)| (platform.clone(), connector.key.clone()))
            .collect(),
        read_only: config.serving.read_only,
        config,
    }
}

/// Serve the console against an at-rest instance's data **without taking the single-writer lock**,
/// performing no writes, and spawning no background work (spec §Boot). The store, graph, and vector
/// index all open read-only; `boot()` is skipped entirely (the at-rest graph is already materialized
/// from the last live boot); no drivers spawn; no model client or arbiter is built. Mutating requests
/// return `409` from [`refuse_mutations_when_read_only`].
///
/// **What is frozen and what is not.** The graph and the vector index are frozen at boot: nothing
/// re-materializes them, so every graph-backed read (memories, entries, briefs, relations) answers
/// from the state the last live boot left behind. The event log is *not* frozen — each read is a fresh
/// query against the SQLite file — so if the agent is running concurrently (which this mode permits,
/// since it takes no lock), the log and the SSE catch-up serve events newer than the graph that has
/// not seen them. The two agree only while the instance is genuinely at rest, which is the intended
/// use. The SSE stream never pushes a live tail regardless: a read-only store has no subscriber feed,
/// so the stream serves its initial catch-up and then idles. Restart to advance the frozen half.
async fn serve_read_only(mut config: EnvConfig) -> Result<(), ServeError> {
    let bind = config.serving.bind;
    let event_log = config.storage.event_log();
    let graph_path = config.storage.graph();
    let vectors_path = config.storage.vectors();
    // The read-only path does not create the storage directory — the instance must already exist on
    // disk (booted live at least once). Creating it here would be a write outside the read-only
    // contract; a missing directory surfaces as an open error naming the path.
    let store =
        SqliteStore::open_read_only(&event_log).map_err(|source| ServeError::OpenEventLog {
            path: event_log.clone(),
            source,
        })?;
    let graph = Graph::open_read_only(&graph_path).map_err(|source| ServeError::OpenGraph {
        path: graph_path.clone(),
        source,
    })?;
    // Semantic retrieval is read-only too: open the index without creating the virtual table. The
    // embedder is still built so `memory.search` can embed the query, but the index file must already
    // exist and be populated by a prior live boot.
    let retrieval = if config.embedding.endpoint.is_empty() {
        None
    } else {
        let embedder: Arc<dyn Embedder> = Arc::new(OpenAiEmbedder::new(&config.embedding));
        let vectors = SqliteVectorIndex::open_read_only(&vectors_path, config.embedding.dimensions)
            .map_err(|source| ServeError::OpenVectors {
                path: vectors_path.clone(),
                source,
            })?;
        Some((embedder, Box::new(vectors) as Box<dyn VectorIndex>))
    };
    let server = match retrieval {
        Some((embedder, vectors)) => Server::with_retrieval(
            Box::new(store),
            graph,
            Box::new(SystemClock),
            embedder,
            vectors,
        ),
        None => Server::new(Box::new(store), graph, Box::new(SystemClock)),
    };
    // No boot() — it writes (template reconciliation, materialization, cursor reseeding). The at-rest
    // graph is already materialized from the last live boot.
    // No model client or arbiter — no model calls will be made. `model` and `backend` are `None`.
    // No MCP or web connection — those spawn subprocesses or build HTTP clients.
    // No background drivers — the read-only server performs no autonomous work.
    let server = Arc::new(server);
    let shutdown = ShutdownFlag::install();
    let metrics = install_metrics();
    // The config the state serves is the config *as booted*, so `/control/config` agrees with
    // `/control/health` for an operator who reached read-only mode through the CLI flag rather than
    // through the file.
    config.serving.read_only = true;
    let (storage_dir, model_endpoint, embedding_endpoint) = boot_log_fields(&config);
    let app = router(shared_app_state(
        server.clone(),
        shutdown.clone(),
        metrics,
        Arc::new(config),
    ));
    tracing::info!(
        %bind,
        %storage_dir,
        %model_endpoint,
        %embedding_endpoint,
        read_only = true,
        "zuihitsu serving"
    );
    serve_until_shutdown(app, bind, &shutdown).await?;
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
///
/// Each surface also carries [`refuse_mutations_when_read_only`], inside its auth layer, which is what
/// makes a read-only boot's `409` structural: the gate reads the request method, so a mutating route
/// added later is refused without anyone remembering to guard it. It sits per sub-router rather than
/// at the top for a second reason — the top-level router's fallback is the console, and a `POST` to an
/// unknown path should stay a routing failure rather than become a read-only refusal.
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
        .route("/merge-proposals", get(merge_proposals))
        .route("/merge", post(confirm_merge))
        .route("/unmerge", post(unmerge))
        .route("/designate-primary", post(designate_primary))
        .route("/interactions", get(interactions))
        .route("/events", get(events))
        .route("/events/stream", get(stream::events_stream))
        .route("/snapshot", post(snapshot_handler))
        .route("/settings", get(settings).put(set_settings))
        .route("/config", get(env_config))
        .route("/metrics", get(metrics))
        .route("/imprint", post(imprint))
        .route("/self", post(edit_self))
        .route("/retract", post(retract_entry))
        .route("/retract-attestation", post(retract_attestation))
        .route("/lua", post(run_lua))
        .route("/lua-api", get(lua_api))
        .route("/prompt", post(register_prompt))
        .route("/prompt-status", get(prompt_status))
        .route("/maintenance/consolidate", post(maintenance_consolidate))
        .route("/maintenance/canonicalize", post(maintenance_canonicalize))
        .route("/maintenance/link-cleanup", post(maintenance_link_cleanup))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            refuse_mutations_when_read_only,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_control_key,
        ));
    // The participant surface: delivering turns, mid-session joins, and roster resyncs (spec §Clients
    // → platform clients). It carries platform identity in the payload, never operator authority.
    let platform = Router::new()
        .route("/messages", post(message))
        .route("/messages/stream", post(message_stream))
        .route("/join", post(join))
        .route("/roster", post(roster))
        .route("/context", post(write_context))
        .route("/project", post(project))
        .route("/self", get(self_memory))
        .route("/link", post(link))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            refuse_mutations_when_read_only,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_platform_key,
        ));
    Router::new()
        .nest("/control", control)
        .nest("/platform", platform)
        .with_state(state)
        // Everything else is the embedded web console: its assets by path, and any client-side route
        // (no matching asset) served `index.html` so the single-page app can route it. The console is
        // served from the agent's own origin, so it connects back to `/control` keylessly as a
        // loopback peer.
        .fallback(console)
}

#[cfg(test)]
mod tests;
