//! Main-crate types: the wire-contract types that originated in the main `zuihitsu` crate.

use serde::{Deserialize, Serialize};

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

/// The circuit's observable state, for the operator health surface and the state gauge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    Closed,
    HalfOpen,
    Open,
}

/// The model transport's health, as the operator surface reports it: the circuit state, the
/// consecutive transient-failure count, the last failure's cause (kept across recovery, so an
/// operator can still read what went wrong), and — while open — how long until the half-open probe.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BackendHealth {
    pub circuit: CircuitState,
    pub consecutive_failures: u32,
    pub last_failure: Option<String>,
    pub open_remaining_ms: Option<u64>,
}
