//! The TypeScript wire-contract types shared between the main crate, the eval crate, and the
//! console. Owns every type that `ts-rs` exports to `console/packages/wire/types/`, so the build
//! pipeline has a single source of truth that depends only on `zuihitsu-core` — no build cycle
//! with the main crate's `build.rs`.
//!
//! The `ts` feature gates the `ts_rs::TS` derives. The `export-types` binary enables it to emit
//! the TypeScript bindings; consumers (the main crate, the eval crate) depend on this crate
//! without the feature for their normal builds.

use serde::{Deserialize, Serialize};

// Re-export the wire types the crate depends on, so the eval crate can reach them through a
// single dependency rather than threading `zuihitsu-core` separately for just these few items.
// `Event`, `EventPayload`, `Usage`, and `TurnProgress` are also used in this module's own type
// definitions — a `pub use` brings them into the local scope too, so no separate private import.
pub use zuihitsu_core::{
    event::{Event, EventPayload},
    ids::{Namespace, NamespacedMemoryName, Seq},
    model::Usage,
    progress::TurnProgress,
};

// ---------------------------------------------------------------------------
// Main-crate types: TurnOutcome, BackendHealth, CircuitState
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Eval-crate types: package.rs
// ---------------------------------------------------------------------------

/// One eval run over the whole scenario suite.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EvalPackage {
    pub meta: RunMeta,
    pub scenarios: Vec<ScenarioReport>,
}

/// What produced this package: the harness, the models, and the wall-clock span.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RunMeta {
    pub harness_version: String,
    /// The repository commit the harness ran at, when resolvable.
    pub git_sha: Option<String>,
    /// Whether the working tree had uncommitted changes when the run started. Best-effort like
    /// `git_sha`: an unavailable or failing git reads as clean. Added additively — an older package
    /// without the field deserializes as `false`.
    #[serde(default)]
    pub git_dirty: bool,
    pub model_id: String,
    pub embedding_model: Option<String>,
    /// The `--scenario` filter the run was targeted with, verbatim; absent for a full-suite run. Added
    /// additively — an older package without the field deserializes as `None`.
    #[serde(default)]
    pub scenario_filter: Option<String>,
    /// Epoch milliseconds.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub started_at_ms: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub finished_at_ms: i64,
    pub runs_per_scenario: u32,
    pub concurrency: u32,
    /// The package a `replay --mode rejudge` re-assessed to produce this one (the source stem). Present
    /// only on a rejudged package; added additively — an older or freshly-run package omits it, and a
    /// reader tells a re-judged package from a fresh run by its presence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejudged_from: Option<String>,
    /// The recorded run a `replay --mode resume` continued to produce this one. Present only on a
    /// resumed package; added additively, so an older or freshly-run package omits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_from: Option<ResumeProvenance>,
}

/// Where a resumed package's continuation was rewound from: the source package, the scenario and run
/// index within it, and the last recorded step kept before the live redo (keep-semantics). Recorded on
/// the resumed package's meta so a reader can trace the continuation back to its origin.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ResumeProvenance {
    /// The source package's path, as passed on the command line.
    pub package: String,
    pub scenario: String,
    pub run: u32,
    /// The last journal step kept from the recording; steps after it were redone live.
    pub step: u32,
}

/// One scenario's N runs plus their aggregate.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScenarioReport {
    pub meta: ScenarioMeta,
    pub runs: Vec<RunRecord>,
    pub aggregate: Aggregate,
}

/// A scenario's identity and its bar — the rubric the aggregate is read against.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScenarioMeta {
    pub name: String,
    pub category: Category,
    pub description: String,
    pub bar: Bar,
}

/// The scenario families. Descriptive groupings for the viewer; the set grows over time. Each
/// variant is a top-level scenario module (`crates/eval/src/scenarios/`), and the declaration order
/// here is the console's display order (`console/src/lib/model/scenarioGroups.ts`), so keep the two
/// aligned.
///
/// The serde aliases keep archived packages loading after a rename or retirement: a legacy
/// `scheduling` string reads back as [`Category::Time`], `description` as [`Category::Synthesis`],
/// and the two retired categories map onto their new homes — `compaction` onto [`Category::Sessions`]
/// (which absorbed the compaction, cold-open, and checkpoint scenarios) and `arbitration` onto
/// [`Category::Synthesis`] (which absorbed the arbitration scenarios).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Recall,
    Identity,
    Relations,
    Tagging,
    #[serde(alias = "scheduling")]
    Time,
    Privacy,
    #[serde(alias = "compaction")]
    Sessions,
    Writes,
    #[serde(alias = "description", alias = "arbitration")]
    Synthesis,
}

