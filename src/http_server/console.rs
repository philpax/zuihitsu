//! The embedded web console: asset resolution and the Ctrl-C shutdown signal.

use std::path::Path;

use axum::{
    http::{Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

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

/// Resolve on the next Ctrl-C. Driving both the HTTP server and the scheduler driver off independent
/// instances of this means a single interrupt stops both.
pub(crate) async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
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
