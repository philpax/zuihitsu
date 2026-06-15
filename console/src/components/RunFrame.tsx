import { Link, Navigate, useOutletContext, useParams } from "react-router-dom";

import type { EvalPackage } from "../types/EvalPackage.ts";
import type { ScenarioReport } from "../types/ScenarioReport.ts";
import type { RunRecord } from "../types/RunRecord.ts";
import { type ReplicaState, useReplica } from "../lib/useReplica.ts";
import { runBase, runPath } from "../lib/routes.ts";
import { useStreamLocation } from "../lib/useStreamLocation.ts";
import { STREAM_VIEWS } from "../lib/streamViews.ts";
import { Eyebrow } from "./primitives.tsx";
import { StreamWorkspace } from "./StreamWorkspace.tsx";
import { VerdictPanel } from "./VerdictPanel.tsx";

/// A single run's deep views, resolved from the URL: `:scenario` (name) and `:run` (index) pick the
/// run out of the package, `:view` selects the view, and `?seq` pins the timeline cursor. Folding the
/// run's embedded log into a [`Replica`] and handing it to the shared [`StreamWorkspace`] — driven
/// from the URL through [`useStreamLocation`], exactly as the agent frame drives it — is what makes
/// browser back and forward step through views and timeline positions. A scenario, run, or view the
/// URL names but the package does not hold redirects back. A rail to the side keeps every scenario one
/// click away (and every run of the open one), so inspecting the suite does not mean trips back to the
/// overview.
export function RunFrame() {
  const pkg = useOutletContext<EvalPackage>();
  const params = useParams();
  const scenario = pkg.scenarios.find((entry) => entry.meta.name === params.scenario) ?? null;
  const run = scenario?.runs.find((entry) => String(entry.index) === params.run) ?? null;
  const replica = useReplica(run?.events ?? null);
  const { view, seq, selectView, setSeq } = useStreamLocation(
    scenario && run ? runBase(scenario.meta.name, run.index) : "",
  );

  if (!scenario || !run) return <Navigate to="/eval" replace />;
  if (!STREAM_VIEWS.some((entry) => entry.id === view)) {
    return <Navigate to={runPath(scenario.meta.name, run.index)} replace />;
  }

  const ready = replica.status === "ready" ? replica.replica : null;

  return (
    <div className="flex flex-1 gap-6">
      <ScenarioRail pkg={pkg} scenario={scenario} run={run} />
      <div className="flex min-w-0 flex-1 flex-col">
        <VerdictPanel run={run} />
        {!ready ? (
          <Pending state={replica} />
        ) : (
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
        )}
      </div>
    </div>
  );
}

/// The scenario switcher: every scenario in the package as a name, the open one marked, with its runs
/// fanned out beneath it as a run switcher. A clay tick flags a scenario whose bar did not hold, so
/// the rail doubles as the overview's health at a glance. Hidden below `lg`, where the transcript and
/// its own sidebar already want the width; the header breadcrumb covers navigation there.
function ScenarioRail({
  pkg,
  scenario,
  run,
}: {
  pkg: EvalPackage;
  scenario: ScenarioReport;
  run: RunRecord;
}) {
  return (
    <aside className="hidden w-40 shrink-0 lg:block">
      <div className="sticky top-4 flex flex-col">
        <Eyebrow>scenarios</Eyebrow>
        <nav className="mt-3 flex flex-col gap-0.5">
          {pkg.scenarios.map((entry) => {
            const active = entry.meta.name === scenario.meta.name;
            return (
              <div key={entry.meta.name}>
                <Link
                  to={runPath(entry.meta.name, entry.runs[0].index)}
                  title={entry.meta.name}
                  className={
                    "-ml-3 flex min-w-0 items-baseline gap-1.5 border-l-2 py-1 pl-2.5 font-mono text-2xs transition-colors " +
                    (active
                      ? "border-clay text-ink"
                      : "border-transparent text-ink-soft hover:text-ink")
                  }
                >
                  {!held(entry) && <span className="shrink-0 text-clay">●</span>}
                  <span className="truncate">{entry.meta.name}</span>
                </Link>
                {active && entry.runs.length > 1 && (
                  <div className="mb-1 ml-1 mt-0.5 flex flex-wrap gap-1.5">
                    {entry.runs.map((entryRun) => (
                      <Link
                        key={entryRun.index}
                        to={runPath(entry.meta.name, entryRun.index)}
                        title={`Run ${entryRun.index}`}
                        className={
                          "font-mono text-2xs transition-colors " +
                          (entryRun.index === run.index
                            ? "text-clay"
                            : "text-ink-faint hover:text-ink")
                        }
                      >
                        {entryRun.index}
                      </Link>
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </nav>
      </div>
    </aside>
  );
}

/// Whether a scenario's bar held — the gate for a gating scenario, the rate against the threshold for
/// a metric one. Mirrors the overview's judgement, so the rail's marks and the overview's agree.
function held(scenario: ScenarioReport): boolean {
  const { meta, aggregate } = scenario;
  return meta.bar.kind === "gating"
    ? aggregate.gating_passed
    : aggregate.rate >= meta.bar.threshold;
}

function Pending({ state }: { state: ReplicaState }) {
  const error = state.status === "error";
  const message = error ? `Could not fold the log — ${state.message}` : "Folding the event log…";
  return (
    <div className={"flex-1 py-16 text-center text-sm " + (error ? "text-clay" : "text-ink-faint")}>
      {message}
    </div>
  );
}