/// How a scenario's runs are judged (spec §Validation → gating versus measurement). A `Gating` bar
/// fails the harness when the rate of held gating-kind verdicts falls below `min_rate`: at the
/// default of 1.0 that is the one-slip discipline for must-not-surface safety properties, while a
/// tolerance below 1.0 suits model-judgment behaviors with a known error band, where an occasional
/// miss is expected but a systematic slide must still fail the run. A `Metric` bar is a
/// should-mark/should-surface rate that is reported against `threshold` but never fails the run.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Bar {
    Gating {
        #[serde(default = "full_rate")]
        min_rate: f64,
    },
    Metric {
        threshold: f64,
    },
}

/// The default gating tolerance: every gating verdict must hold.
fn full_rate() -> f64 {
    1.0
}

impl Bar {
    /// The one-slip gating bar — every gating verdict must hold across every run.
    pub fn gating() -> Self {
        Bar::Gating { min_rate: 1.0 }
    }

    /// A gating bar with tolerance: the harness fails when the held rate of gating verdicts falls
    /// below `min_rate`.
    pub fn gating_at(min_rate: f64) -> Self {
        Bar::Gating { min_rate }
    }

    /// The bar as judged, rendered for the trend record: `gating`, `gating>=<min_rate>` for a
    /// tolerant gate, or `>=<threshold>` for a metric bar (e.g. `>=0.6`). The archive keeps the bar
    /// each scenario was measured against so a later reader can tell a held gate from a met rate
    /// without the package.
    pub fn label(&self) -> String {
        match self {
            Bar::Gating { min_rate } if *min_rate >= 1.0 => "gating".to_owned(),
            Bar::Gating { min_rate } => format!("gating>={min_rate}"),
            Bar::Metric { threshold } => format!(">={threshold}"),
        }
    }

    /// Whether a scenario's aggregate holds this bar for the harness's exit signal. At the default
    /// tolerance the boolean gating signal decides (so packages predating `gating_rate` judge
    /// correctly); below it, the held rate of gating verdicts is compared against `min_rate`. A
    /// `Metric` bar never fails the run.
    pub fn holds(&self, gating_rate: f64, gating_passed: bool) -> bool {
        match self {
            Bar::Gating { min_rate } if *min_rate >= 1.0 => gating_passed,
            Bar::Gating { min_rate } => gating_rate >= *min_rate,
            Bar::Metric { .. } => true,
        }
    }
}

/// One run: the run's whole event log (the deliberation and resulting state the viewer reconstructs),
/// its verdicts, and the metrics computed from its `ModelCalled` events.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RunRecord {
    pub index: u32,
    /// The harness's wall-clock (epoch milliseconds) when the run began driving and when it finished.
    /// The real clock, not the scenario's simulated one — for the viewer's live elapsed and projection.
    /// `#[serde(default)]` fills `0` so a pre-timing sidecar or package still deserializes; a `0` reads
    /// as "unstamped" and the viewer omits the per-run times rather than rendering an epoch.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub started_at_ms: i64,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub finished_at_ms: i64,
    pub events: Vec<Event>,
    /// The executor's per-step journal: each step and the span of event seqs it appended, in step
    /// order. It carries the run's scenario↔log correspondence — which step produced which events, and
    /// the watermark to restore the store up to a given step. Added additively: an older package
    /// without it deserializes to an empty journal, and an empty journal is omitted from the wire so
    /// old and new packages stay compact.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub journal: Vec<StepRecord>,
    pub verdicts: Vec<Verdict>,
    pub metrics: RunMetrics,
}

/// One oracle's outcome for a run. `judge_raw` carries the judge model's verbatim response when a
/// criterion was judged (rather than checked deterministically), so the matcher stays reviewable.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Verdict {
    pub criterion: String,
    pub kind: VerdictKind,
    pub passed: bool,
    pub rationale: String,
    pub judge_raw: Option<String>,
}

impl Verdict {
    /// A gating safety oracle's outcome — a must-not-surface property whose regression fails the
    /// harness. `judge_raw` carries the matcher's verbatim reasoning when one was consulted.
    pub fn oracle(
        criterion: impl Into<String>,
        passed: bool,
        rationale: impl Into<String>,
        judge_raw: Option<String>,
    ) -> Verdict {
        Verdict {
            criterion: criterion.into(),
            kind: VerdictKind::Oracle,
            passed,
            rationale: rationale.into(),
            judge_raw,
        }
    }

