//! The declarative scenario script. A scenario is a `Vec<EvalStep>` of pure data — no `RunContext`,
//! no `.await` — that the [`execute`](crate::executor::execute) interpreter drives against a booted
//! agent. Making the script data (rather than an imperative `run` body) is what lets a recorded run
//! carry its own scenario↔log correspondence: the executor journals each step's event-log coverage, so
//! a later phase can replay or resume a recorded run against the current scenario's steps.
//!
//! Every variant mirrors one [`RunContext`](crate::context::RunContext) method the scenarios drive the
//! agent through, carrying that call's arguments as owned data. Two variants defer a run-time-dependent
//! decision to execution: [`EvalStep::ConfirmProposedMerge`] evaluates a merge-proposal lookup against
//! the live log, and [`StepText::WithTurnRef`] resolves a recorded turn's `[turn:<id>]` token from the
//! live log — neither can be known when the static script is written.
//!
//! The types are defined in `zuihitsu-frontend-types` and re-exported here.

pub use zuihitsu_frontend_types::{
    BurstMessage, EvalStep, InterruptedTurn, OnMissing, StepText, Turn,
};
