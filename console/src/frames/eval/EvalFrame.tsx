import type { ReactNode } from "react";

import type { PackageSummary } from "@zuihitsu/wire/types/PackageSummary.ts";
import type { RunRecord } from "@zuihitsu/wire/types/RunRecord.ts";
import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { InFlightGeneration } from "../../lib/model/inflight.ts";
import {
  type EvalContext,
  type LiveEvalStatus,
  NO_LIVE_RUNS,
  NO_PROGRESS,
  projectFinishMs,
  useNow,
} from "../../lib/api/liveEval.ts";
import { formatSpan, formatTime } from "../../lib/format/format.ts";
import { useDocumentTitle } from "../../lib/nav/useDocumentTitle.ts";
import { useLocation } from "../../lib/nav/historyContext.ts";
import { EvalRouteContext } from "./evalContext.ts";
import { Dot } from "../../components/primitives.tsx";
import { FrameNav } from "../../components/FrameNav.tsx";

/// The eval frame: a package of many runs, whether loaded from a file or folded live from a running
/// harness. The header is shared by both inner screens — the Scenarios overview and a single run's
/// deep views, rendered as `children` — carrying the source (a file name, or a live badge and
/// progress), model, commit, and date. The package itself flows to the inner screen as the eval
/// context, so a run screen can resolve its scenario and run from the location against it.
export function EvalFrame({
  pkg,
  fileName,
  live,
  liveRuns,
  progress,
  getRun,
  onClose,
  children,
}: {
  pkg: PackageSummary;
  fileName?: string | null;
  live?: LiveEvalStatus;
  liveRuns?: ReadonlyMap<string, Event[]>;
  progress?: ReadonlyMap<string, ReadonlyMap<string, InFlightGeneration>>;
  /// Fetch one run's full record for the deep-dive — over the harness's run endpoint for a live eval,
  /// or synchronously from the retained full package for a file-loaded one.
  getRun: (scenario: number, run: number) => Promise<RunRecord>;
  onClose: () => void;
  /// The inner screen — the scenario overview or a single run's deep views — carried the eval context.
  children: ReactNode;
}) {
  const location = useLocation();
  const context: EvalContext = {
    pkg,
    liveRuns: liveRuns ?? NO_LIVE_RUNS,
    live: live ?? null,
    progress: progress ?? NO_PROGRESS,
    getRun,
  };
  // The open run names the view for the document title (absent on the overview); the scenario and run
  // themselves are legible in the frame's own rail, summary, and run picker, so no breadcrumb.
  const view =
    location.kind === "stream" && location.frame.kind === "evalRun"
      ? location.stream.view
      : undefined;
  useDocumentTitle("eval", view);

  return (
    <div className="mx-auto flex min-h-screen max-w-304 flex-col px-4 sm:px-8">
      <header className="border-b border-line py-4 sm:py-6">
        <div className="flex items-baseline justify-between gap-3">
          <div className="flex min-w-0 items-baseline gap-3">
            <span className="font-serif text-xl text-ink">zuihitsu</span>
            <FrameNav current="eval" />
          </div>
          <div className="flex shrink-0 items-baseline gap-3 font-mono text-xs text-ink-soft">
            <span className="hidden items-baseline gap-3 whitespace-nowrap sm:flex">
              {live ? (
                <>
                  <LiveBadge status={live} />
                  <Dot />
                </>
              ) : (
                fileName && (
                  <>
                    <span className="max-w-56 truncate text-ink" title={fileName}>
                      {fileName}
                    </span>
                    <Dot />
                  </>
                )
              )}
              <span className="max-w-[16rem] truncate">{pkg.meta.model_id}</span>
              <Dot />
              {pkg.meta.git_sha && (
                <>
                  <span>{pkg.meta.git_sha.slice(0, 7)}</span>
                  <Dot />
                </>
              )}
              {live ? (
                <>
                  <LiveProgress pkg={pkg} />
                  <Dot />
                  <LiveTiming pkg={pkg} status={live} />
                </>
              ) : (
                <>
                  <span>{new Date(pkg.meta.finished_at_ms).toISOString()}</span>
                  <Provenance pkg={pkg} />
                </>
              )}
            </span>
            <button
              onClick={onClose}
              className="ml-1 shrink-0 text-ink-faint transition-colors hover:text-clay"
              title="Close this package"
            >
              ✕
            </button>
          </div>
        </div>

        {/* On mobile the source drops to a quieter second row. */}
        <div className="mt-2 flex items-baseline justify-end font-mono text-2xs text-ink-soft sm:hidden">
          <span className="truncate text-ink-faint">{fileName ?? pkg.meta.model_id}</span>
        </div>
      </header>

      <EvalRouteContext.Provider value={context}>{children}</EvalRouteContext.Provider>
    </div>
  );
}

