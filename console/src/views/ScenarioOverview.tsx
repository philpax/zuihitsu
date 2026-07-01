import { Link, useOutletContext } from "react-router-dom";

import type { ScenarioReport } from "../types/ScenarioReport.ts";
import { type EvalContext, activeScenarios, liveRunOf } from "../lib/liveEval.ts";
import { formatMs, formatRate, formatTokenSplit } from "../lib/format.ts";
import { runPath } from "../lib/routes.ts";
import { Dot } from "../components/primitives.tsx";

/// The eval-package overview: every scenario with its pass rate, how it is judged, and the cost it
/// ran at. The first thing an operator wants — which scenarios held, and which did not — and the
/// way into a single run for the deeper views. The package arrives as the eval frame's outlet context.
export function ScenarioOverview() {
  const { pkg, liveRuns, live } = useOutletContext<EvalContext>();
  const active = activeScenarios(liveRuns);
  const runsPlanned = pkg.meta.runs_per_scenario;
  const regressions = pkg.scenarios.filter((s) => !s.aggregate.gating_passed).length;
  // For a live run, scenarios fill in over time; track how far along, and soften the gates verdict to
  // "holding" until the plan is complete.
  const done = pkg.scenarios.reduce((sum, s) => sum + s.runs.length, 0);
  const total = pkg.scenarios.length * runsPlanned;
  const complete = done >= total;
  // When watching a live eval that hasn't started any runs yet, show a "waiting" banner so the
  // viewer knows the eval is live, not dead — the first run is pending the model's response.
  const isLive = live !== null && live.status !== "finished";
  const waiting = isLive && done === 0 && liveRuns.size === 0;

  return (
    <main className="flex-1 py-7">
      {waiting && (
        <p className="mb-4 font-mono text-xs text-ink-faint">
          <span className="animate-pulse">waiting for the first run to start…</span>
        </p>
      )}
      <div className="mb-6 flex items-baseline justify-between sm:mb-7">
        <h2 className="font-serif text-xl text-ink sm:text-2xl">Scenarios</h2>
        <span className="font-mono text-xs text-ink-soft">
          {pkg.scenarios.length} scenarios ·{" "}
          {!complete && (
            <>
              <span className="text-ink">
                {done}/{total} runs
              </span>{" "}
              ·{" "}
            </>
          )}
          {regressions === 0 ? (
            <span className="text-sage">{complete ? "all gates held" : "gates holding"}</span>
          ) : (
            <span className="text-clay">
              {regressions} regression{regressions > 1 ? "s" : ""}
            </span>
          )}
        </span>
      </div>

      <ul>
        {pkg.scenarios.map((scenario, index) => (
          <ScenarioRow
            key={scenario.meta.name}
            scenario={scenario}
            runsPlanned={runsPlanned}
            active={active.has(index)}
            liveRun={liveRunOf(liveRuns, index)}
            isLive={isLive}
          />
        ))}
      </ul>
    </main>
  );
}

