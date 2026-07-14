import type { EvalPackage } from "@zuihitsu/wire/types/EvalPackage.ts";
import type { PackageSummary } from "@zuihitsu/wire/types/PackageSummary.ts";
import type { RunRecord } from "@zuihitsu/wire/types/RunRecord.ts";
import type { RunSummary } from "@zuihitsu/wire/types/RunSummary.ts";

/// Reduce a full run record to its lean summary — mirroring the Rust `From<&RunRecord> for RunSummary`
/// (`crates/eval/src/package.rs`). `usages` is each `ModelCalled` event's usage in event order, the
/// input the cache-warmth rollup reads, so the summary carries warmth without the event log.
export function summarizeRun(record: RunRecord): RunSummary {
  return {
    index: record.index,
    started_at_ms: record.started_at_ms,
    finished_at_ms: record.finished_at_ms,
    verdicts: record.verdicts,
    metrics: record.metrics,
    usages: record.events.flatMap((event) =>
      event.payload.type === "ModelCalled" ? [event.payload.usage] : [],
    ),
  };
}

/// Reduce a file-loaded full package to the lean [`PackageSummary`] the eval frame renders — mirroring
/// the Rust `From<&EvalPackage> for PackageSummary`. The full package is retained separately so a
/// deep-dive can resolve a run's whole record synchronously (see `App.tsx`'s file-mode `getRun`); this
/// is only what the scoreboard and rail read.
export function summarizePackage(pkg: EvalPackage): PackageSummary {
  return {
    meta: pkg.meta,
    scenarios: pkg.scenarios.map((scenario) => ({
      meta: scenario.meta,
      runs: scenario.runs.map(summarizeRun),
      aggregate: scenario.aggregate,
    })),
  };
}
