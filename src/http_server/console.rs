//! The embedded web console: asset resolution and the process's single Ctrl-C shutdown signal, fanned
//! out to every path that must stop on interrupt.

use std::path::Path;

use axum::{
    http::{Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;
use tokio::sync::watch;

use crate::http_server::serve_error::ServeError;

/// The web console, built into the binary at compile time (see `build.rs` and `rust_embed`). The
/// embedded build lands in its own `dist-embedded` dir, so a plain `npm run build` for the dev checks
/// never swaps in the standalone (non-embedded) bytes under us.
#[derive(RustEmbed)]
#[folder = "console/dist-embedded"]
pub(crate) struct Console;

/// Serve a console asset by path, falling back to `index.html` for client-side routes so a deep link
/// or a refresh lands in the app rather than on a 404. The HTML shell is served in `agent` mode, so
/// the one shared bundle boots into the agent's live view; the same bundle supports other host modes
/// selected at serve time (see `console`'s `App`).
pub(crate) async fn console(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Console::get(path).or_else(|| Console::get("index.html")) {
        Some(file) => console_asset(file, "agent"),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            "the web console is not built into this binary\n",
        )
            .into_response(),
    }
}

/// Serve an embedded console asset, injecting the app mode into the HTML shell (replacing the
/// `__ZUIHITSU_APP_MODE__` placeholder `index.html` ships with) so the single shared bundle knows
/// which view to boot. Non-HTML assets are served byte-for-byte.
fn console_asset(file: rust_embed::EmbeddedFile, mode: &str) -> Response {
    let mime = file.metadata.mimetype().to_owned();
    if mime.starts_with("text/html") {
        let html = String::from_utf8_lossy(&file.data).replace("__ZUIHITSU_APP_MODE__", mode);
        ([(header::CONTENT_TYPE, mime)], html).into_response()
    } else {
        ([(header::CONTENT_TYPE, mime)], file.data).into_response()
    }
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