/// A package's replay provenance, beside the git sha as quiet metadata: `rejudged from <source>` when
/// a `replay --mode rejudge` re-assessed another package, and `resumed from <package> · run <run> ·
/// step <step>` when a `replay --mode resume` rewound and redrove a recorded run. Both absent — the
/// common case for a fresh run — renders nothing, so the header does not shift.
function Provenance({ pkg }: { pkg: PackageSummary }) {
  const { rejudged_from, resumed_from } = pkg.meta;
  if (!rejudged_from && !resumed_from) return null;
  return (
    <>
      {rejudged_from && (
        <>
          <Dot />
          <span className="max-w-[20rem] truncate text-ink-faint" title={rejudged_from}>
            rejudged from {rejudged_from}
          </span>
        </>
      )}
      {resumed_from && (
        <>
          <Dot />
          <span
            className="max-w-[24rem] truncate text-ink-faint"
            title={`resumed from ${resumed_from.package} · run ${resumed_from.run} · step ${resumed_from.step}`}
          >
            resumed from {resumed_from.package} · run {resumed_from.run} · step {resumed_from.step}
          </span>
        </>
      )}
    </>
  );
}

/// The live source's state, in place of a file name: a sage pulse while runs stream, a quiet dot once
/// the run is complete (the harness keeps serving), or a clay note while a dropped stream reconnects.
function LiveBadge({ status }: { status: LiveEvalStatus }) {
  const tone =
    status.status === "error"
      ? { dot: "bg-clay", text: "text-clay", label: "reconnecting", pulse: false }
      : status.status === "finished"
        ? { dot: "bg-ink-faint", text: "text-ink-soft", label: "complete", pulse: false }
        : { dot: "bg-sage", text: "text-sage", label: "live", pulse: true };
  return (
    <span className="flex items-baseline gap-1.5">
      <span className="relative flex size-1.5 self-center">
        {tone.pulse && (
          <span className="absolute inline-flex size-full animate-ping rounded-full bg-sage opacity-60" />
        )}
        <span className={"relative inline-flex size-1.5 rounded-full " + tone.dot} />
      </span>
      <span className={"tracking-widest uppercase " + tone.text}>{tone.label}</span>
    </span>
  );
}

/// How far a live run has got: completed runs across every scenario, over the planned total.
function LiveProgress({ pkg }: { pkg: PackageSummary }) {
  const done = pkg.scenarios.reduce((sum, scenario) => sum + scenario.runs.length, 0);
  const total = pkg.scenarios.length * pkg.meta.runs_per_scenario;
  return (
    <span title="completed runs of the planned total">
      {done}/{total} runs
    </span>
  );
}

/// The live run's clock, quiet beside the progress: elapsed since it began, and — once a scenario has
/// completed a run to extrapolate from — a projected local finish time (`~HH:MM`). Once the harness
/// reports `finished`, the projection gives way to the run's total duration. Ticks every 30s while
/// streaming; frozen once finished.
function LiveTiming({ pkg, status }: { pkg: PackageSummary; status: LiveEvalStatus }) {
  const finished = status.status === "finished";
  const now = useNow(30_000, !finished);
  const started = pkg.meta.started_at_ms;

  if (finished) {
    return (
      <span className="font-mono text-2xs text-ink-faint" title="total run duration">
        {formatSpan(pkg.meta.finished_at_ms - started)} total
      </span>
    );
  }

  const projected = projectFinishMs(pkg, now);
  return (
    <span className="flex items-baseline gap-3 font-mono text-2xs text-ink-faint">
      <span title="elapsed since the run began">{formatSpan(Math.max(0, now - started))}</span>
      {projected !== null && (
        <span title="projected completion (local time)">~{formatTime(projected)}</span>
      )}
    </span>
  );
}
