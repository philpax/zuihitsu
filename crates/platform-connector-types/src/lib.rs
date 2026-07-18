//! Connector-facing wire types: the response a connector receives, and the streaming protocol's
//! typed frame. Shared between the main crate, the `zuihitsu-platform-connector-api` crate, and the
//! console's TypeScript bindings.
//!
//! The `ts` feature gates the `ts_rs::TS` derives. The export pipeline (in `frontend-types`)
//! enables it to emit the TypeScript bindings; consumers depend on this crate without the
//! feature for their normal builds.

use serde::{Deserialize, Serialize};

use zuihitsu_core::{event::Event, progress::TurnProgress};

/// What a completed turn delivers to the platform client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum TurnOutcome {
    /// A reply to post back.
    Reply(String),
    /// The stay-silent terminal — nothing to post.
    Silent,
    /// The step budget was exhausted without a terminal; recorded for the agent to reason about.
    MaxStepsExceeded,
    /// The inbound message was delivered and durably recorded, but the model backend was
    /// unreachable (transient failure with retries exhausted, or an open circuit), so no response
    /// cycle ran. Nothing is lost, and catch-up is passive by design: the next inbound message's
    /// turn replays the buffer — which includes every deferred inbound — so one response cycle
    /// covers them all. There is no active on-recovery push, because replies have no delivery
    /// channel to platform clients besides the message-response path, and agent-initiated contact
    /// is a deliberately deferred design area.
    Deferred,
    /// The turn was cooperatively cancelled: a newer inbound message batch arrived for the same
    /// conversation while this turn was generating, and the newer batch's turn answers once with
    /// everything in context. No reply is coming via this request. Connectors treat it like
    /// [`TurnOutcome::Silent`] — there is nothing to post, because the successor's reply reaches the
    /// client through its own request. `participant_turn_ids` still carries the recorded inbound turn
    /// ids, so a connector can map its own message ids to zuihitsu turns for later `[turn:<id>]`
    /// injection even though this batch's messages were folded into the successor's answer.
    Superseded,
}

/// The response from `POST /platform/messages` and `POST /platform/messages/stream`: the turn's
/// `outcome` plus the `participant_turn_ids` of the inbound participant turns. A connector uses
/// the participant turn ids to map its own message ids to zuihitsu turns, so it can inject a
/// `[turn:<id>]` token when a user replies to one of those messages later.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PlatformResponse {
    /// The turn's conversational outcome — what the connector should do with the reply.
    pub outcome: TurnOutcome,
    /// The participant turn ids (Crockford ULID strings), one per inbound message, for
    /// `[turn:<id>]` mapping.
    pub participant_turn_ids: Vec<String>,
}

/// The streaming protocol's typed frame. See `docs/connector-protocol.md` for the full protocol
/// specification — endpoints, request/response shapes, wire format, and frame semantics.
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
