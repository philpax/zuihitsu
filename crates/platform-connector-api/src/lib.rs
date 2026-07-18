//! The shared platform API client for zuihitsu connectors.
//!
//! Owns the HTTP transport, SSE parsing, and request/response body types for the `/platform/*`
//! endpoints. A connector wraps this crate with platform-specific logic — addressing, pacing,
//! presence — and delegates all communication with the zuihitsu server to [`PlatformClient`].
//!
//! Auth uses the platform key for all `/platform/*` endpoints. Every error's `Display` leads
//! with a `platform client:` context prefix, so a chained error from a connector reads as
//! nested context.

pub use zuihitsu_platform_connector_types::{PlatformResponse, StreamFrame, TurnOutcome};

use std::fmt;

use futures_util::StreamExt;
use reqwest::{Client as HttpClient, StatusCode};
use serde::Serialize;
use zuihitsu_core::{
    ids::{ConversationLocator, EntryId, PersonId},
    progress::TurnProgress,
};

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
    /// `POST /platform/project` — projecting attributes onto a scoped memory.
    Project,
    /// `POST /platform/link` — asserting or retracting a structural link.
    Link,
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operation::SendMessageStream => write!(f, "send message stream"),
            Operation::Join => write!(f, "join"),
            Operation::WriteContext => write!(f, "write context"),
            Operation::Project => write!(f, "project"),
            Operation::Link => write!(f, "link"),
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

/// One attribute to project onto a scoped memory via `POST /platform/project`. `text` is the value to
/// record now, or `None` to clear a value that is no longer set. `supersedes` is the entry id a prior
/// projection of this same attribute returned, which the server supersedes on a change or retracts on a
/// clear — the connector holds it, so the server needs no per-attribute keying.
#[derive(Serialize)]
pub struct ParticipantAttribute {
    pub text: Option<String>,
    pub supersedes: Option<EntryId>,
}

/// One inbound message to submit to the platform API.
#[derive(Serialize)]
pub struct PlatformMessage {
    pub sender: PersonId,
    pub text: String,
}

/// A scoped memory named on the wire — a participant or a context. It is the endpoint of a structural
/// link (`POST /platform/link`) and the target of an attribute projection (`POST /platform/project`).
/// Only the bare id or scope path rides the wire; the connector's platform is the request's scope, so
/// the server resolves it under it.
pub enum LinkEndpoint {
    Participant(PersonId),
    Context(ConversationLocator),
}

