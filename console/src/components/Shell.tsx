import { useState } from "react";

import type { EvalPackage } from "../types/EvalPackage.ts";
import type { ActiveRun } from "../lib/run.ts";
import { type ReplicaState, useReplica } from "../lib/useReplica.ts";
import { formatDate } from "../lib/format.ts";
import { Dot, Eyebrow } from "./primitives.tsx";
import { StreamWorkspace } from "./StreamWorkspace.tsx";
import { ScenarioOverview } from "../views/ScenarioOverview.tsx";

/// The eval frame: a loaded package of many runs. The Scenarios overview is the run picker; opening
/// a run folds its embedded log into a [`Replica`] and hands it to the shared [`StreamWorkspace`] —
/// the same views, timeline, and room switcher the live agent frame uses, just sourced from a file
/// and scoped to the one run in focus. A breadcrumb returns to the package, the way the agent frame
/// disconnects.
export function Shell({ pkg, onClose }: { pkg: EvalPackage; onClose: () => void }) {
  const [activeRun, setActiveRun] = useState<ActiveRun | null>(null);
  const replica = useReplica(activeRun?.run.events ?? null);
  const ready = replica.status === "ready" ? replica.replica : null;

  return (
    <div className="mx-auto flex min-h-screen max-w-[76rem] flex-col px-4 sm:px-8">
      <header className="border-b border-line py-4 sm:py-6">
        <div className="flex items-baseline justify-between gap-3">
          <div className="flex min-w-0 items-baseline gap-3">
            <span className="font-serif text-xl text-ink">zuihitsu</span>
            <Eyebrow>console · eval</Eyebrow>
            {activeRun && (
              <button
                onClick={() => setActiveRun(null)}
                className="ml-1 hidden min-w-0 items-baseline gap-2 font-mono text-xs text-ink-soft transition-colors hover:text-clay sm:flex"
                title="Back to the package"
              >
                <span className="text-ink-faint">/</span>
                <span className="truncate">{activeRun.scenario.meta.name}</span>
                <span className="shrink-0 text-ink-faint">· run {activeRun.run.index}</span>
              </button>
            )}
          </div>
          <div className="flex items-baseline gap-3 font-mono text-xs text-ink-soft">
            <span className="hidden max-w-[16rem] truncate sm:inline">{pkg.meta.model_id}</span>
            <span className="hidden items-baseline gap-3 sm:flex">
              <Dot />
              {pkg.meta.git_sha && (
                <>
                  <span>{pkg.meta.git_sha.slice(0, 7)}</span>
                  <Dot />
                </>
              )}
              <span>{formatDate(pkg.meta.finished_at_ms)}</span>
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
          {activeRun ? (
            <button
              onClick={() => setActiveRun(null)}
              className="flex min-w-0 items-baseline gap-2 transition-colors hover:text-clay"
            >
              <span className="text-ink-faint">/</span>
              <span className="truncate">{activeRun.scenario.meta.name}</span>
              <span className="shrink-0 text-ink-faint">· run {activeRun.run.index}</span>
            </button>
          ) : (
            <span />
          )}
          <span className="shrink-0 truncate text-ink-faint">{pkg.meta.model_id}</span>
        </div>
      </header>

      {!activeRun ? (
        <main className="flex-1 py-10">
          <ScenarioOverview pkg={pkg} onSelectRun={setActiveRun} />
        </main>
      ) : !ready ? (
        <Pending state={replica} />
      ) : (
        // Key on the run so the workspace resets its cursor and view when a different run is opened.
        <StreamWorkspace
          key={`${activeRun.scenario.meta.name}-${activeRun.run.index}`}
          replica={ready}
          events={activeRun.run.events}
          head={ready.headSeq}
        />
      )}
    </div>
  );
}

function Pending({ state }: { state: ReplicaState }) {
  const error = state.status === "error";
  const message = error ? `Could not fold the log — ${state.message}` : "Folding the event log…";
  return (
    <div className={"flex-1 py-24 text-center text-sm " + (error ? "text-clay" : "text-ink-faint")}>
      {message}
    </div>
  );
}
