//! The scenario contract. A scenario `run`s the agent through a conversation (its product is the run's
//! event log) and `assess`es a run's log + the judge into verdicts. The two are separate so a stored
//! package can be re-assessed without re-running the model (spec §Validation).

use async_trait::async_trait;
use zuihitsu::Event;

use crate::{
    context::RunContext,
    error::EvalError,
    judge::Judge,
    package::{ScenarioMeta, Verdict},
};

#[async_trait]
pub trait Scenario: Send + Sync {
    /// The scenario's identity, category, and bar.
    fn meta(&self) -> ScenarioMeta;

    /// Whether the scenario needs the vector index (semantic recall). The harness skips such scenarios
    /// when no embedding endpoint is configured, rather than running them blind.
    fn needs_retrieval(&self) -> bool {
        false
    }

    /// Drive the conversation. The run's event log (read from `ctx` afterwards) is the product.
    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError>;

    /// Judge a run's log into verdicts. Infallible: a judge error becomes a failed verdict carrying the
    /// error, never a harness crash.
    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict>;
}
