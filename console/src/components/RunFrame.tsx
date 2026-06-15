import { Navigate, useOutletContext, useParams } from "react-router-dom";

import type { EvalPackage } from "../types/EvalPackage.ts";
import { type ReplicaState, useReplica } from "../lib/useReplica.ts";
import { runBase } from "../lib/routes.ts";
import { useStreamLocation } from "../lib/useStreamLocation.ts";
import { STREAM_VIEWS } from "../lib/streamViews.ts";
import { StreamWorkspace } from "./StreamWorkspace.tsx";

/// A single run's deep views, resolved from the URL: `:scenario` (name) and `:run` (index) pick the
/// run out of the package, `:view` selects the view, and `?seq` pins the timeline cursor. Folding the
/// run's embedded log into a [`Replica`] and handing it to the shared [`StreamWorkspace`] — driven
/// from the URL through [`useStreamLocation`], exactly as the agent frame drives it — is what makes
/// browser back and forward step through views and timeline positions. A scenario, run, or view the
/// URL names but the package does not hold redirects back.
export function RunFrame() {
  const pkg = useOutletContext<EvalPackage>();
  const params = useParams();
  const scenario = pkg.scenarios.find((entry) => entry.meta.name === params.scenario) ?? null;
  const run = scenario?.runs.find((entry) => String(entry.index) === params.run) ?? null;
  const replica = useReplica(run?.events ?? null);
  const base = scenario && run ? runBase(scenario.meta.name, run.index) : "";
  const { view, seq, selectView, setSeq } = useStreamLocation(base);

  if (!scenario || !run) return <Navigate to="/eval" replace />;
  if (!STREAM_VIEWS.some((entry) => entry.id === view)) {
    return <Navigate to={`${base}/conversation`} replace />;
  }

  const ready = replica.status === "ready" ? replica.replica : null;
  if (!ready) return <Pending state={replica} />;

  return (
    <StreamWorkspace
      key={`${scenario.meta.name}-${run.index}`}
      replica={ready}
      events={run.events}
      head={ready.headSeq}
      view={view!}
      onSelectView={selectView}
      seq={seq}
      onSeq={setSeq}
    />
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
