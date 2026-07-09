//! The scenario contract. A scenario declares its `steps` — a pure-data script the harness's executor
//! interprets against a booted agent — and `assess`es a run's log + the judge into verdicts. The two
//! are separate so a stored package can be re-assessed without re-running the model (spec §Validation).
//! The script is data, not an imperative body: the executor drives it and journals each step's
//! event-log coverage, so a recorded run carries its own scenario↔log correspondence. The product of a
//! run is still the event log, which `assess` reads.

use async_trait::async_trait;
use zuihitsu::{Event, InstanceFeatures};

use crate::{
    judge::Judge,
    package::{ScenarioMeta, Verdict},
    step::EvalStep,
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

    /// Whether the scenario needs a test MCP host (a fake server returning canned content). The
    /// harness skips such scenarios when no MCP host is configured, rather than running them without
    /// the tools the scenario depends on.
    fn needs_mcp(&self) -> bool {
        false
    }

    /// Which API features the scenario's instance enables — narrows the agent's Lua surface so a
    /// scenario can test a behaviour in isolation (e.g. disable `linking` to test the link-inference
    /// pass as the sole path to a link). Defaults to all-on; a scenario overrides this to narrow.
    fn features(&self) -> InstanceFeatures {
        InstanceFeatures::default()
    }

    /// The scenario's script — the ordered steps the executor drives the agent through. Pure,
    /// infallible data (the run-time-dependent steps defer their fallibility to execution); the run's
    /// event log, produced by executing the steps, is the product `assess` reads.
    fn steps(&self) -> Vec<EvalStep>;

    /// Judge a run's log into verdicts. Infallible: a judge error becomes a failed verdict carrying the
    /// error, never a harness crash.
    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict>;
}
