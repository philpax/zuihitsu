//! The participant surface (`/platform/*`): delivering turns, noting mid-session joins, resyncing a
//! room's roster, and writing context or a participant's identity (spec §Clients → platform clients).
//! It carries no operator authority, and — crucially — no platform in the payload: every request is
//! scoped to exactly one connector by its key (a loopback request to the `direct` interface), so a
//! sender, a present set, a locator's scope path are all resolved under *that* connector's platform,
//! and its writes are attributed to *that* connector. A connector cannot name another's platform. The
//! auth-and-scope layer is applied to the whole surface in [`crate::http_server::router`].

use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
};
use serde::{Deserialize, Serialize};
use zuihitsu::{
    ContextEntry, ConversationLocator, LinkError, LinkNode, MemoryId, MessageInput,
    ParticipantAttribute, PersonId, ProjectOutcome, RosterResync,
};
use zuihitsu_platform_connector_types::{PlatformResponse, StreamFrame};

use crate::http_server::{AppState, auth::PlatformConnectorScope, error::ApiError};

/// The locator for `scope_path` under the request's connector — the platform is the scope's, never the
/// body's.
fn locator(scope: &PlatformConnectorScope, scope_path: String) -> ConversationLocator {
    ConversationLocator::new(scope.platform.clone(), scope_path)
}

/// The participant identity for a bare id under the request's connector.
fn person(scope: &PlatformConnectorScope, id: String) -> PersonId {
    PersonId::new(scope.platform.clone(), id)
}

/// One inbound message on the wire: the sender's bare id (the platform is the request's scope) and its
/// text.
#[derive(Deserialize)]
pub(super) struct WireMessage {
    sender: String,
    text: String,
}

/// `POST /platform/messages` — deliver a batch of participant turns and run one agent response
/// cycle. Each message is recorded as a separate participant turn; the agent sees them all and
/// responds once. Senders and the present set are bare ids resolved under the request's connector;
/// needs the model, so `503` if none is configured.
#[derive(Deserialize)]
pub(super) struct MessageRequest {
    scope_path: String,
    messages: Vec<WireMessage>,
    present: Vec<String>,
}

pub(super) async fn message(
    State(state): State<AppState>,
    Extension(scope): Extension<PlatformConnectorScope>,
    Json(request): Json<MessageRequest>,
) -> Result<Json<PlatformResponse>, ApiError> {
    let model = state.model.as_ref().ok_or(ApiError::NoModel)?;
    let locator = locator(&scope, request.scope_path);
    let messages: Vec<MessageInput> = request
        .messages
        .into_iter()
        .map(|message| MessageInput {
            sender: person(&scope, message.sender),
            text: message.text,
        })
        .collect();
    let present: Vec<PersonId> = request
        .present
        .into_iter()
        .map(|id| person(&scope, id))
        .collect();
    let response = state
        .server
        .platform()
        .route_messages(model.as_ref(), &locator, &messages, &present)
        .await?;
    Ok(Json(response))
}

/// `POST /platform/join` — note a participant arriving mid-session. The model, when configured,
/// feeds the joiner's describe catch-up before the join-brief composes; without one the join still
/// succeeds off the current prose rather than returning a 503.
#[derive(Deserialize)]
pub(super) struct JoinRequest {
    scope_path: String,
    participant: String,
}

