import { useState, type ReactNode } from "react";

import type { EvalPackage } from "../types/EvalPackage.ts";
import type { ActiveRun } from "../lib/run.ts";
import type { Replica } from "../lib/replica.ts";
import { type ReplicaState, useReplica } from "../lib/useReplica.ts";
import { type ViewId } from "../lib/views.ts";
import { formatDate } from "../lib/format.ts";
import { Dot, Eyebrow } from "./primitives.tsx";
import { Nav } from "./Nav.tsx";
import { ScenarioOverview } from "../views/ScenarioOverview.tsx";
import { StateView } from "../views/StateView.tsx";
import { ConversationView } from "../views/ConversationView.tsx";
import { EventsView } from "../views/EventsView.tsx";

/// The loaded-package frame: a header naming the package and the run in focus, the view nav, and the
/// active view. Run-scoped views fold the selected run's log into a [`Replica`] once and share it.
export function Shell({ pkg, onClose }: { pkg: EvalPackage; onClose: () => void }) {
  const [view, setView] = useState<ViewId>("scenarios");
  const [activeRun, setActiveRun] = useState<ActiveRun | null>(null);
  const replica = useReplica(activeRun?.run.events ?? null);

  function selectRun(next: ActiveRun) {
    setActiveRun(next);
    setView("state");
  }

  function clearRun() {
    setActiveRun(null);
    setView("scenarios");
  }

  return (
    <div className="mx-auto flex min-h-screen max-w-[76rem] flex-col px-8">
      <header className="flex items-baseline justify-between border-b border-line py-6">
        <div className="flex items-baseline gap-3">
          <span className="font-serif text-xl text-ink">zuihitsu</span>
          <Eyebrow>console</Eyebrow>
          {activeRun && (
            <button
              onClick={clearRun}
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

      <Nav view={view} onSelect={setView} runActive={!!activeRun} />

      <main className="flex-1 py-10">
        {view === "scenarios" && <ScenarioOverview pkg={pkg} onSelectRun={selectRun} />}
        {view === "state" && (
          <RunScoped state={replica}>{(ready) => <StateView replica={ready} />}</RunScoped>
        )}
        {view === "conversation" && (
          <RunScoped state={replica}>
            {(ready) => <ConversationView replica={ready} events={activeRun!.run.events} />}
          </RunScoped>
        )}
        {view === "events" && (
          <RunScoped state={replica}>
            {(ready) => <EventsView replica={ready} events={activeRun!.run.events} />}
          </RunScoped>
        )}
      </main>
    </div>
  );
}

/// Gate a run-scoped view on its replica: prompt when no run is chosen, reassure while the log
/// folds, surface a failure, and otherwise hand the ready replica to the view.
function RunScoped({
  state,
  children,
}: {
  state: ReplicaState;
  children: (replica: Replica) => ReactNode;
}) {
  switch (state.status) {
    case "idle":
      return <Placeholder>Select a run from Scenarios to inspect its state.</Placeholder>;
    case "loading":
      return <Placeholder>Folding the event log…</Placeholder>;
    case "error":
      return <Placeholder tone="error">Could not fold the log — {state.message}</Placeholder>;
    case "ready":
      return <>{children(state.replica)}</>;
  }
}

function Placeholder({ children, tone }: { children: ReactNode; tone?: "error" }) {
  return (
    <div
      className={"py-24 text-center text-sm " + (tone === "error" ? "text-clay" : "text-ink-faint")}
    >
      {children}
    </div>
  );
}
