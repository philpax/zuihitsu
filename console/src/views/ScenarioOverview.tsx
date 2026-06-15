import { Link, useOutletContext } from "react-router-dom";

import type { EvalPackage } from "../types/EvalPackage.ts";
import type { ScenarioReport } from "../types/ScenarioReport.ts";
import { formatMs, formatRate, formatTokens } from "../lib/format.ts";
import { runPath } from "../lib/routes.ts";
import { Dot } from "../components/primitives.tsx";

/// The eval-package overview: every scenario with its pass rate, how it is judged, and the cost it
/// ran at. The first thing an operator wants — which scenarios held, and which did not — and the
/// way into a single run for the deeper views. The package arrives as the eval frame's outlet context.
export function ScenarioOverview() {
  const pkg = useOutletContext<EvalPackage>();
  const regressions = pkg.scenarios.filter((s) => !s.aggregate.gating_passed).length;

  return (
    <main className="flex-1 py-7">
      <div className="mb-6 flex items-baseline justify-between sm:mb-7">
        <h2 className="font-serif text-xl text-ink sm:text-2xl">Scenarios</h2>
        <span className="font-mono text-xs text-ink-soft">
          {pkg.scenarios.length} scenarios ·{" "}
          {regressions === 0 ? (
            <span className="text-sage">all gates held</span>
          ) : (
            <span className="text-clay">
              {regressions} regression{regressions > 1 ? "s" : ""}
            </span>
          )}
        </span>
      </div>

      <ul>
        {pkg.scenarios.map((scenario) => (
          <ScenarioRow key={scenario.meta.name} scenario={scenario} />
        ))}
      </ul>
    </main>
  );
}

function ScenarioRow({ scenario }: { scenario: ScenarioReport }) {
  const { meta, aggregate } = scenario;
  const threshold = meta.bar.kind === "metric" ? meta.bar.threshold : null;
  const held = meta.bar.kind === "gating" ? aggregate.gating_passed : aggregate.rate >= threshold!;
  const multiRun = scenario.runs.length > 1;

  return (
    <li className="group grid grid-cols-1 items-start gap-x-8 gap-y-3 border-b border-line py-6 first:border-t sm:grid-cols-[1fr_auto]">
      <div>
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1.5">
          <Link
            to={runPath(meta.name, scenario.runs[0].index)}
            className="font-mono text-sm text-ink transition-colors hover:text-clay"
            title="Inspect this run"
          >
            {meta.name}
          </Link>
          <span className="font-mono text-2xs uppercase tracking-widest text-ink-faint">
            {meta.category}
          </span>
          {multiRun && (
            <span className="flex items-baseline gap-1.5">
              {scenario.runs.map((run) => (
                <Link
                  key={run.index}
                  to={runPath(meta.name, run.index)}
                  className="font-mono text-2xs text-ink-faint transition-colors hover:text-clay"
                  title={`Inspect run ${run.index}`}
                >
                  {run.index}
                </Link>
              ))}
            </span>
          )}
        </div>
        <p className="mt-2 max-w-prose text-sm leading-relaxed text-ink-soft">{meta.description}</p>
      </div>

      <div className="flex flex-col items-start gap-2.5 sm:items-end">
        <div className="flex items-baseline gap-2.5">
          <span className="font-mono text-lg text-ink">{formatRate(aggregate.rate)}</span>
          <span className="font-mono text-2xs text-ink-faint">
            {aggregate.runs} run{aggregate.runs > 1 ? "s" : ""}
          </span>
        </div>

        <RateBar rate={aggregate.rate} threshold={threshold} held={held} />

        <div className="flex items-baseline gap-3 font-mono text-2xs text-ink-faint">
          <span className={held ? "text-sage" : "text-clay"}>
            {meta.bar.kind === "gating"
              ? aggregate.gating_passed
                ? "gating · held"
                : "gating · regressed"
              : `metric ≥ ${formatRate(threshold!)}`}
          </span>
          <Dot />
          <span>p50 {formatMs(aggregate.latency_ms.p50)}</span>
          <Dot />
          <span>{formatTokens(aggregate.tokens.total_mean)} tok</span>
        </div>
      </div>
    </li>
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
