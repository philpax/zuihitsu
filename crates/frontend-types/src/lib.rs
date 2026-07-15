//! The TypeScript wire-contract types shared between the main crate, the eval crate, and the
//! console. Owns every type that `ts-rs` exports to `console/packages/wire/types/`, so the build
//! pipeline has a single source of truth that depends only on `zuihitsu-core` — no build cycle
//! with the main crate's `build.rs`.
//!
//! The `ts` feature gates the `ts_rs::TS` derives. The `export-types` binary enables it to emit
//! the TypeScript bindings; consumers (the main crate, the eval crate) depend on this crate
//! without the feature for their normal builds.

pub mod agent;
pub mod executor;
pub mod live;
pub mod package;
pub mod step;

#[cfg(feature = "ts")]
pub mod export;

// Re-export the types at the crate root so consumers can import them without reaching into
// submodules (the types lived at the crate root before the module split).
pub use agent::{BackendHealth, CircuitState, PlatformResponse, TurnOutcome};
pub use executor::StepRecord;
pub use live::LiveEvent;
pub use package::{
    Aggregate, Bar, Category, EvalPackage, PackageSummary, ResumeProvenance, RunMeta, RunMetrics,
    RunRecord, RunSummary, ScenarioMeta, ScenarioReport, ScenarioSummary, Stat, TokenStat, Verdict,
    VerdictKind,
};
pub use step::{EvalStep, OnMissing, StepText, Turn};

// Re-export the wire types the crate depends on, so the eval crate can reach them through a
// single dependency rather than threading `zuihitsu-core` separately for just these few items.
// `Event`, `EventPayload`, `Usage`, and `TurnProgress` are also used in the type definitions —
// a `pub use` brings them into the local scope too, so no separate private import.
pub use zuihitsu_core::{
    event::{Event, EventPayload},
    ids::{Namespace, NamespacedMemoryName, Seq},
    model::Usage,
    progress::TurnProgress,
};
