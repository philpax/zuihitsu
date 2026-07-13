import { useState } from "react";
import { Link, Navigate, useOutletContext, useParams } from "react-router-dom";

import type { PackageSummary } from "../../types/PackageSummary.ts";
import type { ScenarioSummary } from "../../types/ScenarioSummary.ts";
import type { Event } from "../../types/Event.ts";
import { type EvalContext, liveRunOf, runningKey } from "../../lib/api/liveEval.ts";
import { type ReplicaState, useReplica } from "../../lib/replica/useReplica.ts";
import { runBase, runPath } from "../../lib/nav/routes.ts";
import { groupScenariosByCategory } from "../../lib/model/scenarioGroups.ts";
import { useStreamLocation } from "../../lib/nav/useStreamLocation.ts";
import { STREAM_VIEWS } from "../../lib/nav/streamViews.ts";
import { formatMs, formatRate, formatTokens, formatTokenSplit } from "../../lib/format/format.ts";
import { Dot, Eyebrow } from "../../components/primitives.tsx";
import { warmthAggregate } from "../../lib/model/contextDebug.ts";
import { StreamWorkspace } from "../../components/StreamWorkspace.tsx";
import { VerdictPanel } from "./VerdictPanel.tsx";
import { useRunRecord, type RunRecordState } from "./useRunRecord.ts";

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
  const { pkg, liveRuns, progress, getRun } = useOutletContext<EvalContext>();
  const params = useParams();
  const scenarioIndex = pkg.scenarios.findIndex((entry) => entry.meta.name === params.scenario);
  const scenario = scenarioIndex >= 0 ? pkg.scenarios[scenarioIndex] : null;
  // The completed run's lean summary — verdicts and metrics render from it immediately; its full event
  // log is fetched separately below. A run still driving has no summary yet.
  const summary = scenario?.runs.find((entry) => String(entry.index) === params.run) ?? null;
  const runIndex =
    summary !== null ? summary.index : params.run !== undefined ? Number(params.run) : null;
  // A run still driving streams its events into the live map until it completes, so a deep-dive opened
  // on it watches the deliberation arrive.
  const liveEvents =
    scenario && params.run !== undefined && !summary
      ? (liveRuns.get(runningKey(scenarioIndex, Number(params.run))) ?? null)
      : null;

  // A completed run's full record (its event log and journal) is fetched on demand — the lean package
  // carries only the summary.
  const recordState = useRunRecord(getRun, scenarioIndex, runIndex, summary !== null);
  const fetched = recordState.status === "ready" ? recordState.record : null;

  // Bridge the handover: when a live run completes, the fold retires its live buffer and the summary
  // lands, but the record fetch is still in flight. Hold the last live events (a render-phase snapshot,
  // React's store-from-previous-render pattern) so the open view keeps rendering the deliberation
  // rather than flashing back to a loading state until the record arrives. Keyed by the run so a
  // sibling run never reads a stale bridge.
  const heldKey = runIndex !== null ? runningKey(scenarioIndex, runIndex) : null;
  const [held, setHeld] = useState<{ key: string; events: Event[] } | null>(null);
  if (liveEvents && heldKey && (held?.key !== heldKey || held.events !== liveEvents)) {
    setHeld({ key: heldKey, events: liveEvents });
  }
  const bridged = held && held.key === heldKey ? held.events : null;

  // The events the workspace folds: the fetched record's log, else the live buffer, else the bridged
  // last-live events during a handover. Null only when the run is unresolved or a completed run's fetch
  // has not landed yet (and no bridge exists) — the latter shows the Pending treatment, not a redirect.
  const events = fetched?.events ?? liveEvents ?? bridged;
  const replica = useReplica(events);
  const { view, seq, selectView, setSeq } = useStreamLocation(
    scenario && runIndex !== null ? runBase(scenario.meta.name, runIndex) : "",
  );

  // The run is unresolved (a bad URL) when there is neither a completed summary nor a driving live run.
  if (!scenario || runIndex === null || (!summary && !liveEvents)) {
    return <Navigate to="/eval" replace />;
  }
  if (!STREAM_VIEWS.some((entry) => entry.id === view)) {
    return <Navigate to={runPath(scenario.meta.name, runIndex)} replace />;
  }

  const ready = replica.status === "ready" ? replica.replica : null;

  // Distinct keys per sibling: the panel and the workspace both reset per run, but they must not
  // share a key — duplicate keys among siblings break reconciliation, leaving stale panels mounted.
  const runKey = `${scenario.meta.name}/${runIndex}`;

  // The resume boundary applies only to the one run a resumed package continued — its journal steps
  // above the boundary are the restored recording. Matched by scenario and run so it never draws on a
  // sibling run of a multi-run package.
  const resumed = pkg.meta.resumed_from;
  const resumedFromStep =
    resumed && summary && resumed.scenario === scenario.meta.name && resumed.run === summary.index
      ? resumed.step
      : null;

  return (
    <div className="flex flex-1 gap-6 pt-7">
      <ScenarioRail pkg={pkg} active={scenario.meta.name} liveRuns={liveRuns} view={view!} />
      <div className="flex min-w-0 flex-1 flex-col">
        <ScenarioHeader scenario={scenario} />
        <RunPicker
          scenario={scenario}
          active={runIndex}
          liveRun={liveRunOf(liveRuns, scenarioIndex)}
          view={view!}
        />
        {summary && (
          <VerdictPanel
            key={`verdict:${runKey}`}
            run={summary}
            gating={scenario.meta.bar.kind !== "metric"}
          />
        )}
        {!ready || !events ? (
          <Pending state={replica} record={recordState} />
        ) : (
          <StreamWorkspace
            key={`stream:${runKey}`}
            replica={ready}
            progress={progress.get(runningKey(scenarioIndex, runIndex))}
            events={events}
            head={ready.headSeq}
            view={view!}
            onSelectView={selectView}
            seq={seq}
            onSeq={setSeq}
            journal={fetched?.journal}
            resumedFromStep={resumedFromStep}
          />
        )}
      </div>
    </div>
  );
}

