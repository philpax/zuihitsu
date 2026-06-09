//! The CLI's HTTP client: the operator subcommands reach the running server's `/control` API over
//! loopback (spec §Clients → control clients). The CLI no longer opens the store itself — only the
//! server holds the single-writer log lock — so every command is a request to the running instance.

use std::net::SocketAddr;

use reqwest::{
    StatusCode,
    blocking::{Client as Http, RequestBuilder, Response},
};
use serde::{Deserialize, de::DeserializeOwned};
use zuihitsu::{
    Arbitration, EntryView, GenesisStatus, MemoryView, Rollout, SeedSelf, SessionView, Settings,
};

/// A blocking client for the operator/control API, bound to the instance the config selects.
pub struct Client {
    base: String,
    http: Http,
}

impl Client {
    pub fn new(bind: SocketAddr) -> Client {
        Client {
            base: format!("http://{bind}"),
            http: Http::new(),
        }
    }

    /// `POST /control/agent` — create the agent (or resume an interrupted genesis); idempotent.
    pub fn create_agent(&self, seed: &SeedSelf) -> Result<Rollout, ClientError> {
        self.json(self.http.post(self.url("/control/agent")).json(seed))
    }

    /// `GET /control/genesis` — whether an agent exists and is ready.
    pub fn genesis(&self) -> Result<GenesisStatus, ClientError> {
        self.json(self.http.get(self.url("/control/genesis")))
    }

    /// `GET /control/memory?name=` — a memory by name, or `None` if it does not exist.
    pub fn memory(&self, name: &str) -> Result<Option<MemoryView>, ClientError> {
        self.optional(self.get_query("/control/memory", &[("name", name)]))
    }

    /// `GET /control/memories?prefix=` — the live memories in a namespace.
    pub fn memories(&self, prefix: &str) -> Result<Vec<MemoryView>, ClientError> {
        self.json(self.get_query("/control/memories", &[("prefix", prefix)]))
    }

    /// `GET /control/entries?name=` — a memory's local content entries.
    pub fn entries(&self, name: &str) -> Result<Vec<EntryView>, ClientError> {
        self.json(self.get_query("/control/entries", &[("name", name)]))
    }

    /// `GET /control/sessions?platform=&scope=` — a conversation's sessions, oldest first.
    pub fn sessions(&self, platform: &str, scope: &str) -> Result<Vec<SessionView>, ClientError> {
        self.json(self.get_query(
            "/control/sessions",
            &[("platform", platform), ("scope", scope)],
        ))
    }

    /// `GET /control/recurring` — the memories carrying a recurring occurrence.
    pub fn recurring(&self) -> Result<Vec<MemoryView>, ClientError> {
        self.json(self.http.get(self.url("/control/recurring")))
    }

    /// `GET /control/arbitrations` — the recorded belief arbitrations, oldest first.
    pub fn arbitrations(&self) -> Result<Vec<Arbitration>, ClientError> {
        self.json(self.http.get(self.url("/control/arbitrations")))
    }

    /// `GET /control/settings` — the agent's current behavioral settings.
    pub fn settings(&self) -> Result<Settings, ClientError> {
        self.json(self.http.get(self.url("/control/settings")))
    }

    /// `PUT /control/settings` — replace the behavioral settings.
    pub fn set_settings(&self, settings: &Settings) -> Result<(), ClientError> {
        self.no_content(self.http.put(self.url("/control/settings")).json(settings))
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn get_query(&self, path: &str, query: &[(&str, &str)]) -> RequestBuilder {
        self.http.get(self.url(path)).query(query)
    }

    /// Send and decode a JSON response, mapping a non-2xx to the server's error message.
    fn json<T: DeserializeOwned>(&self, request: RequestBuilder) -> Result<T, ClientError> {
        self.checked(request)?.json().map_err(ClientError::Decode)
    }

    /// Like [`Client::json`], but a `404` is `None` rather than an error (a not-found lookup).
    fn optional<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<Option<T>, ClientError> {
        let response = request.send().map_err(ClientError::from_send)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        check_status(response)?
            .json()
            .map(Some)
            .map_err(ClientError::Decode)
    }

    /// Send a request whose success carries no body (a `204`).
    fn no_content(&self, request: RequestBuilder) -> Result<(), ClientError> {
        self.checked(request)?;
        Ok(())
    }

    fn checked(&self, request: RequestBuilder) -> Result<Response, ClientError> {
        check_status(request.send().map_err(ClientError::from_send)?)
    }
}

/// Pass a successful response through; turn a non-2xx into a [`ClientError::Status`], reading the
/// server's `{ "error": … }` body when present.
fn check_status(response: Response) -> Result<Response, ClientError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let message = response
        .json::<ErrorBody>()
        .map(|body| body.error)
        .unwrap_or_else(|_| status.to_string());
    Err(ClientError::Status {
        code: status,
        message,
    })
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

/// A failure talking to the server.
#[derive(Debug)]
pub enum ClientError {
    /// The connection was refused — the server is most likely not running.
    Connect,
    /// The request could not be sent (a transport error other than a refused connection).
    Send(reqwest::Error),
    /// The server returned an error status, carrying its message.
    Status { code: StatusCode, message: String },
    /// The response body could not be decoded.
    Decode(reqwest::Error),
}

impl ClientError {
    fn from_send(error: reqwest::Error) -> ClientError {
        if error.is_connect() {
            ClientError::Connect
        } else {
            ClientError::Send(error)
        }
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Connect => {
                write!(f, "could not reach the server — is `zuihitsu` running?")
            }
            ClientError::Send(error) => write!(f, "the request could not be sent: {error}"),
            ClientError::Status { code, message } => {
                write!(f, "the server returned {code}: {message}")
            }
            ClientError::Decode(error) => {
                write!(f, "could not decode the server's response: {error}")
            }
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClientError::Send(error) | ClientError::Decode(error) => Some(error),
            ClientError::Connect | ClientError::Status { .. } => None,
        }
    }
}
