//! The control surface's push channel: `GET /control/events/stream`, a server-sent events
//! stream carrying every committed event as it lands plus the ephemeral turn-progress frames a
//! live viewer renders token by token (spec §Observability). The polling `GET /control/events`
//! remains for catch-up and for clients that never upgraded; this endpoint is the same data
//! pushed instead of polled — with one addition, the `progress` frames, which exist only here
//! because they are never stored.
//!
//! Every SSE event has a `data:` payload that is a JSON `StreamFrame` (see
//! `zuihitsu_frontend_types::StreamFrame`). No `event:` field is emitted — the frame's type is
//! inside the JSON. A consumer reads SSE events, takes each `data:` field, and deserialises it
//! as a `StreamFrame`.

use axum::{
    extract::{Query, State},
    response::{
        IntoResponse,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
};
use tokio::sync::broadcast;
use zuihitsu::{Event, ids::Seq};
use zuihitsu_platform_connector_types::StreamFrame;

use crate::http_server::{AppState, control::FromQuery, error::ApiError};

/// Wrap a `StreamFrame` as an SSE event with a JSON `data:` payload. No `event:` field is
/// emitted — the frame's type is inside the JSON (`{"type":"progress",…}`), so the SSE event
/// name carries no information.
fn frame(frame: StreamFrame) -> Result<SseEvent, axum::Error> {
    SseEvent::default().json_data(frame)
}

/// The live fan-out bridging the store's synchronous subscription onto an async broadcast the
/// stream handlers can select over. Built once at router construction: a dedicated thread owns
/// the store's `std::sync::mpsc` receiver and forwards each committed event; the thread ends
/// when the store drops its sender at shutdown. Lossy by design on the consumer side — a
/// receiver that lags reconnects and catches up through the snapshot, exactly like the eval
/// viewer's stream.
pub(super) struct LiveEvents {
    sender: broadcast::Sender<Event>,
}

impl LiveEvents {
    /// Subscribe the store and start the forwarding thread.
    pub(super) fn start(server: &zuihitsu::Server) -> LiveEvents {
        let subscription = server.subscribe_events();
        let (sender, _) = broadcast::channel(1024);
        let fanout = sender.clone();
        std::thread::Builder::new()
            .name("control-event-fanout".to_owned())
            .spawn(move || {
                while let Ok(event) = subscription.recv() {
                    // No receivers is not an error — the console may simply not be open.
                    let _ = fanout.send(event);
                }
            })
            .expect("the event-fanout thread spawns");
        LiveEvents { sender }
    }

    fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }
}

/// `GET /control/events/stream?from=N` — the catch-up from `N` as `event` frames, then the live
/// tail pushed as it commits, with `progress` frames interleaved. The subscription is taken
/// before the snapshot is read and the overlap deduplicated by seq, so the cut is gapless. A
/// client that lags off the buffer has its stream ended and reconnects `?from=<last seen + 1>`.
///
/// Each SSE event has the name `d` and a `data:` payload that is a JSON `StreamFrame`. On
/// shutdown or broadcast lag the stream emits a `StreamFrame::End` and closes.
pub(super) async fn events_stream(
    State(state): State<AppState>,
    Query(query): Query<FromQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let mut events = state.live.subscribe();
    let mut progress = state.server.subscribe_progress();
    let shutdown = state.shutdown.clone();
    let snapshot = state.server.control().events_from(Seq(query.from))?;
    // An empty snapshot still honours `from`: falling back to zero would let the tail deliver
    // events below the requested horizon.
    let mut last_seq = snapshot
        .last()
        .map(|event| event.seq.0)
        .unwrap_or(query.from.saturating_sub(1));

    let body = async_stream::stream! {
        for event in snapshot {
            yield frame(StreamFrame::Event(Box::new(event)));
        }
        loop {
            tokio::select! {
                // The shared shutdown flag: without this arm the loop is unbounded (its feeds never
                // close on their own), so `with_graceful_shutdown` would wait on this connection
                // forever and the server would never exit. Breaking lets the connection drain.
                _ = shutdown.clone().wait() => {
                    yield frame(StreamFrame::End);
                    break;
                }
                committed = events.recv() => match committed {
                    Ok(event) => {
                        // The snapshot/subscription overlap: anything at or below the snapshot's
                        // horizon has already been sent.
                        if event.seq.0 <= last_seq {
                            continue;
                        }
                        last_seq = event.seq.0;
                        yield frame(StreamFrame::Event(Box::new(event)));
                    }
                    // Fell behind the committed feed: end the stream so the client reconnects from
                    // its horizon rather than resume across a gap.
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        yield frame(StreamFrame::End);
                        break;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        yield frame(StreamFrame::End);
                        break;
                    }
                },
                progress_frame = progress.recv() => match progress_frame {
                    Ok(progress) => {
                        yield frame(StreamFrame::Progress(progress));
                    }
                    // Progress is cosmetic; missing frames costs smoothness, never correctness, so
                    // a lag skips ahead rather than ending the stream.
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            }
        }
    };
    Ok(Sse::new(body).keep_alive(KeepAlive::default()))
}
