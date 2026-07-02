//! The operator surface (`/control/*`): agent creation, read-only inspection, settings and prompt
//! edits, the Lua console, and on-demand snapshots (spec §Clients → control clients). The CLI and the
//! web console drive these; the auth layer is applied to the whole surface in [`super::router`].

use axum::{
    Json,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use zuihitsu::{
    ApiEntry, Arbitration, BackendHealth, ConversationLocator, EntryView, EnvConfig, Event,
    LuaConsoleOutcome, MemoryView, MergeProposal, ModelCall, PromptTemplateName, Rollout, SeedSelf,
    Seq, SessionView, Settings, TurnOutcome, genesis::GenesisStatus,
};

use super::{AppState, error::ApiError};

/// The serving health/status: whether an agent exists yet, and the model transport's health — the
/// circuit-breaker state, the consecutive-failure count, and the last failure's cause — which the
/// console polls to drive its degraded-backend banner. `model` is `None` when no model endpoint is
/// configured (the conversing endpoints answer 503, which is its own signal).
#[derive(Serialize)]
pub(super) struct Health {
    genesis: GenesisStatus,
    model: Option<BackendHealth>,
}

pub(super) async fn health(State(state): State<AppState>) -> Result<Json<Health>, ApiError> {
    let genesis = state.server.control().genesis_status()?;
    Ok(Json(Health {
        genesis,
        model: state.backend.as_ref().map(|backend| backend.health()),
    }))
}

/// `POST /control/agent` — create the agent (or resume an interrupted genesis); idempotent.
pub(super) async fn create_agent(
    State(state): State<AppState>,
    Json(seed): Json<SeedSelf>,
) -> Result<Json<Rollout>, ApiError> {
    Ok(Json(state.server.control().create_agent(&seed)?))
}

/// `GET /control/genesis` — whether an agent exists and is ready.
pub(super) async fn genesis(
    State(state): State<AppState>,
) -> Result<Json<GenesisStatus>, ApiError> {
    Ok(Json(state.server.control().genesis_status()?))
}

/// `GET /control/config` — the environmental config this instance booted from (the TOML), read-only:
/// storage paths, model and embedding endpoints, the bind address, snapshots, and the MCP servers.
/// Secrets are redacted by the types themselves (API keys as counts, MCP env as its variable names).
pub(super) async fn env_config(State(state): State<AppState>) -> Json<EnvConfig> {
    Json((*state.config).clone())
}

/// `?format=` query retained for compatibility; the metrics endpoint renders Prometheus text only.
/// (The `metrics` crate's exporter is the single source of truth for the rendered shape; a JSON
/// variant would re-introduce the snapshot duplication the migration removed.)
#[derive(Deserialize)]
pub(super) struct MetricsFormatQuery {
    #[serde(default)]
    #[allow(dead_code)]
    format: Option<String>,
}

/// `GET /control/metrics` — the runtime metrics a Grafana (or any Prometheus scraper) consumes
/// directly, as Prometheus text-format. The instance-derived gauges (graph size, head, lag,
/// sessions, MCP) are refreshed from state on each scrape, then the recorder renders. `503` when the
/// recorder could not be installed at boot.
pub(super) async fn metrics(
    State(state): State<AppState>,
    Query(_): Query<MetricsFormatQuery>,
) -> Result<Response, ApiError> {
    let handle = state.metrics.as_ref().ok_or(ApiError::MetricsDisabled)?;
    state.server.control().refresh_gauges()?;
    let store_size_bytes = std::fs::metadata(state.config.storage.event_log().as_path())
        .ok()
        .map(|metadata| metadata.len());
    zuihitsu::metrics::set_process_gauges(state.boot.elapsed().as_secs_f64(), store_size_bytes);
    let body = handle.render();
    Ok((
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response())
}

/// A `?name=` query — a memory or entry name (which may contain `/` and `@`, so it rides as a query
/// parameter rather than a path segment).
#[derive(Deserialize)]
pub(super) struct NameQuery {
    name: String,
}

/// `GET /control/memory?name=` — inspect a memory by name; `404` if it does not exist.
pub(super) async fn memory(
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
pub(super) struct PrefixQuery {
    prefix: String,
}

/// `GET /control/memories?prefix=` — the live memories in a namespace, ordered by name.
pub(super) async fn memories(
    State(state): State<AppState>,
    Query(query): Query<PrefixQuery>,
) -> Result<Json<Vec<MemoryView>>, ApiError> {
    Ok(Json(state.server.control().memories(&query.prefix)?))
}

/// `GET /control/entries?name=` — a memory's local content entries (empty if the memory is unknown).
pub(super) async fn entries(
    State(state): State<AppState>,
    Query(query): Query<NameQuery>,
) -> Result<Json<Vec<EntryView>>, ApiError> {
    Ok(Json(state.server.control().entries(&query.name)?))
}

/// A `?platform=&scope=` query addressing a conversation by its locator.
#[derive(Deserialize)]
pub(super) struct LocatorQuery {
    platform: String,
    scope: String,
}

/// `GET /control/sessions?platform=&scope=` — the sessions of a conversation, oldest first.
pub(super) async fn sessions(
    State(state): State<AppState>,
    Query(query): Query<LocatorQuery>,
) -> Result<Json<Vec<SessionView>>, ApiError> {
    let locator = ConversationLocator::new(query.platform, query.scope);
    Ok(Json(state.server.control().sessions(&locator)?))
}

/// `GET /control/recurring` — the memories carrying a recurring occurrence.
pub(super) async fn recurring(
    State(state): State<AppState>,
) -> Result<Json<Vec<MemoryView>>, ApiError> {
    Ok(Json(state.server.control().recurring()?))
}

/// `GET /control/arbitrations` — the recorded belief arbitrations, oldest first.
pub(super) async fn arbitrations(
    State(state): State<AppState>,
) -> Result<Json<Vec<Arbitration>>, ApiError> {
    Ok(Json(state.server.control().arbitrations()?))
}

/// `GET /control/merge-proposals` — the cross-platform merge proposals still awaiting the operator, in
/// first-proposal order (the operator's backstop for merges the evidence did not yet justify).
pub(super) async fn merge_proposals(
    State(state): State<AppState>,
) -> Result<Json<Vec<MergeProposal>>, ApiError> {
    Ok(Json(state.server.control().merge_proposals()?))
}

/// `GET /control/interactions` — the recorded model interactions, oldest first (the deliberation
/// surface: per-call request, reasoning, token usage, and latency).
pub(super) async fn interactions(
    State(state): State<AppState>,
) -> Result<Json<Vec<ModelCall>>, ApiError> {
    Ok(Json(state.server.control().model_calls()?))
}

/// A `?from=` query — the lowest `seq` to return, defaulting to `0` (the whole log). The live
/// console seeds its replica with `from=0`, then polls `from=<head + 1>` for the new tail.
#[derive(Deserialize)]
pub(super) struct FromQuery {
    #[serde(default)]
    from: u64,
}

/// `GET /control/events?from=` — the event log from `from` onward, in order (the whole log when
/// `from` is omitted). The eval package embeds the full log per run; the live console seeds its
/// replica with one `from=0` read, then polls the tail with `from=<head + 1>` (spec §Observability →
/// live phase).
pub(super) async fn events(
    State(state): State<AppState>,
    Query(query): Query<FromQuery>,
) -> Result<Json<Vec<Event>>, ApiError> {
    Ok(Json(state.server.control().events_from(Seq(query.from))?))
}

/// `POST /control/snapshot` — write a graph snapshot now (the operator's take-one-before-an-experiment
/// trigger). `409` when snapshotting is disabled. The response names the file written, or reports that
/// the graph was already snapshotted at its current head.
pub(super) async fn snapshot(
    State(state): State<AppState>,
) -> Result<Json<SnapshotResponse>, ApiError> {
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
pub(super) struct SnapshotResponse {
    snapshot: Option<String>,
}

/// `GET /control/settings` — the agent's current behavioral settings.
pub(super) async fn settings(State(state): State<AppState>) -> Result<Json<Settings>, ApiError> {
    Ok(Json(state.server.control().settings()?))
}

/// `PUT /control/settings` — replace the behavioral settings (logged as an operator `ConfigSet`).
pub(super) async fn set_settings(
    State(state): State<AppState>,
    Json(settings): Json<Settings>,
) -> Result<StatusCode, ApiError> {
    state.server.control().set_settings(settings)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /control/imprint` — one operator message of the imprint interview. Operator authority (the
/// only path that may write `self`); needs the model, so `503` if none is configured.
#[derive(Deserialize)]
pub(super) struct ImprintRequest {
    text: String,
}

pub(super) async fn imprint(
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

/// `POST /control/lua` — run an ad-hoc operator Lua block in a no-commit sandbox and return its
/// rendered result (or error). MCP is off unless `allow_mcp` is set; needs no model (a block only
/// embeds if it calls `memory.search`, which uses the embedder, not the chat model).
#[derive(Deserialize)]
pub(super) struct LuaRequest {
    script: String,
    #[serde(default)]
    allow_mcp: bool,
}

pub(super) async fn run_lua(
    State(state): State<AppState>,
    Json(request): Json<LuaRequest>,
) -> Result<Json<LuaConsoleOutcome>, ApiError> {
    let outcome = state
        .server
        .control()
        .run_lua(&request.script, request.allow_mcp)
        .await?;
    Ok(Json(outcome))
}

/// `GET /control/lua-api` — the structured Lua API catalogue the console renders as a reference guide.
pub(super) async fn lua_api(
    State(state): State<AppState>,
) -> Result<Json<Vec<ApiEntry>>, ApiError> {
    Ok(Json(state.server.control().lua_api()))
}

/// `POST /control/prompt` — register a new version of a prompt template (the operator edit path); the
/// body replaces the template from the next read on, logged as an operator `PromptTemplateRegistered`.
#[derive(Deserialize)]
pub(super) struct PromptRequest {
    name: PromptTemplateName,
    body: String,
}

pub(super) async fn register_prompt(
    State(state): State<AppState>,
    Json(request): Json<PromptRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .server
        .control()
        .register_prompt(request.name, &request.body)?;
    Ok(StatusCode::NO_CONTENT)
}