/// The scenario switcher: every scenario in the package as a name, the open one marked, and — in place
/// of a bare status dot — a compact success rate, color-coded clay (low) to sage (high) along the house
/// palette. A scenario still working (a run driving, or part-way through its planned runs on a live
/// eval) shows its running tally in neutral ink with a sage working pulse instead, a treatment that
/// cannot be read as either end of the rate scale. A regressed gate keeps its own clay mark beside the
/// rate, since a gate can slip at an otherwise healthy rate. Hidden below `lg`, where the views want the
/// width and the header breadcrumb covers navigation.
function ScenarioRail({
  pkg,
  active,
  liveRuns,
  view,
}: {
  pkg: PackageSummary;
  active: string;
  liveRuns: ReadonlyMap<string, Event[]>;
  view: string;
}) {
  const runsPlanned = pkg.meta.runs_per_scenario;
  return (
    <aside className="hidden w-48 shrink-0 lg:block">
      <div className="sticky top-4 flex flex-col">
        <Eyebrow>scenarios</Eyebrow>
        <nav className="mt-3 flex flex-col gap-0.5">
          {groupScenariosByCategory(pkg.scenarios).map((group) => (
            <div key={group.category} className="mb-2 flex flex-col gap-0.5 last:mb-0">
              <span className="pt-1 font-mono text-2xs uppercase tracking-widest text-ink-faint">
                {group.category}
              </span>
              {group.entries.map(({ scenario: entry, index }) => {
                const isActive = entry.meta.name === active;
                const liveIndex = liveRunOf(liveRuns, index);
                const completed = entry.runs.length;
                const first = entry.runs[0];
                // Open the first completed run, or — if none has landed — the one driving live.
                // The driving run wins the row's link — opening a scenario mid-run lands on the
                // action — else the first completed run.
                const openRun = liveIndex ?? (first ? first.index : null);
                // Ongoing: a run is driving now, or the scenario is part-way through its planned runs on a
                // live eval. Its rate is provisional, so the row shows progress, not a percentage.
                const ongoing = liveIndex !== null || (completed > 0 && completed < runsPlanned);
                const tint = isActive
                  ? "border-clay text-ink"
                  : "border-transparent text-ink-soft hover:text-ink";
                const rowClass =
                  "-ml-3 flex min-w-0 items-baseline justify-between gap-1.5 border-l-2 py-1 pl-2.5 font-mono text-2xs transition-colors " +
                  tint;
                const status = ongoing ? (
                  // The working pulse is the console's established in-flight cue (sage); the tally itself
                  // stays neutral ink-faint so it never reads as a point on the clay→sage rate scale.
                  // items-baseline with the dot self-centered: the dot is a baseline-less box, so left to
                  // lead the flex line it would drag the tally off the scenario name's baseline.
                  <span
                    className="flex shrink-0 items-baseline gap-1 text-ink-faint"
                    title="running"
                  >
                    <span className="relative flex h-1 w-1 self-center">
                      <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-sage opacity-70" />
                      <span className="relative inline-flex h-1 w-1 rounded-full bg-sage" />
                    </span>
                    <span>
                      {completed}/{runsPlanned}
                    </span>
                  </span>
                ) : completed > 0 ? (
                  <span className="flex shrink-0 items-baseline gap-1">
                    {(() => {
                      const warmth = warmthAggregate(entry.runs.flatMap((run) => run.usages));
                      return warmth.median !== null && warmth.median < 0.9 ? (
                        <span
                          className="text-clay"
                          title={`median cache warmth ${Math.round(warmth.median * 100)}% — this scenario thrashes the prefix cache`}
                        >
                          {Math.round(warmth.median * 100)}%↻
                        </span>
                      ) : null;
                    })()}
                    {!held(entry) && (
                      <span className="text-clay" title="gate regressed">
                        ●
                      </span>
                    )}
                    <span style={{ color: rateColor(entry.aggregate.rate) }} title="success rate">
                      {formatRate(entry.aggregate.rate)}
                    </span>
                  </span>
                ) : null;
                // Not started and not driving: a quiet, non-clickable row until a run lands or begins.
                return openRun !== null ? (
                  <Link
                    key={entry.meta.name}
                    to={runPath(entry.meta.name, openRun, view)}
                    title={entry.meta.name}
                    className={rowClass}
                  >
                    <span className="truncate">{entry.meta.name}</span>
                    {status}
                  </Link>
                ) : (
                  <span
                    key={entry.meta.name}
                    title={entry.meta.name}
                    className={rowClass + " opacity-60"}
                  >
                    <span className="truncate">{entry.meta.name}</span>
                    {status}
                  </span>
                );
              })}
            </div>
          ))}
        </nav>
      </div>
    </aside>
  );
}

