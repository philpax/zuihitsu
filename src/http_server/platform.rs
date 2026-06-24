//! The participant surface (`/platform/*`): delivering turns and noting mid-session joins (spec
//! §Clients → platform clients). It carries the platform identity in the payload — the locator's
//! platform, the sender, the present set — never operator authority. The auth layer is applied to the
//! whole surface in [`super::router`].

use axum::{Json, extract::State, http::StatusCode};
use serde::Deserialize;
use zuihitsu::{ConversationLocator, TurnOutcome};

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

/// `POST /platform/join` — note a participant arriving mid-session (no model needed).
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
        .note_join(&request.locator, &request.participant)?;
    Ok(StatusCode::NO_CONTENT)
}