pub(super) async fn join(
    State(state): State<AppState>,
    Extension(scope): Extension<PlatformConnectorScope>,
    Json(request): Json<JoinRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .server
        .platform()
        .note_join(
            state.model.as_deref(),
            &locator(&scope, request.scope_path),
            &person(&scope, request.participant),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /platform/roster` — resync a room's full roster, for a connector that observes presence
/// directly rather than only through messages. Diffs the reported roster against the live session:
/// arrivals get the same join-brief as `/platform/join`, departures are acknowledged but eventless.
/// The response reports the arrivals it briefed and the count of prior members no longer present.
/// The model, when configured, feeds each arrival's describe catch-up before its brief composes;
/// without one the resync still succeeds off the current prose rather than returning a 503.
#[derive(Deserialize)]
pub(super) struct RosterRequest {
    scope_path: String,
    roster: Vec<String>,
}

/// The roster resync on the wire: the bare ids briefed in (the platform is the request's scope) and the
/// count of prior members no longer present.
#[derive(Serialize)]
pub(super) struct RosterResyncBody {
    joined: Vec<String>,
    departed: usize,
}

pub(super) async fn roster(
    State(state): State<AppState>,
    Extension(scope): Extension<PlatformConnectorScope>,
    Json(request): Json<RosterRequest>,
) -> Result<Json<RosterResyncBody>, ApiError> {
    let roster: Vec<PersonId> = request
        .roster
        .into_iter()
        .map(|id| person(&scope, id))
        .collect();
    let RosterResync { joined, departed } = state
        .server
        .platform()
        .note_presence(
            state.model.as_deref(),
            &locator(&scope, request.scope_path),
            &roster,
        )
        .await?;
    Ok(Json(RosterResyncBody {
        joined: joined
            .into_iter()
            .map(|person| person.id.to_string())
            .collect(),
        departed,
    }))
}

/// `POST /platform/context` — write context entries to a conversation's context memory directly.
/// A connector (e.g. the Discord bot) uses this to write channel metadata and laconic guidance on
/// first contact, posting structured data rather than interpolating untrusted strings into code. The
/// write is attributed in the event log to the request's connector, not the agent.
#[derive(Deserialize)]
pub(super) struct ContextRequest {
    scope_path: String,
    entries: Vec<ContextEntry>,
}

pub(super) async fn write_context(
    State(state): State<AppState>,
    Extension(scope): Extension<PlatformConnectorScope>,
    Json(request): Json<ContextRequest>,
) -> Result<StatusCode, ApiError> {
    state.server.platform().write_context(
        &locator(&scope, request.scope_path),
        &scope.platform,
        &request.entries,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /platform/project` — project platform attributes onto a scoped memory as public entries: a
/// participant's identity (username, display name, nickname) onto their `person/*` stub, or a guild's
/// name onto its `context/*` memory. Each attribute records a new value or clears one, superseding or
/// retracting the entry a prior projection returned. The write is attributed to the request's connector;
/// the response is the memory id the projection landed on and the new entry id per attribute, in order.
#[derive(Deserialize)]
pub(super) struct ProjectRequest {
    target: WireLinkNode,
    attributes: Vec<ParticipantAttribute>,
}

pub(super) async fn project(
    State(state): State<AppState>,
    Extension(scope): Extension<PlatformConnectorScope>,
    Json(request): Json<ProjectRequest>,
) -> Result<Json<ProjectOutcome>, ApiError> {
    let outcome = state.server.platform().project(
        &link_node(&scope, request.target),
        &scope.platform,
        &request.attributes,
    )?;
    Ok(Json(outcome))
}

/// The response to `GET /platform/self`: the id of the agent's own reserved `self` memory.
#[derive(Serialize)]
pub(super) struct SelfBody {
    memory_id: MemoryId,
}

/// `GET /platform/self` — the id of the agent's own reserved `self` memory. A connector uses it to
/// splice a `[mem:<id>]` reference when the agent itself is @mentioned, the way a mentioned
/// participant's projection returns their memory id.
pub(super) async fn self_memory(State(state): State<AppState>) -> Result<Json<SelfBody>, ApiError> {
    let memory_id = state.server.platform().self_memory()?;
    Ok(Json(SelfBody { memory_id }))
}

/// One endpoint of a link on the wire — a bare participant id or a bare scope path, each resolved
/// under the request's connector, never naming a platform of its own.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WireLinkNode {
    Participant { id: String },
    Context { scope_path: String },
}

/// `POST /platform/link` — assert or retract a structural link between two of the connector's own
/// scoped memories (a channel or a member `part_of` a guild, say). Both endpoints are resolved under
/// the request's connector, so a connector can only link memories it owns; `remove` retracts instead of
/// asserting. The write is attributed to the request's connector.
#[derive(Deserialize)]
pub(super) struct LinkRequest {
    from: WireLinkNode,
    to: WireLinkNode,
    relation: String,
    #[serde(default)]
    remove: bool,
}

pub(super) async fn link(
    State(state): State<AppState>,
    Extension(scope): Extension<PlatformConnectorScope>,
    Json(request): Json<LinkRequest>,
) -> Result<StatusCode, ApiError> {
    let from = link_node(&scope, request.from);
    let to = link_node(&scope, request.to);
    state
        .server
        .platform()
        .link(
            &from,
            &to,
            &request.relation,
            &scope.platform,
            request.remove,
        )
        .map_err(link_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Scope a wire link endpoint to the request's connector — the platform is the scope's, never the
/// body's, mirroring [`person`] and [`locator`].
fn link_node(scope: &PlatformConnectorScope, node: WireLinkNode) -> LinkNode {
    match node {
        WireLinkNode::Participant { id } => LinkNode::Participant(person(scope, id)),
        WireLinkNode::Context { scope_path } => LinkNode::Context(locator(scope, scope_path)),
    }
}

/// A connector-contract violation (an unregistered relation, or an attempt at `same_as`) is a `400`
/// for the connector to fix; an underlying store or graph failure is a `500`.
fn link_error(error: LinkError) -> ApiError {
    match error {
        LinkError::SameAsForbidden | LinkError::UnknownRelation(_) => {
            ApiError::BadRequest(error.to_string())
        }
        LinkError::Instance(error) => ApiError::Server(error),
    }
}

/// `POST /platform/messages/stream` — deliver a batch of turns and watch its generation arrive: the
/// reply (and reasoning) tokens as `progress` frames while the agent deliberates, then the whole
/// `PlatformResponse` as the terminal `outcome` frame — the same response the unary endpoint
/// returns, so a connector that ignores every `progress` frame behaves identically to one that
/// never upgraded. A connector uses this to drive a typing indicator or a partial-message edit; the
/// frames are ephemeral (never stored), and a turn's failure arrives as a terminal `error` frame
/// with the failure's message.
///
/// When a newer inbound batch supersedes this request's turn, the stream terminates promptly with a
/// normal `outcome` frame carrying `TurnOutcome::Superseded` — well before the successor completes,
/// since the successor's turn answers with everything in context through its own request.
///
/// The response is an SSE stream. Every event has a `data:` payload that is a JSON `StreamFrame`
/// (see `zuihitsu_frontend_types::StreamFrame`). No `event:` field is emitted — the frame's type
/// is inside the JSON. A consumer reads SSE events, takes each `data:` field, and deserialises
/// it as a `StreamFrame`.
pub(super) async fn message_stream(
    State(state): State<AppState>,
    Extension(scope): Extension<PlatformConnectorScope>,
    Json(request): Json<MessageRequest>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    let model = state.model.clone().ok_or(ApiError::NoModel)?;
    let locator = locator(&scope, request.scope_path);
    let messages: Vec<MessageInput> = request
        .messages
        .into_iter()
        .map(|message| MessageInput {
            sender: person(&scope, message.sender),
            text: message.text,
        })
        .collect();
    let present: Vec<PersonId> = request
        .present
        .into_iter()
        .map(|id| person(&scope, id))
        .collect();
    // Resolve (or mint) the conversation up front so the progress subscription can filter this
    // room's frames from the shared feed; route_messages resolves to the same id.
    let conversation = state.server.platform().ensure_conversation(&locator)?;
    let mut progress = state.server.subscribe_progress();

    let server = state.server.clone();
    let mut turn = tokio::spawn(async move {
        server
            .platform()
            .route_messages(model.as_ref(), &locator, &messages, &present)
            .await
    });

    let body = async_stream::stream! {
        let mut progress_open = true;
        loop {
            tokio::select! {
                progress_frame = progress.recv(), if progress_open => match progress_frame {
                    Ok(progress_frame) if progress_frame.conversation == conversation => {
                        yield frame(StreamFrame::Progress(progress_frame));
                    }
                    Ok(_) => continue,
                    // Progress is cosmetic: a lag skips ahead. A closed feed would otherwise
                    // resolve instantly forever, so its arm is disabled — the turn's completion
                    // still ends the stream.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        progress_open = false;
                        continue;
                    }
                },
                result = &mut turn => {
                    // The turn is done; drain the frames it published before we observed
                    // completion, so the tail of the reply is not dropped.
                    while let Ok(progress_frame) = progress.try_recv() {
                        if progress_frame.conversation == conversation {
                            yield frame(StreamFrame::Progress(progress_frame));
                        }
                    }
                    match result {
                        Ok(Ok(response)) => {
                            yield frame(StreamFrame::Outcome(response));
                        }
                        Ok(Err(error)) => {
                            yield frame(StreamFrame::Error {
                                message: error.to_string(),
                            });
                        }
                        Err(join_error) => {
                            yield frame(StreamFrame::Error {
                                message: join_error.to_string(),
                            });
                        }
                    }
                    return;
                }
            }
        }
    };
    Ok(Sse::new(body).keep_alive(KeepAlive::default()))
}

/// Wrap a `StreamFrame` as an SSE event with a JSON `data:` payload. No `event:` field is
/// emitted — the frame's type is inside the JSON (`{"type":"progress",…}`), so the SSE event
/// name carries no information.
fn frame(frame: StreamFrame) -> Result<SseEvent, axum::Error> {
    SseEvent::default().json_data(frame)
}
