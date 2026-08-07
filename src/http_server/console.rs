//! The agent's console fallback and the process's single Ctrl-C shutdown signal, fanned out to
//! every path that must stop on interrupt.
//!
//! The embedded web console itself lives in `zuihitsu-console` (its build script runs the full
//! frontend pipeline; its `serve_embedded` serves the bundle with the app mode injected). This
//! module holds only the thin fallback that calls into it when the `console` feature is on, or
//! serves the root-owned placeholder page (`placeholder.html`) when the optional dependency is not
//! compiled in.

use std::path::Path;

use axum::{http::Uri, response::Response};
#[cfg(not(feature = "console"))]
use axum::{http::header, response::IntoResponse};
use tokio::sync::watch;

use crate::http_server::serve_error::ServeError;

/// Serve the embedded console in `agent` mode: its assets by path, and any client-side route (no
/// matching asset) as `index.html` so the single-page app can route it. `async` is required: axum
/// 0.8's `Handler` is implemented for async functions, and this is the router's fallback.
#[cfg(feature = "console")]
pub(crate) async fn console(uri: Uri) -> Response {
    zuihitsu_console::serve_embedded(&uri, "agent")
}

/// Serve the placeholder page this build ships in place of the console. A `--no-default-features`
/// build skips the `zuihitsu-console` dependency (whose build script would run the whole frontend
/// pipeline), so everything here is root-owned: the page tells the operator how to get the real
/// console. `async` for the same axum 0.8 `Handler` bound as the feature-on sibling.
#[cfg(not(feature = "console"))]
pub(crate) async fn console(_uri: Uri) -> Response {
    (
        [(header::CONTENT_TYPE, "text/html")],
        include_str!("placeholder.html"),
    )
        .into_response()
}

/// The process's single shutdown source, fanned out to every path that must stop on Ctrl-C — the HTTP
/// server's graceful shutdown, each background driver, and the streaming handlers. [`install`] spawns
/// one interrupt listener that latches the flag; every consumer holds a clone and awaits [`wait`], so
/// there is a single source of shutdown truth rather than one interrupt registration per consumer. The
/// flag latches, so a consumer that only checks after the interrupt (a stream opened late) still sees
/// it.
///
/// [`install`]: ShutdownFlag::install
/// [`wait`]: ShutdownFlag::wait
#[derive(Clone)]
pub(crate) struct ShutdownFlag(watch::Receiver<bool>);

impl ShutdownFlag {
    /// Spawn the process's single Ctrl-C listener and return the flag it latches on the first
    /// interrupt. Call once, inside the runtime, before handing clones to the shutdown paths.
    pub(crate) fn install() -> ShutdownFlag {
        let (tx, rx) = watch::channel(false);
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            let _ = tx.send(true);
        });
        ShutdownFlag(rx)
    }

    /// Resolve once shutdown has been signalled (or the source is gone, so a late awaiter never blocks
    /// past shutdown). Consumes a clone, so each consumer takes its own future — clone the flag per
    /// await in a `select!` loop, or hand a fresh clone to each driver.
    pub(crate) async fn wait(mut self) {
        let _ = self.0.wait_for(|&stop| stop).await;
    }

    /// A flag that never fires, for a test that builds an [`AppState`] without a running server.
    #[cfg(test)]
    pub(crate) fn never() -> ShutdownFlag {
        let (tx, rx) = watch::channel(false);
        // Leak the sender so the flag stays pending rather than reading as an already-closed channel.
        std::mem::forget(tx);
        ShutdownFlag(rx)
    }

    /// A flag whose firing the caller controls, for a test that asserts a consumer stops when
    /// shutdown is signalled: send `true` on the returned sender to fire it.
    #[cfg(test)]
    pub(crate) fn controllable() -> (ShutdownFlag, watch::Sender<bool>) {
        let (tx, rx) = watch::channel(false);
        (ShutdownFlag(rx), tx)
    }
}

pub(crate) fn ensure_parent_dir(path: &Path) -> Result<(), ServeError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|source| ServeError::CreateDir {
            path: parent.to_owned(),
            source,
        })?;
    }
    Ok(())
}
