//! The participant surface (`/platform/*`): delivering turns, noting mid-session joins, and
//! resyncing a room's roster (spec §Clients → platform clients). It carries the platform identity in
//! the payload — the locator's
//! platform, the sender, the present set — never operator authority. The auth layer is applied to the
//! whole surface in [`super::router`].

use axum::{Json, extract::State, http::StatusCode};
use serde::Deserialize;
use zuihitsu::{ConversationLocator, RosterResync, TurnOutcome};

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
