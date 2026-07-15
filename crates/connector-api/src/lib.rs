//! The shared platform API client for zuihitsu connectors.
//!
//! Owns the HTTP transport, SSE parsing, and request/response body types for the `/platform/*`
//! endpoints. A connector wraps this crate with platform-specific logic — addressing, pacing,
//! presence — and delegates all communication with the zuihitsu server to [`PlatformClient`].
//!
//! Auth uses the platform key for all `/platform/*` endpoints. Every error's `Display` leads
//! with a `platform client:` context prefix, so a chained error from a connector reads as
//! nested context.

pub use zuihitsu_connector_types::{PlatformResponse, StreamFrame, TurnOutcome};

use std::fmt;

use futures_util::StreamExt;
use reqwest::{Client as HttpClient, StatusCode};
use serde::Serialize;
use zuihitsu_core::{ids::ConversationLocator, progress::TurnProgress};

/// A failure in the platform API client.
#[derive(Debug)]
pub enum Error {
    /// An HTTP transport error during an API call — the request failed to send or the response
    /// body failed to read.
    Http {
        operation: Operation,
        source: reqwest::Error,
    },
    /// The server returned a non-success status.
    Status {
        operation: Operation,
        status: StatusCode,
        body: String,
    },
}

/// The platform API operation that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// `POST /platform/messages/stream` — delivering a batch of turns.
    SendMessageStream,
    /// `POST /platform/join` — noting a participant arrival.
    Join,
    /// `POST /platform/context` — writing context entries.
    WriteContext,
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operation::SendMessageStream => write!(f, "send message stream"),
            Operation::Join => write!(f, "join"),
            Operation::WriteContext => write!(f, "write context"),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Http { operation, source } => {
                write!(f, "platform client: {operation}: {source}")
            }
            Error::Status {
                operation,
                status,
                body,
            } => {
                write!(f, "platform client: {operation} returned {status}: {body}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Http { source, .. } => Some(source),
            Error::Status { .. } => None,
        }
    }
}

/// A type alias for results that carry the platform client's error.
pub type Result<T> = std::result::Result<T, Error>;

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

    /// `POST /platform/messages/stream` — deliver a batch of turns and watch its generation arrive.
    /// Calls `on_progress` for each progress fragment as it arrives (so the caller can start a
    /// typing indicator on the first `Reply` fragment), and returns the terminal outcome or error.
    ///
    /// The response body is a newline-delimited JSON stream of `StreamFrame` values. Each line is
    /// one complete JSON object; the client reads lines and deserialises each as a `StreamFrame`.
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
            .map_err(|e| Error::Http {
                operation: Operation::SendMessageStream,
                source: e,
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Status {
                operation: Operation::SendMessageStream,
                status,
                body,
            });
        }

        // Parse the SSE stream. Every event has a `data:` payload that is a JSON `StreamFrame`.
        // No `event:` field is emitted — the frame's type is inside the JSON. The SSE grammar
        // handles framing; the client deserialises each `data:` payload and matches on the tag.
        let mut on_progress = on_progress;
        let mut data = String::new();
        let mut outcome = None;

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| Error::Http {
                operation: Operation::SendMessageStream,
                source: e,
            })?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(line_end) = buffer.find('\n') {
                let line: String = buffer[..line_end].trim_end_matches('\r').to_owned();
                buffer = buffer[line_end + 1..].to_owned();

                if let Some(payload) = line.strip_prefix("data: ") {
                    data = payload.to_owned();
                } else if line.is_empty() && !data.is_empty() {
                    // End of one SSE event — deserialise the data payload as a StreamFrame.
                    match serde_json::from_str::<StreamFrame>(&data) {
                        Ok(StreamFrame::Progress(progress)) => on_progress(&progress),
                        Ok(StreamFrame::Outcome(response)) => {
                            outcome = Some(StreamOutcome::Outcome(response));
                        }
                        Ok(StreamFrame::Error { message }) => {
                            outcome = Some(StreamOutcome::Error(message));
                        }
                        Ok(StreamFrame::Event(_) | StreamFrame::End) => {}
                        Err(error) => {
                            tracing::warn!(
                                %error,
                                "platform client: could not parse stream frame"
                            );
                        }
                    }
                    data.clear();
                }
            }
        }

        Ok(outcome.unwrap_or_else(|| {
            StreamOutcome::Error("the stream ended without an outcome".to_owned())
        }))
    }

    /// `POST /platform/join` — note a participant arriving mid-session.
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
            .map_err(|e| Error::Http {
                operation: Operation::Join,
                source: e,
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Status {
                operation: Operation::Join,
                status,
                body,
            });
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
            .map_err(|e| Error::Http {
                operation: Operation::WriteContext,
                source: e,
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Status {
                operation: Operation::WriteContext,
                status,
                body,
            });
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
