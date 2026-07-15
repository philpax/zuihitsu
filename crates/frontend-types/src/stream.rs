//! The streaming protocol's typed frame. See `docs/connector-protocol.md` for the full protocol
//! specification — endpoints, request/response shapes, wire format, and frame semantics.

use serde::{Deserialize, Serialize};

use crate::{Event, TurnProgress, agent::PlatformResponse};

/// One frame in a streaming response. Tagged with `#[serde(tag = "type")]` so each SSE `data:`
/// payload is a self-describing JSON object whose `type` field discriminates the variant.
///
/// The wire format is one SSE event per frame, with no `event:` field — just `data:`:
///
/// ```text
/// data: {"type":"progress","conversation":"01J…","turn_id":"01J…","phase":"reply","kind":"reply","text":"Hello","step":0}
///
/// data: {"type":"outcome","outcome":{"Reply":"Hello there, Dave."},"participant_turn_ids":["01J…"]}
/// ```
///
/// A consumer reads SSE events, takes each `data:` payload, and deserialises it as `StreamFrame`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamFrame {
    /// An ephemeral progress fragment from an in-flight generation. Never stored; a dropped
    /// frame costs smoothness, never correctness.
    Progress(TurnProgress),
    /// A committed event appended to the store, delivered to a live viewer. The `seq` field on
    /// the event is the monotonic cursor a consumer tracks for reconnection (`?from=<last+1>`).
    /// Boxed because `Event` is much larger than the other variants.
    Event(Box<Event>),
    /// The terminal frame for `/platform/messages/stream`: the turn completed, and this is the
    /// same `PlatformResponse` the unary endpoint returns. A connector that ignores every
    /// `Progress` frame behaves identically to one that never upgraded.
    Outcome(PlatformResponse),
    /// The terminal frame for `/control/events/stream`: the server is closing the stream (a
    /// broadcast lag or shutdown), and the consumer should reconnect from its last seen seq.
    End,
    /// A turn failure — the error message from the server. Terminal for
    /// `/platform/messages/stream`.
    Error { message: String },
}