/// A success rate's color along the house palette: clay at 0, sage at 1, mixed continuously between.
/// Every point on this line holds ≥5.3:1 on the paper ground (clay 5.3:1, sage 5.85:1), so the value
/// stays legible at any rate. `color-mix` over the token vars keeps it on the palette — no ad-hoc hex.
function rateColor(rate: number): string {
  const sage = Math.round(Math.min(Math.max(rate, 0), 1) * 100);
  return `color-mix(in srgb, var(--color-sage) ${sage}%, var(--color-clay))`;
}

/// The open scenario's headline — the eval summary you have drilled into: its name and category, the
/// aggregate pass rate and whether the bar held, the typical latency and cost, and the description.
function ScenarioHeader({ scenario }: { scenario: ScenarioSummary }) {
  const { meta, aggregate } = scenario;
  const threshold = meta.bar.kind === "metric" ? meta.bar.threshold : null;
  const warmth = warmthAggregate(scenario.runs.flatMap((run) => run.usages));

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
        {warmth.median !== null && (
          <>
            <Dot />
            <span
              className={warmth.median < 0.9 ? "text-clay" : undefined}
              title="Median measured cache warmth across the scenario's calls, with the total tokens the provider encoded fresh — a low median means the scenario thrashes the prefix cache."
            >
              {Math.round(warmth.median * 100)}% warm · {formatTokens(warmth.rePrefilled)} ↻
            </span>
          </>
        )}
      </div>
      {meta.description && (
        <p className="mt-3 max-w-prose text-sm leading-relaxed text-ink-soft">{meta.description}</p>
      )}
    </header>
  );
}

/// The runs of the open scenario, laid out as a horizontal row beneath the summary so the drill-down
/// reads top to bottom. State color and selection are separate cues: a run whose gate regressed reads
/// in clay, a clean one stays neutral, and the *open* run — whatever its state — carries a neutral ink
/// ring, so selection never borrows the clay that means "failed". The one driving live (not yet a
/// completed record) shows last, in sage, so any in-flight run is reachable.
function RunPicker({
  scenario,
  active,
  liveRun,
  view,
}: {
  scenario: ScenarioSummary;
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
          // Two independent axes: the *state* color (neutral for a pass, clay for a regression) and
          // the *selection* affordance. Selection is a neutral ink ring, not more clay — so it
          // composes over either state, keeping "selected failed", "unselected failed", and
          // "selected passing" all distinct at the small dot size.
          const state = passed
            ? "border-line text-ink-soft hover:border-ink-faint "
            : "border-clay/60 bg-clay-soft/15 text-clay hover:border-clay ";
          const selection = isActive ? "ring-1 ring-inset ring-ink font-semibold " : "";
          return (
            <Link
              key={run.index}
              to={runPath(scenario.meta.name, run.index, view)}
              title={`Run ${run.index} · ${passed ? "passed" : "failed"}${isActive ? " · open" : ""}`}
              className={
                "flex h-7 min-w-[1.75rem] items-center justify-center border px-1.5 font-mono text-2xs transition-colors " +
                state +
                selection
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
                ? "border-sage text-sage ring-1 ring-inset ring-ink font-semibold "
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
function held(scenario: ScenarioSummary): boolean {
  const { meta, aggregate } = scenario;
  return meta.bar.kind === "gating"
    ? aggregate.gating_passed
    : aggregate.rate >= meta.bar.threshold;
}

/// The interstitial while the run's views are not yet ready: fetching the run's full record, a fetch
/// error, or the replica folding the log once it arrives. The fetch is the first hop (the lean package
/// carries no event log), so its state leads; the fold state shows once the record is in hand.
function Pending({ state, record }: { state: ReplicaState; record: RunRecordState }) {
  const { error, message } =
    record.status === "error"
      ? { error: true, message: `Could not fetch the run — ${record.message}` }
      : record.status === "connecting"
        ? { error: false, message: "Fetching the run's event log…" }
        : state.status === "error"
          ? { error: true, message: `Could not fold the log — ${state.message}` }
          : { error: false, message: "Folding the event log…" };
  return (
    <div className={"flex-1 py-16 text-center text-sm " + (error ? "text-clay" : "text-ink-faint")}>
      {message}
    </div>
  );
}
