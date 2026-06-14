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
    <div className="mx-auto flex min-h-screen max-w-[76rem] flex-col px-8">
      <header className="flex items-baseline justify-between border-b border-line py-6">
        <div className="flex items-baseline gap-3">
          <span className="font-serif text-xl text-ink">zuihitsu</span>
          <Eyebrow>console · eval</Eyebrow>
          {activeRun && (
            <button
              onClick={() => setActiveRun(null)}
              className="ml-1 flex items-baseline gap-2 font-mono text-xs text-ink-soft transition-colors hover:text-clay"
              title="Back to the package"
            >
              <span className="text-ink-faint">/</span>
              <span>{activeRun.scenario.meta.name}</span>
              <span className="text-ink-faint">· run {activeRun.run.index}</span>
            </button>
          )}
        </div>
        <div className="flex items-baseline gap-3 font-mono text-xs text-ink-soft">
          <span>{pkg.meta.model_id}</span>
          <Dot />
          {pkg.meta.git_sha && (
            <>
              <span>{pkg.meta.git_sha.slice(0, 7)}</span>
              <Dot />
            </>
          )}
          <span>{formatDate(pkg.meta.finished_at_ms)}</span>
          <button
            onClick={onClose}
            className="ml-1 text-ink-faint transition-colors hover:text-clay"
            title="Close this package"
          >
            ✕
          </button>
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
