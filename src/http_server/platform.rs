//! The participant surface (`/platform/*`): delivering turns, noting mid-session joins, and
//! resyncing a room's roster (spec §Clients → platform clients). It carries the platform identity in
//! the payload — the locator's
//! platform, the sender, the present set — never operator authority. The auth layer is applied to the
//! whole surface in [`super::router`].

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
};
use serde::Deserialize;
use zuihitsu::{ContextEntry, ConversationLocator, PlatformResponse, RosterResync};

use super::{AppState, error::ApiError};

/// `POST /platform/message` — deliver a participant turn and run the agent's response cycle. Carries
/// the platform identity in the payload (the locator's platform, the sender, the present set); needs
/// the model, so `503` if none is configured.
#[derive(Deserialize)]
pub(super) struct MessageRequest {
    locator: ConversationLocator,
    sender: String,
    text: String,
    present: Vec<String>,
}

pub(super) async fn message(
    State(state): State<AppState>,
    Json(request): Json<MessageRequest>,
) -> Result<Json<PlatformResponse>, ApiError> {
    let model = state.model.as_ref().ok_or(ApiError::NoModel)?;
    let present: Vec<&str> = request.present.iter().map(String::as_str).collect();
    let response = state
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
    Ok(Json(response))
}

/// `POST /platform/join` — note a participant arriving mid-session. The model, when configured,
/// feeds the joiner's describe catch-up before the join-brief composes; without one the join still
/// succeeds off the current prose rather than returning a 503.
#[derive(Deserialize)]
pub(super) struct JoinRequest {
    locator: ConversationLocator,
    participant: String,
}

pub(super) async fn join(
    State(state): State<AppState>,
    Json(request): Json<JoinRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .server
        .platform()
        .note_join(
            state.model.as_deref(),
            &request.locator,
            &request.participant,
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
    locator: ConversationLocator,
    roster: Vec<String>,
}

pub(super) async fn roster(
    State(state): State<AppState>,
    Json(request): Json<RosterRequest>,
) -> Result<Json<RosterResync>, ApiError> {
    let roster: Vec<&str> = request.roster.iter().map(String::as_str).collect();
    let resync = state
        .server
        .platform()
        .note_presence(state.model.as_deref(), &request.locator, &roster)
        .await?;
    Ok(Json(resync))
}

/// `POST /platform/context` — write context entries to a conversation's context memory directly.
/// A connector (e.g. the Discord bot) uses this to write channel metadata and laconic guidance on
/// first contact, posting structured data rather than interpolating untrusted strings into code.
#[derive(Deserialize)]
pub(super) struct ContextRequest {
    locator: ConversationLocator,
    entries: Vec<ContextEntry>,
}

pub(super) async fn write_context(
    State(state): State<AppState>,
    Json(request): Json<ContextRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .server
        .platform()
        .write_context(&request.locator, &request.entries)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /platform/message/stream` — deliver a turn and watch its generation arrive: the reply (and
/// reasoning) tokens as `progress` frames while the agent deliberates, then the whole
/// `PlatformResponse` as the terminal `outcome` frame — the same response the unary endpoint
/// returns, so a connector that ignores every `progress` frame behaves identically to one that
/// never upgraded. A connector uses this to drive a typing indicator or a partial-message edit; the
/// frames are ephemeral (never stored), and a turn's failure arrives as a terminal `error` frame
/// with the failure's message.
pub(super) async fn message_stream(
    State(state): State<AppState>,
    Json(request): Json<MessageRequest>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    let model = state.model.clone().ok_or(ApiError::NoModel)?;
    // Resolve (or mint) the conversation up front so the progress subscription can filter this
    // room's frames from the shared feed; route_message resolves to the same id.
    let conversation = state
        .server
        .platform()
        .ensure_conversation(&request.locator)?;
    let mut progress = state.server.subscribe_progress();

    let server = state.server.clone();
    let mut turn = tokio::spawn(async move {
        let present: Vec<&str> = request.present.iter().map(String::as_str).collect();
        server
            .platform()
            .route_message(
                model.as_ref(),
                &request.locator,
                &request.sender,
                &request.text,
                &present,
            )
            .await
    });

    let body = async_stream::stream! {
        let mut progress_open = true;
        loop {
            tokio::select! {
                frame = progress.recv(), if progress_open => match frame {
                    Ok(frame) if frame.conversation == conversation => {
                        if let Ok(json) = serde_json::to_string(&frame) {
                            yield Ok::<_, std::convert::Infallible>(
                                SseEvent::default().event("progress").data(json),
                            );
                        }
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
                    while let Ok(frame) = progress.try_recv() {
                        if frame.conversation == conversation
                            && let Ok(json) = serde_json::to_string(&frame)
                        {
                            yield Ok(SseEvent::default().event("progress").data(json));
                        }
                    }
                    match result {
                        Ok(Ok(response)) => {
                            if let Ok(json) = serde_json::to_string(&response) {
                                yield Ok(SseEvent::default().event("outcome").data(json));
                            }
                        }
                        Ok(Err(error)) => {
                            yield Ok(SseEvent::default().event("error").data(error.to_string()));
                        }
                        Err(join_error) => {
                            yield Ok(SseEvent::default().event("error").data(join_error.to_string()));
                        }
                    }
                    return;
                }
            }
        }
    };
    Ok(Sse::new(body).keep_alive(KeepAlive::default()))
}
