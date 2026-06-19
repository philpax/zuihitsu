//! The `--serve` live endpoint: an SSE stream of the eval as it runs, for the console to fold into the
//! live view. One `GET /eval/stream` per viewer — a `snapshot` of the current package, then the `live`
//! deltas. The console sets its state from the snapshot and patches it with each delta: `RunCompleted`
//! is authoritative (it carries the run's record), while the live `RunEvent`s animate the in-flight
//! run's deep-dive — so a viewer who joins mid-run still converges on the canonical package.

use std::{convert::Infallible, net::SocketAddr, sync::Arc};

use axum::{
    Router,
    extract::State,
    http::{HeaderValue, header},
    response::{
        IntoResponse,
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
        .with_state(sink);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(EvalError::Serve)?;
    tracing::info!("serving the live eval at http://{addr}/eval/stream");
    axum::serve(listener, app).await.map_err(EvalError::Serve)
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
