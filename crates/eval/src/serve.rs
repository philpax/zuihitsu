//! The `--serve` live endpoint: an SSE stream of the eval as it runs, for the console to fold into the
//! live view. One `GET /eval/stream` per viewer — a `snapshot` of the current package, then the `live`
//! deltas. The console sets its state from the snapshot and patches it with each delta: `RunCompleted`
//! is authoritative (it carries the run's record), while the live `RunEvent`s animate the in-flight
//! run's deep-dive — so a viewer who joins mid-run still converges on the canonical package.

use std::{convert::Infallible, net::SocketAddr, sync::Arc};

use axum::{
    Router,
    extract::State,
    http::{HeaderValue, StatusCode, Uri, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use rust_embed::RustEmbed;
use tokio::sync::broadcast::error::RecvError;

use crate::{error::EvalError, live::EvalSink};

/// Serve the live eval at `addr` until the task is dropped. One route, `GET /eval/stream`; the run
/// drives concurrently, emitting into the shared `sink`.
pub async fn serve(addr: SocketAddr, sink: Arc<EvalSink>) -> Result<(), EvalError> {
    let app = Router::new()
        .route("/eval/stream", get(stream))
        // Everything else is the embedded console, served in eval mode, so opening the address shows
        // the live viewer; `/eval/stream` is matched first, before the console fallback.
        .fallback(console)
        .with_state(sink);
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|source| {
        // Serving is best-effort and runs in a detached task, so a bind failure (typically the port
        // already held by a concurrent run) is not propagated to the run — warn here, or it vanishes.
        tracing::warn!(%addr, %source, "could not bind the live-serve address; running without it");
        EvalError::Serve(source)
    })?;
    tracing::info!("serving the live eval console at http://{addr}/");
    axum::serve(listener, app).await.map_err(EvalError::Serve)
}

/// The web console, the same bundle the agent embeds (built once into `console/dist-embedded`),
/// served here in `eval` mode so opening the serve address lands on the live eval viewer. Any path
/// without a matching asset falls back to `index.html` for the single-page app to route.
#[derive(RustEmbed)]
#[folder = "../../console/dist-embedded"]
struct Console;

/// Serve a console asset by path, falling back to `index.html` for client-side routes (a deep link or
/// a refresh), with the app mode injected into the HTML shell.
async fn console(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Console::get(path).or_else(|| Console::get("index.html")) {
        Some(file) => console_asset(file, "eval"),
        None => (
            StatusCode::NOT_FOUND,
            "the web console is not built into this binary\n",
        )
            .into_response(),
    }
}

/// Inject the app mode into the HTML shell (replacing the `__ZUIHITSU_APP_MODE__` placeholder
/// `index.html` ships with) so the shared bundle boots into the eval viewer; serve other assets as-is.
fn console_asset(file: rust_embed::EmbeddedFile, mode: &str) -> Response {
    let mime = file.metadata.mimetype().to_owned();
    if mime.starts_with("text/html") {
        let html = String::from_utf8_lossy(&file.data).replace("__ZUIHITSU_APP_MODE__", mode);
        ([(header::CONTENT_TYPE, mime)], html).into_response()
    } else {
        ([(header::CONTENT_TYPE, mime)], file.data).into_response()
    }
}

/// One viewer's stream: the current package as a `snapshot` event, then every later delta as a `live`
/// event (carrying the [`LiveEvent`] and its monotonic id). The snapshot-and-subscribe is atomic in the
/// sink, so no delta is missed or double-counted across the cut.
async fn stream(State(sink): State<Arc<EvalSink>>) -> impl IntoResponse {
    let (snapshot, catch_up, mut receiver) = sink.subscribe();
    let body = async_stream::stream! {
        if let Ok(json) = serde_json::to_string(&snapshot) {
            yield Ok::<_, Infallible>(Event::default().event("snapshot").data(json));
        }
        // Replay the in-flight runs' events so far (as live deltas) before the ongoing stream, so a
        // client joining mid-run folds the deliberation from its start, not from the moment it connected.
        for event in catch_up {
            if let Ok(json) = serde_json::to_string(&event) {
                yield Ok(Event::default().event("live").data(json));
            }
        }
        loop {
            match receiver.recv().await {
                Ok((id, event)) => {
                    if let Ok(json) = serde_json::to_string(&event) {
                        yield Ok(Event::default().event("live").id(id.to_string()).data(json));
                    }
                }
                // Fell behind the buffer: end the stream so the console reconnects and re-snapshots,
                // rather than resume from a gap. Rare — the buffer is large.
                Err(RecvError::Lagged(_)) => break,
                Err(RecvError::Closed) => break,
            }
        }
    };
    // The console is served from its own origin (the Vite dev server, or a static host), so the live
    // endpoint is cross-origin; allow it. A read-only event stream of a local eval has nothing to guard.
    let cors = [(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    )];
    (cors, Sse::new(body).keep_alive(KeepAlive::default()))
}
