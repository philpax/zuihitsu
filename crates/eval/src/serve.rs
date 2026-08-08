//! The `--serve` live endpoint: an SSE stream of the eval as it runs, for the console to fold into the
//! live view. One `GET /eval/stream` per viewer — a `snapshot` of the current package as a lean
//! [`PackageSummary`] (verdicts, metrics, and per-call usage, without the event logs), then the `live`
//! deltas. The console sets its state from the snapshot and patches it with each delta: `RunSummarized`
//! is authoritative for the scoreboard (it carries the run's summary and the recomputed aggregate),
//! while the live `RunEvent`s animate the in-flight run's deep-dive. The one open run's full event log
//! — the bulk a snapshot deliberately omits — is fetched on demand over `GET /eval/run/{scenario}/{run}`,
//! which returns that run's whole [`RunRecord`].

use std::{convert::Infallible, net::SocketAddr, sync::Arc};

use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, Uri, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use tokio::sync::broadcast::error::RecvError;

use crate::{error::EvalError, live::EvalSink};

/// Serve the live eval at `addr` until the task is dropped. One route, `GET /eval/stream`; the run
/// drives concurrently, emitting into the shared `sink`.
pub async fn serve(addr: SocketAddr, sink: Arc<EvalSink>) -> Result<(), EvalError> {
    let app = Router::new()
        .route("/eval/stream", get(stream))
        // The deep-dive fetches one run's full event log here — the bulk the lean snapshot omits.
        .route("/eval/run/{scenario}/{run}", get(run_record))
        // Everything else is the embedded console, served in eval mode, so opening the address shows
        // the live viewer; the eval routes are matched first, before the console fallback.
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

/// Serve the web console, the shared bundle the agent also embeds (built by `zuihitsu-console`'s
/// build script into `crates/console/dist-embedded`), in `eval` mode so opening the serve address
/// lands on the live eval viewer. Any path without a matching asset falls back to `index.html` for
/// the single-page app to route. `async` required for axum 0.8's `Handler` bound.
async fn console(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match zuihitsu_console::asset_or_index(path) {
        Some(file) => {
            let (mime, bytes) =
                zuihitsu_console::render_embedded(file, zuihitsu_frontend_types::AppMode::Eval);
            ([(header::CONTENT_TYPE, mime)], bytes).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            "the web console is not built into this binary\n",
        )
            .into_response(),
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
    (
        allow_cross_origin(),
        Sse::new(body).keep_alive(KeepAlive::default()),
    )
}

/// One run's full [`RunRecord`] as JSON — the event log the lean snapshot omits, fetched when a
/// deep-dive opens the run. A `404` with a short plain message when the scenario index or run index is
/// absent (a stale link, or a run not yet complete). Cross-origin like the stream: a read-only read of
/// a local eval has nothing to guard.
async fn run_record(
    State(sink): State<Arc<EvalSink>>,
    Path((scenario, run)): Path<(u32, u32)>,
) -> Response {
    let Some(record) = sink.run_record(scenario, run) else {
        return (StatusCode::NOT_FOUND, allow_cross_origin(), "no such run\n").into_response();
    };
    match serde_json::to_string(&record) {
        Ok(json) => (
            allow_cross_origin(),
            [(header::CONTENT_TYPE, "application/json")],
            json,
        )
            .into_response(),
        Err(source) => {
            tracing::warn!(%source, scenario, run, "could not serialize a run record");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                allow_cross_origin(),
                "could not serialize the run record\n",
            )
                .into_response()
        }
    }
}

/// The permissive CORS header the live endpoints share: the console is served from its own origin (the
/// Vite dev server, or a static host), so the live reads are cross-origin; allow them. A read-only read
/// of a local eval has nothing to guard.
fn allow_cross_origin() -> [(header::HeaderName, HeaderValue); 1] {
    [(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    )]
}
