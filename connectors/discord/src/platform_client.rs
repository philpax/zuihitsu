//! Platform client: async HTTP client wrapping `reqwest` to talk to the zuihitsu platform API.
//!
//! Handles three SSE event types from `/platform/message/stream`: `progress` (streaming tokens),
//! `outcome` (terminal `PlatformResponse`), and `error` (turn failure). Auth uses the platform
//! key for all `/platform/*` endpoints.

use futures_util::StreamExt;
use reqwest::Client as HttpClient;
use serde::Serialize;
use zuihitsu_core::{ids::ConversationLocator, progress::TurnProgress};
use zuihitsu_frontend_types::PlatformResponse;

use crate::error::{Error, Result};

/// The terminal outcome of a streaming message request.
pub enum StreamOutcome {
    /// The turn completed with this response.
    Outcome(PlatformResponse),
    /// A turn failure — the error message from the server.
    Error(String),
}

/// One entry to write to a conversation's context memory via `POST /platform/context`.
#[derive(Serialize)]
pub struct ContextEntry {
    pub text: String,
}

/// One inbound message to submit to the platform API.
#[derive(Serialize)]
pub struct PlatformMessage {
    pub sender: String,
    pub text: String,
}

/// The async platform API client.
pub struct PlatformClient {
    http: HttpClient,
    base_url: String,
    platform_key: String,
}

impl PlatformClient {
    pub fn new(base_url: String, platform_key: String) -> Self {
        PlatformClient {
            http: HttpClient::new(),
            base_url,
            platform_key,
        }
    }

    /// `POST /platform/message/stream` — deliver a turn and watch its generation arrive.
    /// Calls `on_progress` for each progress fragment as it arrives (so the caller can start a
    /// typing indicator on the first `Reply` fragment), and returns the terminal outcome or error.
    pub async fn send_message_stream(
        &self,
        locator: &ConversationLocator,
        messages: &[PlatformMessage],
        present: &[&str],
        on_progress: impl FnMut(&TurnProgress),
    ) -> Result<StreamOutcome> {
        let body = MessageBody {
            locator,
            messages,
            present,
        };
        let url = format!("{}/platform/messages/stream", self.base_url);
        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.platform_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::platform("could not send streaming message", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(Error::platform(
                format!("streaming message returned {status}: {text}"),
                std::io::Error::other("platform error"),
            ));
        }

        // Parse the SSE stream as it arrives. Each line is either "event: <type>" or "data: <json>".
        let mut on_progress = on_progress;
        let mut event_type = String::new();
        let mut data = String::new();
        let mut outcome = None;

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| Error::platform("could not read stream chunk", e))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(line_end) = buffer.find('\n') {
                let line = buffer[..line_end].trim_end_matches('\r').to_owned();
                buffer = buffer[line_end + 1..].to_owned();

                if let Some(event) = line.strip_prefix("event: ") {
                    event_type = event.to_owned();
                } else if let Some(payload) = line.strip_prefix("data: ") {
                    data = payload.to_owned();
                } else if line.is_empty() && !event_type.is_empty() {
                    // End of one SSE event — process it.
                    match event_type.as_str() {
                        "progress" => match serde_json::from_str::<TurnProgress>(&data) {
                            Ok(progress) => on_progress(&progress),
                            Err(error) => {
                                tracing::warn!(
                                    %error,
                                    "platform client: could not parse progress frame"
                                );
                            }
                        },
                        "outcome" => match serde_json::from_str::<PlatformResponse>(&data) {
                            Ok(response) => {
                                outcome = Some(StreamOutcome::Outcome(response));
                            }
                            Err(error) => {
                                tracing::warn!(
                                    %error,
                                    "platform client: could not parse outcome frame"
                                );
                            }
                        },
                        "error" => {
                            outcome = Some(StreamOutcome::Error(data.clone()));
                        }
                        _ => {}
                    }
                    event_type.clear();
                    data.clear();
                }
            }
        }

        Ok(outcome.unwrap_or(StreamOutcome::Error(
            "the stream ended without an outcome".to_owned(),
        )))
    }

    /// `POST /platform/join` — note a participant arriving mid-session.
    #[allow(dead_code)]
    pub async fn join(&self, locator: &ConversationLocator, participant: &str) -> Result<()> {
        let body = JoinBody {
            locator,
            participant,
        };
        let url = format!("{}/platform/join", self.base_url);
        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.platform_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::platform("could not send join", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(Error::platform(
                format!("join returned {status}: {text}"),
                std::io::Error::other("platform error"),
            ));
        }
        Ok(())
    }

    /// `POST /platform/context` — write context entries to a conversation's context memory directly.
    /// A connector uses this to write channel metadata and laconic guidance on first contact. The
    /// `connector_id` identifies the caller in the event log.
    pub async fn write_context(
        &self,
        locator: &ConversationLocator,
        connector_id: &str,
        entries: &[ContextEntry],
    ) -> Result<()> {
        let body = ContextBody {
            locator,
            connector: connector_id,
            entries,
        };
        let url = format!("{}/platform/context", self.base_url);
        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.platform_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::platform("could not write context", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(Error::platform(
                format!("write context returned {status}: {text}"),
                std::io::Error::other("platform error"),
            ));
        }
        Ok(())
    }
}

/// The request body for `POST /platform/messages` and `/platform/messages/stream`.
#[derive(Serialize)]
struct MessageBody<'a> {
    locator: &'a ConversationLocator,
    messages: &'a [PlatformMessage],
    present: &'a [&'a str],
}

/// The request body for `POST /platform/join`.
#[allow(dead_code)]
#[derive(Serialize)]
struct JoinBody<'a> {
    locator: &'a ConversationLocator,
    participant: &'a str,
}

/// The request body for `POST /platform/context`.
#[derive(Serialize)]
struct ContextBody<'a> {
    locator: &'a ConversationLocator,
    connector: &'a str,
    entries: &'a [ContextEntry],
}