function ScenarioRow({
  scenario,
  runsPlanned,
  active,
  liveRun,
  isLive,
}: {
  scenario: ScenarioReport;
  runsPlanned: number;
  active: boolean;
  liveRun: number | null;
  isLive: boolean;
}) {
  const { meta, aggregate } = scenario;
  const completed = scenario.runs.length;
  // `active` = a run is driving right now (the live RunStarted), even before its first result lands.
  // Pending = not started and not active; running = driving, or part-way through its planned runs.
  const pending = completed === 0 && !active;
  const running = active || (completed > 0 && completed < runsPlanned);
  const threshold = meta.bar.kind === "metric" ? meta.bar.threshold : null;
  const held = meta.bar.kind === "gating" ? aggregate.gating_passed : aggregate.rate >= threshold!;
  // Show per-run links once there is more than one run to pick between — completed runs, plus a run
  // driving live (which has no completed record yet).
  const showRuns = completed > 1 || (completed >= 1 && liveRun !== null);
  const firstRun = scenario.runs[0];
  // The run to open: the first completed one, or — if none has landed yet — the one driving live.
  const openRun = firstRun ? firstRun.index : liveRun;
  // A pending scenario dims on a static package (nothing is coming), but stays at full opacity on a
  // live eval (the first run is queued, not absent).
  const dim = pending && !isLive;

  return (
    <li
      className={
        "group grid grid-cols-1 items-start gap-x-8 gap-y-3 border-b border-line py-6 first:border-t sm:grid-cols-[1fr_auto] " +
        (dim ? "opacity-50" : "")
      }
    >
      <div>
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1.5">
          {active && <ActivityDot />}
          {openRun !== null ? (
            <Link
              to={runPath(meta.name, openRun)}
              className="font-mono text-sm text-ink transition-colors hover:text-clay"
              title={firstRun ? "Inspect this run" : "Watch this run live"}
            >
              {meta.name}
            </Link>
          ) : (
            // Not started: the name stays put, quietly, until a run lands or one begins driving.
            <span className="font-mono text-sm text-ink-soft">{meta.name}</span>
          )}
          <span className="font-mono text-2xs uppercase tracking-widest text-ink-faint">
            {meta.category}
          </span>
          {showRuns && (
            <span className="flex items-baseline gap-1.5">
              {scenario.runs.map((run) => (
                <Link
                  key={run.index}
                  to={runPath(meta.name, run.index)}
                  className="font-mono text-xs text-ink-faint transition-colors hover:text-clay"
                  title={`Inspect run ${run.index}`}
                >
                  {run.index}
                </Link>
              ))}
              {liveRun !== null && (
                <Link
                  to={runPath(meta.name, liveRun)}
                  title={`Run ${liveRun} · streaming live`}
                  className="font-mono text-xs text-sage transition-colors hover:text-clay"
                >
                  {liveRun}
                </Link>
              )}
            </span>
          )}
        </div>
        <p className="mt-2 max-w-prose text-sm leading-relaxed text-ink-soft">{meta.description}</p>
      </div>

      <div className="flex flex-col items-start gap-2.5 sm:items-end">
        {completed === 0 ? (
          active && openRun !== null ? (
            <Link
              to={runPath(meta.name, openRun)}
              title="Watch this run live"
              className="flex items-center gap-2 font-mono text-2xs uppercase tracking-widest text-sage transition-colors hover:text-clay"
            >
              <ActivityDot />
              running · 0/{runsPlanned}
            </Link>
          ) : (
            <span className="font-mono text-2xs uppercase tracking-widest text-ink-faint">
              pending · 0/{runsPlanned}
            </span>
          )
        ) : (
          <>
            <div className="flex items-baseline gap-2.5">
              <span className="font-mono text-lg text-ink">{formatRate(aggregate.rate)}</span>
              <span className="font-mono text-xs text-ink-faint">
                {running
                  ? `${completed}/${runsPlanned} runs`
                  : `${aggregate.runs} run${aggregate.runs > 1 ? "s" : ""}`}
              </span>
            </div>

            <RateBar rate={aggregate.rate} threshold={threshold} held={held} />

            <div className="flex items-baseline gap-3 font-mono text-xs text-ink-faint">
              <span className={held ? "text-sage" : "text-clay"}>
                {meta.bar.kind === "gating"
                  ? aggregate.gating_passed
                    ? running
                      ? "gating · holding"
                      : "gating · held"
                    : "gating · regressed"
                  : `metric ≥ ${formatRate(threshold!)}`}
              </span>
              <Dot />
              <span>p50 {formatMs(aggregate.latency_ms.p50)}</span>
              <Dot />
              <span>
                {formatTokenSplit(aggregate.tokens.prompt_mean, aggregate.tokens.completion_mean)}
              </span>
            </div>
          </>
        )}
      </div>
    </li>
  );
}

/// A sage pulse marking a scenario whose run is driving right now.
function ActivityDot() {
  return (
    <span className="relative flex h-1.5 w-1.5 self-center">
      <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-sage opacity-60" />
      <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-sage" />
    </span>
  );
}

/// A slim rule that fills to the pass rate — sage when the bar held, clay when it did not. For a
/// metric bar, a hairline tick marks the threshold the rate is judged against.
function RateBar({
  rate,
  threshold,
  held,
}: {
  rate: number;
  threshold: number | null;
  held: boolean;
}) {
  return (
    <div className="relative h-[3px] w-44 bg-line">
      <div
        className={"absolute inset-y-0 left-0 " + (held ? "bg-sage" : "bg-clay")}
        style={{ width: `${Math.max(rate * 100, 1.5)}%` }}
      />
      {threshold !== null && (
        <div
          className="absolute inset-y-[-2px] w-px bg-ink-faint"
          style={{ left: `${threshold * 100}%` }}
          title={`threshold ${formatRate(threshold)}`}
        />
      )}
    </div>
  );
}
