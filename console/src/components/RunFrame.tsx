import { Link, Navigate, useOutletContext, useParams } from "react-router-dom";

import type { EvalPackage } from "../types/EvalPackage.ts";
import type { ScenarioReport } from "../types/ScenarioReport.ts";
import type { Event } from "../types/Event.ts";
import { type EvalContext, liveRunOf, runningKey } from "../lib/liveEval.ts";
import { type ReplicaState, useReplica } from "../lib/useReplica.ts";
import { runBase, runPath } from "../lib/routes.ts";
import { useStreamLocation } from "../lib/useStreamLocation.ts";
import { STREAM_VIEWS } from "../lib/streamViews.ts";
import { formatMs, formatRate, formatTokenSplit } from "../lib/format.ts";
import { Dot, Eyebrow } from "./primitives.tsx";
import { StreamWorkspace } from "./StreamWorkspace.tsx";
import { VerdictPanel } from "./VerdictPanel.tsx";

/// A single run's deep views, resolved from the URL: `:scenario` (name) and `:run` (index) pick the
/// run out of the package, `:view` selects the view, and `?seq` pins the timeline cursor. Folding the
/// run's embedded log into a [`Replica`] and handing it to the shared [`StreamWorkspace`] — driven
/// from the URL through [`useStreamLocation`], exactly as the agent frame drives it — is what makes
/// browser back and forward step through views and timeline positions. A scenario, run, or view the
/// URL names but the package does not hold redirects back.
///
/// The layout reads as a drill-down: the scenario list on the left, then the scenario's summary, the
/// run picker, this run's verdicts, and finally the run's views — outer scope to inner, top to bottom.
export function RunFrame() {
  const { pkg, liveRuns } = useOutletContext<EvalContext>();
  const params = useParams();
  const scenarioIndex = pkg.scenarios.findIndex((entry) => entry.meta.name === params.scenario);
  const scenario = scenarioIndex >= 0 ? pkg.scenarios[scenarioIndex] : null;
  const completed = scenario?.runs.find((entry) => String(entry.index) === params.run) ?? null;
  // A run still driving has no record yet; its events stream into the live map until it completes, so
  // a deep-dive opened on it watches the deliberation arrive. Once its `RunCompleted` lands, the record
  // takes over (verdicts appear, the conversation stays).
  const live =
    scenario && params.run !== undefined && !completed
      ? (liveRuns.get(runningKey(scenarioIndex, Number(params.run))) ?? null)
      : null;
  const events = completed?.events ?? live;
  const replica = useReplica(events);
  const runIndex = completed
    ? completed.index
    : params.run !== undefined
      ? Number(params.run)
      : null;
  const { view, seq, selectView, setSeq } = useStreamLocation(
    scenario && runIndex !== null ? runBase(scenario.meta.name, runIndex) : "",
  );

  if (!scenario || runIndex === null || !events) return <Navigate to="/eval" replace />;
  if (!STREAM_VIEWS.some((entry) => entry.id === view)) {
    return <Navigate to={runPath(scenario.meta.name, runIndex)} replace />;
  }

  const ready = replica.status === "ready" ? replica.replica : null;

  // Distinct keys per sibling: the panel and the workspace both reset per run, but they must not
  // share a key — duplicate keys among siblings break reconciliation, leaving stale panels mounted.
  const runKey = `${scenario.meta.name}/${runIndex}`;

  return (
    <div className="flex flex-1 gap-6 pt-7">
      <ScenarioRail pkg={pkg} active={scenario.meta.name} liveRuns={liveRuns} view={view!} />
      <div className="flex min-w-0 flex-1 flex-col">
        <ScenarioSummary scenario={scenario} />
        <RunPicker
          scenario={scenario}
          active={runIndex}
          liveRun={liveRunOf(liveRuns, scenarioIndex)}
          view={view!}
        />
        {completed && (
          <VerdictPanel
            key={`verdict:${runKey}`}
            run={completed}
            gating={scenario.meta.bar.kind === "gating"}
          />
        )}
        {!ready ? (
          <Pending state={replica} />
        ) : (
          <StreamWorkspace
            key={`stream:${runKey}`}
            replica={ready}
            events={events}
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

/// The scenario switcher: every scenario in the package as a name, the open one marked, a clay tick
/// flagging any whose bar did not hold — so the rail doubles as the overview's health at a glance.
/// Hidden below `lg`, where the views want the width and the header breadcrumb covers navigation.
function ScenarioRail({
  pkg,
  active,
  liveRuns,
  view,
}: {
  pkg: EvalPackage;
  active: string;
  liveRuns: ReadonlyMap<string, Event[]>;
  view: string;
}) {
  return (
    <aside className="hidden w-40 shrink-0 lg:block">
      <div className="sticky top-4 flex flex-col">
        <Eyebrow>scenarios</Eyebrow>
        <nav className="mt-3 flex flex-col gap-0.5">
          {pkg.scenarios.map((entry, index) => {
            const isActive = entry.meta.name === active;
            const liveIndex = liveRunOf(liveRuns, index);
            const first = entry.runs[0];
            // Open the first completed run, or — if none has landed — the one driving live.
            const openRun = first ? first.index : liveIndex;
            const tint = isActive
              ? "border-clay text-ink"
              : "border-transparent text-ink-soft hover:text-ink";
            const rowClass =
              "-ml-3 flex min-w-0 items-baseline gap-1.5 border-l-2 py-1 pl-2.5 font-mono text-2xs transition-colors " +
              tint;
            const marker =
              liveIndex !== null ? (
                <span className="shrink-0 text-sage">●</span>
              ) : first && !held(entry) ? (
                <span className="shrink-0 text-clay">●</span>
              ) : null;
            // Not started and not driving: a quiet, non-clickable row until a run lands or begins.
            return openRun !== null ? (
              <Link
                key={entry.meta.name}
                to={runPath(entry.meta.name, openRun, view)}
                title={entry.meta.name}
                className={rowClass}
              >
                {marker}
                <span className="truncate">{entry.meta.name}</span>
              </Link>
            ) : (
              <span
                key={entry.meta.name}
                title={entry.meta.name}
                className={rowClass + " opacity-60"}
              >
                {marker}
                <span className="truncate">{entry.meta.name}</span>
              </span>
            );
          })}
        </nav>
      </div>
    </aside>
  );
}

/// The open scenario's headline — the eval summary you have drilled into: its name and category, the
/// aggregate pass rate and whether the bar held, the typical latency and cost, and the description.
function ScenarioSummary({ scenario }: { scenario: ScenarioReport }) {
  const { meta, aggregate } = scenario;
  const threshold = meta.bar.kind === "metric" ? meta.bar.threshold : null;

  return (
    <header className="border-b border-line pb-4">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <h2 className="font-serif text-xl text-ink sm:text-2xl">{meta.name}</h2>
        <span className="font-mono text-2xs uppercase tracking-widest text-ink-faint">
          {meta.category}
        </span>
      </div>
      <div className="mt-2 flex flex-wrap items-baseline gap-3 font-mono text-2xs text-ink-faint">
        <span className="text-sm text-ink">{formatRate(aggregate.rate)}</span>
        <span className={held(scenario) ? "text-sage" : "text-clay"}>
          {meta.bar.kind === "gating"
            ? aggregate.gating_passed
              ? "gating · held"
              : "gating · regressed"
            : `metric ≥ ${formatRate(threshold!)}`}
        </span>
        <Dot />
        <span>
          {aggregate.runs} run{aggregate.runs > 1 ? "s" : ""}
        </span>
        <Dot />
        <span>p50 {formatMs(aggregate.latency_ms.p50)}</span>
        <Dot />
        <span>
          {formatTokenSplit(aggregate.tokens.prompt_mean, aggregate.tokens.completion_mean)}
        </span>
      </div>
      {meta.description && (
        <p className="mt-3 max-w-prose text-sm leading-relaxed text-ink-soft">{meta.description}</p>
      )}
    </header>
  );
}

/// The runs of the open scenario, laid out as a horizontal row beneath the summary so the drill-down
/// reads top to bottom. The open run is marked; a run whose gate regressed shows in clay; the one
/// driving live (not yet a completed record) shows last, in sage, so any in-flight run is reachable.
function RunPicker({
  scenario,
  active,
  liveRun,
  view,
}: {
  scenario: ScenarioReport;
  active: number;
  liveRun: number | null;
  view: string;
}) {
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-line py-3">
      <Eyebrow>runs</Eyebrow>
      <nav className="flex flex-wrap gap-1.5">
        {scenario.runs.map((run) => {
          const isActive = run.index === active;
          // A run is clean only if the gate held *and* every criterion passed — a metric-only eval
          // gates nothing, so its failures show only in the verdicts, not in gating_passed.
          const passed =
            run.metrics.gating_passed && run.verdicts.every((verdict) => verdict.passed);
          // A regressed run reads in clay (border, tint, and text); the open one is filled.
          const tone = isActive
            ? passed
              ? "border-clay bg-clay-soft/25 text-ink "
              : "border-clay bg-clay-soft/40 text-clay "
            : passed
              ? "border-line text-ink-soft hover:border-ink-faint "
              : "border-clay/50 bg-clay-soft/15 text-clay hover:border-clay ";
          return (
            <Link
              key={run.index}
              to={runPath(scenario.meta.name, run.index, view)}
              title={`Run ${run.index} · ${passed ? "passed" : "failed"}`}
              className={
                "flex h-7 min-w-[1.75rem] items-center justify-center border px-1.5 font-mono text-2xs transition-colors " +
                tone
              }
            >
              {run.index}
            </Link>
          );
        })}
        {liveRun !== null && (
          <Link
            to={runPath(scenario.meta.name, liveRun, view)}
            title={`Run ${liveRun} · streaming live`}
            className={
              "flex h-7 min-w-[1.75rem] items-center justify-center gap-1.5 border px-1.5 font-mono text-2xs transition-colors " +
              (liveRun === active
                ? "border-sage bg-sage/15 text-sage "
                : "border-sage/50 text-sage hover:border-sage ")
            }
          >
            <span className="relative flex h-1 w-1">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-sage opacity-70" />
              <span className="relative inline-flex h-1 w-1 rounded-full bg-sage" />
            </span>
            {liveRun}
          </Link>
        )}
      </nav>
    </div>
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