    /// A gating oracle whose rationale reads differently when it holds than when it does not — the
    /// `oracle` counterpart of [`Verdict::metric_outcome`], for a deterministic correctness property
    /// reliable enough that a regression should fail the harness rather than only lower a rate. The
    /// `when_failed` message is what makes that failure legible.
    pub fn oracle_outcome(
        criterion: impl Into<String>,
        passed: bool,
        when_passed: impl Into<String>,
        when_failed: impl Into<String>,
    ) -> Verdict {
        let rationale = if passed {
            when_passed.into()
        } else {
            when_failed.into()
        };
        Verdict::oracle(criterion, passed, rationale, None)
    }

    /// A deterministically-checked quality metric (no judge; `judge_raw` is `None`).
    pub fn metric(
        criterion: impl Into<String>,
        passed: bool,
        rationale: impl Into<String>,
    ) -> Verdict {
        Verdict {
            criterion: criterion.into(),
            kind: VerdictKind::Metric,
            passed,
            rationale: rationale.into(),
            judge_raw: None,
        }
    }

    /// A deterministic metric whose rationale reads differently when it holds than when it does not —
    /// the common "passed: did X / failed: did not do X" shape. `when_passed` is recorded if `passed`,
    /// `when_failed` otherwise.
    pub fn metric_outcome(
        criterion: impl Into<String>,
        passed: bool,
        when_passed: impl Into<String>,
        when_failed: impl Into<String>,
    ) -> Verdict {
        let rationale = if passed {
            when_passed.into()
        } else {
            when_failed.into()
        };
        Verdict::metric(criterion, passed, rationale)
    }

    /// A judged quality metric, carrying the matcher's verbatim reasoning (`judge_raw`) so a rate built
    /// from model judgments stays reviewable.
    pub fn metric_judged(
        criterion: impl Into<String>,
        passed: bool,
        rationale: impl Into<String>,
        judge_raw: String,
    ) -> Verdict {
        Verdict {
            criterion: criterion.into(),
            kind: VerdictKind::Metric,
            passed,
            rationale: rationale.into(),
            judge_raw: Some(judge_raw),
        }
    }
}

/// Whether a verdict is a gating safety oracle or a reported quality metric.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum VerdictKind {
    Oracle,
    Metric,
}

/// Per-run measurements, summed from the run's `ModelCalled` events.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RunMetrics {
    pub model_calls: u32,
    pub steps: u32,
    /// The run's actual wall-clock to drive (turns plus the synchronous catch-ups it forces). The
    /// truthful cost: unlike `total_latency_ms`, it includes work that records no `ModelCalled` — the
    /// background describer's synthesis, run synchronously in the harness.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub wall_clock_ms: u64,
    /// Summed `ModelCalled` duration. Conversational model calls only — off-hot-path synthesis records
    /// no `ModelCalled`, so this undercounts total compute; read `wall_clock_ms` for that.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub total_latency_ms: u64,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    /// Whether every gating oracle in this run passed.
    pub gating_passed: bool,
}

/// A scenario's aggregate across its N runs.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Aggregate {
    pub runs: u32,
    /// The pass rate over the runs (1.0 = every run passed every oracle of its bar).
    pub rate: f64,
    /// True iff every gating oracle held in every run (the safety invariant; drives the exit code).
    pub gating_passed: bool,
    /// The rate of runs whose gating oracles all held — what a tolerant gating bar's `min_rate` is
    /// judged against. Defaults to 1.0 for packages predating the field; those are only ever judged
    /// at the default tolerance, where the boolean signal decides.
    #[serde(default = "full_rate")]
    pub gating_rate: f64,
    /// The per-run drive wall-clock distribution (the truthful cost; see [`RunMetrics::wall_clock_ms`]).
    pub wall_clock_ms: Stat,
    pub latency_ms: Stat,
    pub tokens: TokenStat,
    pub steps_mean: f64,
}

/// A latency distribution across runs (milliseconds).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Stat {
    pub p50: f64,
    pub p95: f64,
    pub mean: f64,
}

/// Mean token usage across runs.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TokenStat {
    pub prompt_mean: f64,
    pub completion_mean: f64,
    pub total_mean: f64,
}

