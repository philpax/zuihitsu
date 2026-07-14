//! The declarative scenario script types. A scenario is a `Vec<EvalStep>` of pure data — no
//! `RunContext`, no `.await` — that the executor interpreter drives against a booted agent.

use serde::{Deserialize, Serialize};
use zuihitsu_core::event::EventPayload;

/// One beat of a scenario's script — a single operation the executor performs against the run's agent,
/// mirroring the `RunContext` method of the same name. Owned data with no borrows, so a script
/// serializes into the run record and a recorded step compares structurally (`PartialEq`) against the
/// current scenario's step — phase two's drift detector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum EvalStep {
    /// Route one inbound participant message and run the agent's turn.
    Turn(Turn),
    /// Drive one operator imprint-interview turn.
    Imprint { text: String },
    /// Let both background synthesis passes settle — the describer, then the vector indexer.
    Settle,
    /// Advance the run's clock by `millis`.
    Advance {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        millis: i64,
    },
    /// Regenerate descriptions, belief arbitration, and temporal extraction.
    DescribeCatchUp,
    /// Adjudicate the merges proposed so far.
    AdjudicateCatchUp,
    /// Infer links from the content written so far.
    LinkInferenceCatchUp,
    /// Run one checkpoint sweep over the live sessions.
    CheckpointSweep,
    /// Append raw events to the store and materialize the graph.
    SeedEvents(Vec<EventPayload>),
    /// Tighten the compaction trigger so a short scripted session crosses the token budget.
    TightenCompaction {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        token_budget: i64,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        flush_min_turns: i64,
    },
    /// Force a compaction of the open session in `platform`/`scope`.
    ForceCompaction { platform: String, scope: String },
    /// Tune the checkpoint gates so a scripted two-room exchange trips them.
    TuneCheckpoint {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        min_delta_chars: i64,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        cooldown_seconds: i64,
        /// Whether a fresh session open flushes the other live rooms first. A timer-path scenario
        /// sets this `false` so the open trigger does not pre-empt its explicit `CheckpointSweep`.
        /// Absent from a package recorded before the field existed, it defaults `true` — the setting's
        /// own default — so an older run's `TuneCheckpoint` still deserializes.
        #[serde(default = "default_flush_on_open")]
        flush_on_open: bool,
    },
    /// Confirm the first merge proposed in the live log as the operator would, resolved at execution
    /// time: the proposed pair is looked up against the run's log and, if found, an operator `same_as`
    /// merge is authored. When no proposal is present, `on_missing` decides — skip the step or fail
    /// the run.
    ConfirmProposedMerge { on_missing: OnMissing },
}

/// The serde default for [`EvalStep::TuneCheckpoint`]'s `flush_on_open`, matching the setting's own
/// build default, so a package recorded before the field existed deserializes with the open trigger on.
fn default_flush_on_open() -> bool {
    true
}

impl EvalStep {
    /// An [`EvalStep::Imprint`] carrying `text` — the ergonomic constructor for the common case.
    pub fn imprint(text: impl Into<String>) -> EvalStep {
        EvalStep::Imprint { text: text.into() }
    }

    /// Whether performing this step routes an inbound and runs the agent's model-driven turn loop —
    /// the steps that unconditionally issue at least one generation. Only [`EvalStep::Turn`] and
    /// [`EvalStep::Imprint`] qualify; the catch-up, seeding, and tuning steps never call the
    /// conversational model. [`EvalStep::ForceCompaction`] is deliberately excluded even though its
    /// flush can call the model: the flush is a no-op when no `Flush` template is registered, so a
    /// forced compaction may legitimately record no calls, and counting it here would let the
    /// infra-failure detector mistake that no-op for an outage.
    ///
    /// The `infra_failed` detector reads this to tell a run whose every turn deferred (the model
    /// backend was unreachable) from a scenario that legitimately never calls the model at all.
    pub fn drives_model(&self) -> bool {
        matches!(self, EvalStep::Turn(_) | EvalStep::Imprint { .. })
    }
}

/// One inbound participant message — the payload of [`EvalStep::Turn`], carrying the arguments
/// `RunContext::turn` drives. `present` defaults to just the sender; [`Turn::with_present`] overrides
/// it when others share the room, since who else is present changes what the visibility predicate
/// surfaces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Turn {
    pub platform: String,
    pub scope: String,
    pub sender: String,
    pub text: StepText,
    pub present: Vec<String>,
}

impl Turn {
    /// A turn from `sender` in `platform`/`scope`, with `sender` as the only one present. `text` is any
    /// [`StepText`] — a bare `&str`/`String` becomes a [`StepText::Literal`].
    pub fn new(
        platform: impl Into<String>,
        scope: impl Into<String>,
        sender: impl Into<String>,
        text: impl Into<StepText>,
    ) -> Turn {
        let sender = sender.into();
        Turn {
            platform: platform.into(),
            scope: scope.into(),
            present: vec![sender.clone()],
            sender,
            text: text.into(),
        }
    }

    /// Override who is present for this turn (the default is the sender alone). The sender is always
    /// present, so it is added if the caller's set omits it.
    pub fn with_present(mut self, present: &[&str]) -> Turn {
        self.present = present.iter().map(|name| (*name).to_owned()).collect();
        if !self.present.iter().any(|name| name == &self.sender) {
            self.present.push(self.sender.clone());
        }
        self
    }
}

impl From<Turn> for EvalStep {
    fn from(turn: Turn) -> EvalStep {
        EvalStep::Turn(turn)
    }
}

/// A turn's text: either a literal string, or a template whose `{turn}` marker is replaced at
/// execution time by the `[turn:<id>]` token of a recorded turn. The recorded turn is the first
/// participant `ConversationTurn` in the run's log whose text is exactly `of_turn` — the connector
/// contract's canonical token, resolved against the live log so the script references the exact turn id
/// the agent will resolve rather than a fabricated one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum StepText {
    Literal(String),
    WithTurnRef { template: String, of_turn: String },
}

impl StepText {
    /// A template referencing an earlier recorded turn: the first participant turn whose text is
    /// exactly `of_turn`. Its `[turn:<id>]` token is substituted for the `{turn}` marker in `template`
    /// when the step executes.
    pub fn with_turn_ref(template: impl Into<String>, of_turn: impl Into<String>) -> StepText {
        StepText::WithTurnRef {
            template: template.into(),
            of_turn: of_turn.into(),
        }
    }
}

impl From<&str> for StepText {
    fn from(text: &str) -> StepText {
        StepText::Literal(text.to_owned())
    }
}

impl From<String> for StepText {
    fn from(text: String) -> StepText {
        StepText::Literal(text)
    }
}

/// What [`EvalStep::ConfirmProposedMerge`] does when no merge proposal is present in the live log. A
/// scenario whose whole point is the no-proposal case uses [`OnMissing::Skip`] — a hard failure would
/// abort the run and destroy the verdicts that document that case — while a scenario that requires a
/// proposal uses [`OnMissing::Fail`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum OnMissing {
    /// Record the step as skipped in the journal and continue.
    Skip,
    /// Fail the run.
    Fail,
}