/// One scoped-memory reference on the wire — a bare participant id or a bare scope path, matching the
/// server's `WireLinkNode`. The connector's platform is the request's scope.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum WireLinkNode<'a> {
    Participant { id: &'a str },
    Context { scope_path: &'a str },
}

impl LinkEndpoint {
    fn wire(&self) -> WireLinkNode<'_> {
        match self {
            LinkEndpoint::Participant(person) => WireLinkNode::Participant {
                id: person.id.as_str(),
            },
            LinkEndpoint::Context(locator) => WireLinkNode::Context {
                scope_path: locator.scope_path.as_str(),
            },
        }
    }
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
    ///
    /// When a newer batch supersedes this request's turn, the stream terminates promptly with a
    /// normal `Outcome` frame carrying [`TurnOutcome::Superseded`] — the successor's turn answers
    /// with everything in context, so there is nothing to post for this request.
    pub async fn send_message_stream(
        &self,
        locator: &ConversationLocator,
        messages: &[PlatformMessage],
        present: &[PersonId],
        on_progress: impl FnMut(&TurnProgress),
    ) -> Result<StreamOutcome> {
        /// One inbound message on the wire — the sender's bare id (the platform is the request's connector
        /// scope, from the key) and its text.
        #[derive(Serialize)]
        struct WireMessage<'a> {
            sender: &'a str,
            text: &'a str,
        }

        /// The request body for `POST /platform/messages` and `/platform/messages/stream`. No platform: the
        /// key scopes the request to one connector's platform, so ids ride bare.
        #[derive(Serialize)]
        struct MessageBody<'a> {
            scope_path: &'a str,
            messages: Vec<WireMessage<'a>>,
            present: Vec<&'a str>,
        }

        let body = MessageBody {
            scope_path: locator.scope_path.as_str(),
            messages: messages
                .iter()
                .map(|message| WireMessage {
                    sender: message.sender.id.as_str(),
                    text: &message.text,
                })
                .collect(),
            present: present.iter().map(|person| person.id.as_str()).collect(),
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
    pub async fn join(&self, locator: &ConversationLocator, participant: &PersonId) -> Result<()> {
        /// The request body for `POST /platform/join`.
        #[derive(Serialize)]
        struct JoinBody<'a> {
            scope_path: &'a str,
            participant: &'a str,
        }

        let body = JoinBody {
            scope_path: locator.scope_path.as_str(),
            participant: participant.id.as_str(),
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
    /// A connector uses this to write channel metadata and laconic guidance on first contact. The write
    /// is attributed to the connector the request's key registers.
    pub async fn write_context(
        &self,
        locator: &ConversationLocator,
        entries: &[ContextEntry],
    ) -> Result<()> {
        /// The request body for `POST /platform/context`.
        #[derive(Serialize)]
        struct ContextBody<'a> {
            scope_path: &'a str,
            entries: &'a [ContextEntry],
        }

        let body = ContextBody {
            scope_path: locator.scope_path.as_str(),
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

    /// `POST /platform/project` — project attributes onto a scoped memory as public entries: a
    /// participant's identity (username, display name, nickname) onto their `person/*` stub, or a
    /// guild's name onto its `context/*` memory. Each attribute records a new value or clears one,
    /// superseding or retracting the entry a prior projection returned for it. Returns the new entry id
    /// per attribute, in request order — `Some` for a recorded value, `None` for a cleared one — which
    /// the connector holds to supersede on the next change.
    pub async fn project(
        &self,
        target: &LinkEndpoint,
        attributes: &[ParticipantAttribute],
    ) -> Result<Vec<Option<EntryId>>> {
        /// The request body for `POST /platform/project`.
        #[derive(Serialize)]
        struct ProjectBody<'a> {
            target: WireLinkNode<'a>,
            attributes: &'a [ParticipantAttribute],
        }

        let body = ProjectBody {
            target: target.wire(),
            attributes,
        };
        let url = format!("{}/platform/project", self.base_url);
        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.platform_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Http {
                operation: Operation::Project,
                source: e,
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Status {
                operation: Operation::Project,
                status,
                body,
            });
        }
        response
            .json::<Vec<Option<EntryId>>>()
            .await
            .map_err(|e| Error::Http {
                operation: Operation::Project,
                source: e,
            })
    }

    /// `POST /platform/link` — assert (or, with `remove`, retract) a structural link between two of the
    /// connector's own scoped memories: a channel or a member `part_of` a guild, say. Both endpoints
    /// ride the wire as bare ids resolved under the request's connector, so a connector can link only
    /// memories it owns. `same_as` is refused server-side: cross-platform identity is operator-confirmed.
    pub async fn link(
        &self,
        from: &LinkEndpoint,
        to: &LinkEndpoint,
        relation: &str,
        remove: bool,
    ) -> Result<()> {
        /// The request body for `POST /platform/link`.
        #[derive(Serialize)]
        struct LinkBody<'a> {
            from: WireLinkNode<'a>,
            to: WireLinkNode<'a>,
            relation: &'a str,
            remove: bool,
        }

        let body = LinkBody {
            from: from.wire(),
            to: to.wire(),
            relation,
            remove,
        };
        let url = format!("{}/platform/link", self.base_url);
        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.platform_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Http {
                operation: Operation::Link,
                source: e,
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Status {
                operation: Operation::Link,
                status,
                body,
            });
        }
        Ok(())
    }
}