/// The live wire's lean face of one run: everything the scoreboard, the rail, and the deep-dive's
/// verdict panel render, without the run's event log. The log is the bulk of a package (a soak run's
/// is hundreds of megabytes over the whole suite), and only the one open run's deep-dive ever needs
/// it, so the console fetches a single run's full [`RunRecord`] on demand rather than holding every
/// run's log. `usages` is each `ModelCalled` event's [`Usage`] in event order — exactly what the
/// console's cache-warmth rollup reads, kept here so the rail's warmth marks need no event log.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RunSummary {
    pub index: u32,
    /// The run's wall-clock stamps, mirroring [`RunRecord::started_at_ms`] and its finish; see there.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub started_at_ms: i64,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub finished_at_ms: i64,
    pub verdicts: Vec<Verdict>,
    pub metrics: RunMetrics,
    /// Each `ModelCalled` event's usage, in event order.
    pub usages: Vec<Usage>,
}

impl From<&RunRecord> for RunSummary {
    fn from(record: &RunRecord) -> Self {
        let usages = record
            .events
            .iter()
            .filter_map(|event| match &event.payload {
                EventPayload::ModelCalled { usage, .. } => Some(*usage),
                _ => None,
            })
            .collect();
        RunSummary {
            index: record.index,
            started_at_ms: record.started_at_ms,
            finished_at_ms: record.finished_at_ms,
            verdicts: record.verdicts.clone(),
            metrics: record.metrics,
            usages,
        }
    }
}

/// The lean face of one scenario's report: its identity and bar, its runs as [`RunSummary`]s, and the
/// aggregate — the whole scenario overview, without any run's event log.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScenarioSummary {
    pub meta: ScenarioMeta,
    pub runs: Vec<RunSummary>,
    pub aggregate: Aggregate,
}

impl From<&ScenarioReport> for ScenarioSummary {
    fn from(report: &ScenarioReport) -> Self {
        ScenarioSummary {
            meta: report.meta.clone(),
            runs: report.runs.iter().map(RunSummary::from).collect(),
            aggregate: report.aggregate,
        }
    }
}

/// The lean face of a whole package: the run's metadata and every scenario as a [`ScenarioSummary`].
/// This is what the live `--serve` snapshot ships and the console folds — everything the scoreboard
/// and rail need to render, without the event logs, which the console fetches per run on demand.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PackageSummary {
    pub meta: RunMeta,
    pub scenarios: Vec<ScenarioSummary>,
}

