//! Error types for the Discord connector.
//!
//! Every error's `Display` leads with a context prefix naming the subsystem, so a chained error
//! reads as nested context. The bot must not crash on a single message failure: only config errors
//! and gateway connection failures are fatal.

use std::fmt;

/// A failure in the Discord connector, grouped by category.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    context: String,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

/// The category of a connector failure.
#[derive(Debug)]
#[allow(dead_code)]
pub enum ErrorKind {
    /// A configuration problem — missing required fields, unparseable TOML.
    Config,
    /// A Discord gateway or HTTP error.
    Discord,
    /// A zuihitsu platform API error.
    Platform,
    /// An I/O error.
    Io,
}

impl Error {
    pub fn config(context: impl Into<String>) -> Self {
        Error {
            kind: ErrorKind::Config,
            context: context.into(),
            source: None,
        }
    }

    #[allow(dead_code)]
    pub fn discord(
        context: impl Into<String>,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Error {
            kind: ErrorKind::Discord,
            context: context.into(),
            source: Some(source.into()),
        }
    }

    #[allow(dead_code)]
    pub fn platform(
        context: impl Into<String>,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Error {
            kind: ErrorKind::Platform,
            context: context.into(),
            source: Some(source.into()),
        }
    }

    #[allow(dead_code)]
    pub fn io(
        context: impl Into<String>,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Error {
            kind: ErrorKind::Io,
            context: context.into(),
            source: Some(source.into()),
        }
    }

    /// Whether this error is fatal — only config and gateway-connection failures are.
    #[allow(dead_code)]
    pub fn is_fatal(&self) -> bool {
        matches!(self.kind, ErrorKind::Config)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ErrorKind::Config => write!(f, "config: {}", self.context),
            ErrorKind::Discord => write!(f, "discord connector: {}", self.context),
            ErrorKind::Platform => write!(f, "platform client: {}", self.context),
            ErrorKind::Io => write!(f, "io: {}", self.context),
        }?;
        if let Some(source) = &self.source {
            write!(f, ": {source}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl From<zuihitsu_platform_connector_api::Error> for Error {
    fn from(error: zuihitsu_platform_connector_api::Error) -> Self {
        Error {
            kind: ErrorKind::Platform,
            context: error.to_string(),
            source: Some(Box::new(error)),
        }
    }
}

/// A type alias for results that carry the connector's error.
pub type Result<T> = std::result::Result<T, Error>;
