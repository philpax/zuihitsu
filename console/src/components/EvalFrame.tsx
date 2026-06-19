import { Link, Outlet, useMatch } from "react-router-dom";

import type { EvalPackage } from "../types/EvalPackage.ts";
import type { Event } from "../types/Event.ts";
import { type EvalContext, type LiveEvalStatus, NO_LIVE_RUNS } from "../lib/liveEval.ts";
import { formatDate } from "../lib/format.ts";
import { Dot } from "./primitives.tsx";
import { FrameNav } from "./FrameNav.tsx";

/// The eval frame: a package of many runs, whether loaded from a file or folded live from a running
/// harness. The header is shared by every nested route — the Scenarios overview at the index, and a
/// single run's deep views below it — carrying the source (a file name, or a live badge and progress),
/// model, commit, and date, with a breadcrumb back to the overview that shows while a run is open.
/// The package itself flows to the nested routes as the outlet context, so a run route can resolve
/// its scenario and run from the URL against it.
export function EvalFrame({
  pkg,
  fileName,
  live,
  liveRuns,
  onClose,
}: {
  pkg: EvalPackage;
  fileName?: string | null;
  live?: LiveEvalStatus;
  liveRuns?: ReadonlyMap<string, Event[]>;
  onClose: () => void;
}) {
  const context: EvalContext = { pkg, liveRuns: liveRuns ?? NO_LIVE_RUNS };
  // The active run, if any, drives the breadcrumb. `:run` is the run index; `:scenario` its name.
  const runMatch = useMatch("/eval/:scenario/:run/:view");
  const crumb = runMatch
    ? { scenario: runMatch.params.scenario ?? "", run: runMatch.params.run ?? "" }
    : null;

  return (
    <div className="mx-auto flex min-h-screen max-w-[76rem] flex-col px-4 sm:px-8">
      <header className="border-b border-line py-4 sm:py-6">
        <div className="flex items-baseline justify-between gap-3">
          <div className="flex min-w-0 items-baseline gap-3">
            <span className="font-serif text-xl text-ink">zuihitsu</span>
            <FrameNav current="eval" />
            {crumb && (
              <Link
                to="/eval"
                className="ml-1 hidden min-w-0 items-baseline gap-2 font-mono text-xs text-ink-soft transition-colors hover:text-clay sm:flex"
                title="Back to the package"
              >
                <span className="text-ink-faint">/</span>
                <span className="truncate">{crumb.scenario}</span>
                <span className="shrink-0 text-ink-faint">· run {crumb.run}</span>
              </Link>
            )}
          </div>
          <div className="flex items-baseline gap-3 font-mono text-xs text-ink-soft">
            <span className="hidden items-baseline gap-3 sm:flex">
              {live ? (
                <>
                  <LiveBadge status={live} />
                  <Dot />
                </>
              ) : (
                fileName && (
                  <>
                    <span className="max-w-[14rem] truncate text-ink" title={fileName}>
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
                <LiveProgress pkg={pkg} />
              ) : (
                <span>{formatDate(pkg.meta.finished_at_ms)}</span>
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

        {/* On mobile the run breadcrumb and model drop to a quieter second row. */}
        <div className="mt-2 flex items-baseline justify-between gap-3 font-mono text-2xs text-ink-soft sm:hidden">
          {crumb ? (
            <Link
              to="/eval"
              className="flex min-w-0 items-baseline gap-2 transition-colors hover:text-clay"
            >
              <span className="text-ink-faint">/</span>
              <span className="truncate">{crumb.scenario}</span>
              <span className="shrink-0 text-ink-faint">· run {crumb.run}</span>
            </Link>
          ) : (
            <span />
          )}
          <span className="shrink-0 truncate text-ink-faint">{fileName ?? pkg.meta.model_id}</span>
        </div>
      </header>

      <Outlet context={context} />
    </div>
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
      <span className="relative flex h-1.5 w-1.5 self-center">
        {tone.pulse && (
          <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-sage opacity-60" />
        )}
        <span className={"relative inline-flex h-1.5 w-1.5 rounded-full " + tone.dot} />
      </span>
      <span className={"uppercase tracking-widest " + tone.text}>{tone.label}</span>
    </span>
  );
}

/// How far a live run has got: completed runs across every scenario, over the planned total.
function LiveProgress({ pkg }: { pkg: EvalPackage }) {
  const done = pkg.scenarios.reduce((sum, scenario) => sum + scenario.runs.length, 0);
  const total = pkg.scenarios.length * pkg.meta.runs_per_scenario;
  return (
    <span title="completed runs of the planned total">
      {done}/{total} runs
    </span>
  );
}