impl From<&EvalPackage> for PackageSummary {
    fn from(package: &EvalPackage) -> Self {
        PackageSummary {
            meta: package.meta.clone(),
            scenarios: package
                .scenarios
                .iter()
                .map(ScenarioSummary::from)
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Eval-crate types: step.rs (EvalStep, Turn, StepText, OnMissing)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Eval-crate types: executor.rs (StepRecord)
// ---------------------------------------------------------------------------

/// One step's event-log coverage. The span (`first_seq`..=`last_seq`) is the events the step appended,
/// and `seq_after` is the log head after it — the watermark phase two restores the store up to when it
/// resumes from a step. The spans are contiguous and non-overlapping across the journal, so every
/// event belongs to exactly one step; `seq_after` is monotone non-decreasing, unchanged by a step that
/// appended nothing, and equal to the log head after the final step.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StepRecord {
    pub index: u32,
    pub step: EvalStep,
    /// The seq of the first event the step appended, or `None` if it appended none (`Advance` only
    /// moves the clock; a skipped `ConfirmProposedMerge` performs nothing).
    pub first_seq: Option<Seq>,
    /// The seq of the last event the step appended, or `None` if it appended none.
    pub last_seq: Option<Seq>,
    /// The log head after this step — the restore watermark for resuming at step K.
    pub seq_after: Seq,
    /// Whether the step performed no operation because a run-time precondition was absent (a
    /// `ConfirmProposedMerge` with `on_missing: Skip` and no proposal in the log).
    pub skipped: bool,
}

// ---------------------------------------------------------------------------
// Eval-crate types: live/mod.rs (LiveEvent)
// ---------------------------------------------------------------------------

/// One event in an eval run's live log. Emitted in order; the console folds the sequence into an
/// [`EvalPackage`], and the harness persists it as a `.jsonl` sidecar.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LiveEvent {
    /// The plan: the run's metadata and every scenario that will run, in order. First event; seeds the
    /// scoreboard with all scenarios before any has a result. `scenario` indices elsewhere point into
    /// this list.
    Manifest {
        meta: RunMeta,
        scenarios: Vec<ScenarioMeta>,
    },
    /// A run began. `at_ms` is the harness's wall-clock (epoch milliseconds) at the start — the real
    /// clock, for the viewer's live elapsed and projection. `#[serde(default)]` fills `0` for a
    /// pre-timing sidecar line.
    RunStarted {
        scenario: u32,
        run: u32,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        at_ms: i64,
    },
    /// One domain event from a run's deliberation, streamed live as it happens. Broadcast-only — it
    /// drives the deep-dive's unfolding view, but is *not* written to the sidecar: the authoritative
    /// record arrives in `RunCompleted`, so a client that joined mid-run (and missed some of these) is
    /// still made whole.
    RunEvent {
        scenario: u32,
        run: u32,
        event: Event,
    },
    /// One in-flight generation fragment from a run's deliberation — the same [`TurnProgress`]
    /// frame the live agent console streams, multiplexed with which run produced it. Broadcast-only
    /// and deliberately not buffered for catch-up: a viewer joining mid-generation misses earlier
    /// tokens and simply picks up from the next frame, because the durable record (`ModelCalled`,
    /// or `ModelCallAborted` for a discarded attempt) arrives as a `RunEvent` regardless.
    RunProgress {
        scenario: u32,
        run: u32,
        frame: TurnProgress,
    },
    /// A run finished: its whole record (events, verdicts, metrics) and the scenario's aggregate
    /// recomputed over its runs so far. Authoritative — folding it reproduces the canonical package
    /// regardless of which live `RunEvent`s a client happened to see.
    RunCompleted {
        scenario: u32,
        run: u32,
        record: RunRecord,
        aggregate: Aggregate,
        /// The harness's wall-clock (epoch milliseconds) at completion — mirrors `record.finished_at_ms`
        /// so a viewer folding only the live stream has the finish clock without unpacking the record.
        /// `#[serde(default)]` fills `0` for a pre-timing sidecar line.
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        at_ms: i64,
    },
    /// A run finished, in the live wire's lean face: the run's [`RunSummary`] (verdicts, metrics, and
    /// each model call's usage — everything the scoreboard and rail read) and the scenario's
    /// recomputed aggregate, without the event log. Broadcast-only, like `RunEvent`/`RunProgress`: it
    /// is never written to the sidecar, because the sidecar's `RunCompleted` carries the authoritative
    /// full record, which the console fetches per run over `GET /eval/run/{scenario}/{run}` when a
    /// deep-dive opens the run.
    RunSummarized {
        scenario: u32,
        run: u32,
        summary: RunSummary,
        aggregate: Aggregate,
        /// The harness's wall-clock (epoch milliseconds) at completion — mirrors `RunCompleted`'s
        /// `at_ms`, defaulted defensively even though the frame is broadcast-only and never persisted.
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        at_ms: i64,
    },
    /// The whole run completed.
    Finished {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        finished_at_ms: i64,
    },
}

// ---------------------------------------------------------------------------
// Type export and console-constants generation
// ---------------------------------------------------------------------------

/// Export all TypeScript wire-contract types to `dir`, and write the Rust constants the console
/// consumes as runtime values. This is the body of the `export-types` binary.
#[cfg(feature = "ts")]
pub fn export_types(dir: &std::path::Path) -> Result<(), String> {
    use ts_rs::TS;

    EvalPackage::export_all_to(dir)
        .and_then(|()| PackageSummary::export_all_to(dir))
        .and_then(|()| LiveEvent::export_all_to(dir))
        .and_then(|()| Namespace::export_all_to(dir))
        .and_then(|()| NamespacedMemoryName::export_all_to(dir))
        .and_then(|()| TurnOutcome::export_all_to(dir))
        .and_then(|()| BackendHealth::export_all_to(dir))
        .and_then(|()| TurnProgress::export_all_to(dir))
        .map_err(|error| error.to_string())
        .and_then(|()| write_console_constants(dir).map_err(|error| error.to_string()))?;
    println!(
        "exported the eval-package and live-event types, and the console constants, to {}",
        dir.display()
    );
    Ok(())
}

/// Emit the Rust constants the console needs as runtime *values* (ts-rs exports types, not consts),
/// so Rust stays the single source of truth for values that are load-bearing on both sides. Today
/// that is the `DIRECT_PLATFORM` key: identity resolution merges an arrival on it under operator
/// authority (spec §Cross-platform identity), and the console builds its own room locators with it —
/// a drift between the two would silently break that reconciliation.
#[cfg(feature = "ts")]
fn write_console_constants(dir: &std::path::Path) -> std::io::Result<()> {
    use zuihitsu_core::ids::DIRECT_PLATFORM;
    let contents = format!(
        "// Generated by `cargo build -p zuihitsu` — do not edit. Rust constants the console consumes \
         as values.\n\n\
         /// The platform key for the operator's own direct interface (Rust `ids::DIRECT_PLATFORM`).\n\
         export const DIRECT_PLATFORM = {:?};\n",
        DIRECT_PLATFORM,
    );
    std::fs::write(dir.join("constants.ts"), contents)
}

#[cfg(test)]
mod tests {
    use super::RunRecord;

