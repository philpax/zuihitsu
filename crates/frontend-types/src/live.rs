//! The live eval log: the event stream a run emits.

use serde::{Deserialize, Serialize};
use zuihitsu_core::{event::Event, progress::TurnProgress};

use crate::package::{Aggregate, RunMeta, RunRecord, RunSummary, ScenarioMeta};

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
        // Boxed to keep this variant from dwarfing the enum's others: an `Event` carries the whole
        // event payload (the largest being a settings snapshot), so inlining it here trips
        // `large_enum_variant`. `Box` is serde- and ts-rs-transparent, so the wire and the generated
        // TypeScript are unchanged.
        event: Box<Event>,
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
