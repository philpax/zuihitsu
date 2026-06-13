import type { HistoryEntry, HistoryScenario } from "../lib/history.ts";
import { formatDate, formatMs, formatRate, formatTokens } from "../lib/format.ts";
import { Sparkline } from "../components/Sparkline.tsx";
import { Eyebrow } from "../components/primitives.tsx";

/// The Trends view: each scenario's pass rate, latency, and token cost over the tracked run history,
/// as small trend lines. The metrics history is the one thing that outlives a single package — the
/// shape of how the agent's behavior moves as the model and the code change.
export function TrendsView({ entries }: { entries: HistoryEntry[] }) {
  const names = scenarioOrder(entries);
  const models = [...new Set(entries.map((entry) => entry.model_id))];
  const span =
    entries.length > 0
      ? `${formatDate(entries[0].ts_ms)} – ${formatDate(entries[entries.length - 1].ts_ms)}`
      : "";

  return (
    <section>
      <div className="mb-9 flex items-baseline justify-between">
        <h2 className="font-serif text-2xl text-ink">Trends</h2>
        <span className="font-mono text-xs text-ink-soft">
          {entries.length} runs · {span} · {models.join(", ")}
        </span>
      </div>

      <div className="grid grid-cols-[1fr_9rem_9rem_9rem] items-end gap-x-8 border-b border-line pb-2">
        <Eyebrow>scenario</Eyebrow>
        <Eyebrow>pass rate</Eyebrow>
        <Eyebrow>latency p50</Eyebrow>
        <Eyebrow>tokens</Eyebrow>
      </div>

      {names.map((name) => {
        const points = entries
          .map((entry) => entry.scenarios.find((scenario) => scenario.name === name))
          .filter((scenario): scenario is HistoryScenario => scenario !== undefined);
        const last = points[points.length - 1];
        const rateColor = last.gating_passed ? "var(--color-sage)" : "var(--color-clay)";

        return (
          <div
            key={name}
            className="grid grid-cols-[1fr_9rem_9rem_9rem] items-center gap-x-8 border-b border-line py-5"
          >
            <span className="font-mono text-sm text-ink">{name}</span>
            <Metric
              chart={
                <Sparkline values={points.map((p) => p.rate)} domainMax={1} stroke={rateColor} />
              }
              value={formatRate(last.rate)}
            />
            <Metric
              chart={<Sparkline values={points.map((p) => p.latency_p50_ms)} stroke={INK_FAINT} />}
              value={formatMs(last.latency_p50_ms)}
            />
            <Metric
              chart={
                <Sparkline values={points.map((p) => p.total_tokens_mean)} stroke={INK_FAINT} />
              }
              value={formatTokens(last.total_tokens_mean)}
            />
          </div>
        );
      })}
    </section>
  );
}

const INK_FAINT = "var(--color-ink-faint)";

function Metric({ chart, value }: { chart: React.ReactNode; value: string }) {
  return (
    <div className="flex flex-col gap-1">
      {chart}
      <span className="font-mono text-2xs text-ink-soft">{value}</span>
    </div>
  );
}

/// The scenarios in first-seen order across the history, so the rows stay stable as the corpus grows.
function scenarioOrder(entries: HistoryEntry[]): string[] {
  const names: string[] = [];
  const seen = new Set<string>();
  for (const entry of entries) {
    for (const scenario of entry.scenarios) {
      if (!seen.has(scenario.name)) {
        seen.add(scenario.name);
        names.push(scenario.name);
      }
    }
  }
  return names;
}
