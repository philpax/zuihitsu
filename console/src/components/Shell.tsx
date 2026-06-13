import { useState, type ReactNode } from "react";

import type { EvalPackage } from "../types/EvalPackage.ts";
import type { ActiveRun } from "../lib/run.ts";
import type { Replica } from "../lib/replica.ts";
import { type ReplicaState, useReplica } from "../lib/useReplica.ts";
import { type ViewId } from "../lib/views.ts";
import { formatDate } from "../lib/format.ts";
import { Dot, Eyebrow } from "./primitives.tsx";
import { Nav } from "./Nav.tsx";
import { Timeline } from "./Timeline.tsx";
import { ScenarioOverview } from "../views/ScenarioOverview.tsx";
import { StateView } from "../views/StateView.tsx";
import { ConversationView } from "../views/ConversationView.tsx";
import { EventsView } from "../views/EventsView.tsx";

/// The loaded-package frame: a header naming the package and the run in focus, the view nav, the
/// active view, and — for the run-scoped views — a global timeline. The selected run's log folds
/// into a shared [`Replica`]; one seq cursor drives every run-scoped view, so scrubbing the timeline
/// folds the State graph and stops the Conversation and Events at the same point.
export function Shell({ pkg, onClose }: { pkg: EvalPackage; onClose: () => void }) {
  const [view, setView] = useState<ViewId>("scenarios");
  const [activeRun, setActiveRun] = useState<ActiveRun | null>(null);
  // null means "the head" — the latest state. A number pins the cursor to an earlier seq.
  const [seq, setSeq] = useState<number | null>(null);
  const replica = useReplica(activeRun?.run.events ?? null);

  const ready = replica.status === "ready" ? replica.replica : null;
  const head = ready?.headSeq ?? 0;
  const cursor = seq ?? head;
  const runScoped = view !== "scenarios";

  function selectRun(next: ActiveRun) {
    setActiveRun(next);
    setSeq(null);
    setView("state");
  }

  function clearRun() {
    setActiveRun(null);
    setSeq(null);
    setView("scenarios");
  }

  function scrub(next: number) {
    ready?.foldTo(next);
    setSeq(next >= head ? null : next);
  }

  function reset() {
    ready?.foldTo(head);
    setSeq(null);
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
          <RunScoped state={replica}>{(r) => <StateView replica={r} cursor={cursor} />}</RunScoped>
        )}
        {view === "conversation" && (
          <RunScoped state={replica}>
            {(r) => <ConversationView replica={r} events={activeRun!.run.events} cursor={cursor} />}
          </RunScoped>
        )}
        {view === "events" && (
          <RunScoped state={replica}>
            {(r) => <EventsView replica={r} events={activeRun!.run.events} cursor={cursor} />}
          </RunScoped>
        )}
      </main>

      {runScoped && ready && activeRun && (
        <Timeline
          head={head}
          seq={cursor}
          events={activeRun.run.events}
          onScrub={scrub}
          onReset={reset}
        />
      )}
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