    /// A pre-timing package predates the wall-clock stamps; `#[serde(default)]` must fill `0` so the
    /// old record still deserializes, and a `0` reads as "unstamped" for the viewer.
    #[test]
    fn a_run_record_without_stamps_defaults_them_to_zero() {
        let old = r#"{"index":3,"events":[],"verdicts":[],"metrics":{"model_calls":0,"steps":0,"wall_clock_ms":0,"total_latency_ms":0,"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"gating_passed":true}}"#;
        let record: RunRecord = serde_json::from_str(old).expect("old-shape record parses");
        assert_eq!(record.index, 3);
        assert_eq!(record.started_at_ms, 0);
        assert_eq!(record.finished_at_ms, 0);
    }

    /// A stamped record round-trips through JSON, carrying both wall-clock stamps.
    #[test]
    fn a_stamped_run_record_round_trips() {
        let record = RunRecord {
            index: 1,
            started_at_ms: 1_700_000_000_000,
            finished_at_ms: 1_700_000_042_000,
            events: Vec::new(),
            journal: Vec::new(),
            verdicts: Vec::new(),
            metrics: super::RunMetrics::default(),
        };
        let json = serde_json::to_string(&record).expect("serializes");
        let back: RunRecord = serde_json::from_str(&json).expect("round-trips");
        assert_eq!(back.started_at_ms, 1_700_000_000_000);
        assert_eq!(back.finished_at_ms, 1_700_000_042_000);
    }

    /// A package predating the step journal has no `journal` field; `#[serde(default)]` must fill an
    /// empty journal so the old record still deserializes, and an empty journal is omitted on the wire.
    #[test]
    fn a_run_record_without_a_journal_defaults_it_empty() {
        let old = r#"{"index":0,"started_at_ms":0,"finished_at_ms":0,"events":[],"verdicts":[],"metrics":{"model_calls":0,"steps":0,"wall_clock_ms":0,"total_latency_ms":0,"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"gating_passed":true}}"#;
        let record: RunRecord = serde_json::from_str(old).expect("journal-less record parses");
        assert!(record.journal.is_empty());
        let json = serde_json::to_string(&record).expect("serializes");
        assert!(
            !json.contains("journal"),
            "empty journal is omitted: {json}"
        );
    }

    /// A record carrying a journal round-trips: the steps and their seq coverage survive the wire.
    #[test]
    fn a_run_record_with_a_journal_round_trips() {
        use zuihitsu_core::ids::Seq;

        use super::StepRecord;
        use crate::{EvalStep, Turn};

        let record = RunRecord {
            index: 2,
            started_at_ms: 0,
            finished_at_ms: 0,
            events: Vec::new(),
            journal: vec![
                StepRecord {
                    index: 0,
                    step: EvalStep::Turn(Turn::new("discord", "team", "dave", "hello")),
                    first_seq: Some(Seq(1)),
                    last_seq: Some(Seq(4)),
                    seq_after: Seq(4),
                    skipped: false,
                },
                StepRecord {
                    index: 1,
                    step: EvalStep::Advance { millis: 1_000 },
                    first_seq: None,
                    last_seq: None,
                    seq_after: Seq(4),
                    skipped: false,
                },
            ],
            verdicts: Vec::new(),
            metrics: super::RunMetrics::default(),
        };
        let json = serde_json::to_string(&record).expect("serializes");
        let back: RunRecord = serde_json::from_str(&json).expect("round-trips");
        assert_eq!(back.journal.len(), 2);
        assert_eq!(back.journal[0].step, record.journal[0].step);
        assert_eq!(back.journal[0].last_seq, Some(Seq(4)));
        assert_eq!(back.journal[1].seq_after, Seq(4));
    }
}
