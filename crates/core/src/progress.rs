//! Ephemeral turn progress: the deliberation a live viewer watches arrive token by token.
//!
//! A [`TurnProgress`] frame is **not an event**. It never reaches the store, the materialiser never
//! sees one, and replay is byte-identical with or without a viewer — the committed record of a model
//! call remains the terminal `ModelCalled`, whose full completion, reasoning, and usage exist only at
//! stream end. Frames exist solely so the console can render an in-flight generation as it happens
//! rather than a silent gap followed by a wall of text; they ride a lossy broadcast channel and a
//! server-sent-event stream, and a dropped or missed frame costs nothing but smoothness.

use serde::{Deserialize, Serialize};

use crate::{
    event::ModelPhase,
    ids::{ConversationId, TurnId},
};

/// One fragment of an in-flight generation, attributed to the turn producing it so a viewer
/// following several conversations can file it correctly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TurnProgress {
    pub conversation: ConversationId,
    pub turn_id: TurnId,
    pub phase: ModelPhase,
    pub kind: ProgressKind,
    /// The fragment's text, appended to whatever this turn has already streamed for `kind`. A new
    /// model call within the same turn (the next step of the loop) starts a fresh accumulation;
    /// the `step` counter marks the boundary.
    pub text: String,
    /// Which model call of the turn this fragment belongs to, counting from zero — the console
    /// resets its accumulated text when the step advances, since each step is its own generation.
    pub step: u32,
}

/// What a fragment carries: one of the generation's two text surfaces, or a lifecycle marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ProgressKind {
    /// The serving layer's `reasoning_content` — the deliberation before the reply.
    Reasoning,
    /// The reply text itself.
    Reply,
    /// The retry wrapper discarded the attempt streamed so far and is re-driving the request: a
    /// viewer voids everything accumulated for this step. `text` carries the failure's cause.
    Restart,
    /// The generation died with no durable successor — retries exhausted, the turn defers. A
    /// deferral records no agent `ConversationTurn`, so without this marker a viewer would show a
    /// frozen "generating…" turn and pulse the room until some later turn happened to land; on it,
    /// a viewer drops the step's accumulation outright. `text` carries the failure's cause.
    Abandoned,
}
