import {
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  Line,
  LineChart,
  ResponsiveContainer,
  Scatter,
  ScatterChart,
  Tooltip,
  XAxis,
  YAxis,
  ZAxis,
} from "recharts";

import type { HistoryEntry, HistoryScenario } from "../lib/history.ts";
import { formatDate, formatMs, formatRate, formatTokens } from "../lib/format.ts";
import { Eyebrow } from "../components/primitives.tsx";

// The palette as concrete colors — Recharts draws SVG, so it gets hex rather than the CSS tokens.
const INK = "#2c2823";
const FAINT = "#9c9484";
const LINE = "#ddd4c3";
const CLAY = "#af6047";
const SAGE = "#707a55";
const PAPER = "#f4efe5";

const TICK = { fill: FAINT, fontSize: 10, fontFamily: "var(--font-mono)" } as const;
const TOOLTIP = {
  background: PAPER,
  border: `1px solid ${LINE}`,
  borderRadius: 2,
  fontFamily: "var(--font-mono)",
  fontSize: 11,
  color: INK,
} as const;

/// The Trends view: the metrics history as a small spread of charts — a pass-rate trend, a latency
/// comparison, and a cost scatter. The one surface that outlives a single package; the shape of how
/// the agent's behavior moves as the model and the code change.
export function TrendsView({ entries }: { entries: HistoryEntry[] }) {
  const names = scenarioOrder(entries);
  // Each scenario's most recent data point. Entries are oldest-first, so the last write wins; this
  // keeps a partial run (a single scenario re-run) from blanking the latest-state comparisons.
  const latestByName = new Map<string, HistoryScenario>();
  for (const entry of entries) for (const s of entry.scenarios) latestByName.set(s.name, s);
  const gatingByName = new Map([...latestByName].map(([name, s]) => [name, s.gating_passed]));
  const models = [...new Set(entries.map((e) => e.model_id))];
  const span =
    entries.length > 0
      ? `${formatDate(entries[0].ts_ms)} – ${formatDate(entries[entries.length - 1].ts_ms)}`
      : "";

  // Only the scenarios whose rate actually moves earn a line; the rest are noted as steady, so the
  // trend chart stays legible rather than a thicket of flat lines at 100%.
  const moving = names.filter((name) => {
    const rates = entries
      .map((e) => e.scenarios.find((s) => s.name === name)?.rate)
      .filter((rate): rate is number => rate !== undefined);
    return new Set(rates).size > 1;
  });
  const steady = names.length - moving.length;

  const rateData = entries.map((entry, index) => {
    const row: Record<string, number | string> = { run: `${index + 1}` };
    for (const scenario of entry.scenarios) row[scenario.name] = scenario.rate;
    return row;
  });

  const recent = [...latestByName.values()];
  const latency = [...recent]
    .sort((a, b) => b.latency_p50_ms - a.latency_p50_ms)
    .map((s) => ({ name: s.name, latency: s.latency_p50_ms, ok: s.gating_passed }));

  const cost = recent.map((s) => ({
    name: s.name,
    latency: s.latency_p50_ms,
    tokens: s.total_tokens_mean,
  }));

  return (
    <section className="flex flex-col gap-6 sm:gap-8">
      <div className="flex flex-wrap items-baseline justify-between gap-y-1">
        <h2 className="font-serif text-xl text-ink sm:text-2xl">Trends</h2>
        <span className="font-mono text-xs text-ink-soft">
          {entries.length} runs · {span} · {models.join(", ")}
        </span>
      </div>

      <Panel label={`pass rate over time · ${moving.length} moving, ${steady} steady at 100%`}>
        <ResponsiveContainer width="100%" height={260}>
          <LineChart data={rateData} margin={{ top: 8, right: 16, bottom: 0, left: 0 }}>
            <CartesianGrid vertical={false} stroke={LINE} />
            <XAxis dataKey="run" tick={TICK} tickLine={false} axisLine={{ stroke: LINE }} />
            <YAxis
              domain={[0, 1]}
              tickFormatter={(v: number) => formatRate(v)}
              tick={TICK}
              tickLine={false}
              axisLine={false}
              width={38}
            />
            <Tooltip
              contentStyle={TOOLTIP}
              formatter={(value) => formatRate(Number(value))}
              itemStyle={{ color: INK }}
              labelFormatter={(label) => `run ${label}`}
            />
            {moving.map((name) => (
              <Line
                key={name}
                type="monotone"
                dataKey={name}
                stroke={gatingByName.get(name) ? SAGE : CLAY}
                strokeWidth={1.5}
                dot={{ r: 2 }}
                activeDot={{ r: 3 }}
                connectNulls
              />
            ))}
          </LineChart>
        </ResponsiveContainer>
      </Panel>

      <Panel label="latency p50 by scenario · most recent">
        <ResponsiveContainer width="100%" height={names.length * 24 + 32}>
          <BarChart
            data={latency}
            layout="vertical"
            margin={{ top: 0, right: 24, bottom: 0, left: 8 }}
          >
            <CartesianGrid horizontal={false} stroke={LINE} />
            <XAxis
              type="number"
              tickFormatter={(v: number) => formatMs(v)}
              tick={TICK}
              tickLine={false}
              axisLine={{ stroke: LINE }}
            />
            <YAxis
              type="category"
              dataKey="name"
              width={210}
              tick={TICK}
              tickLine={false}
              axisLine={false}
            />
            <Tooltip
              contentStyle={TOOLTIP}
              cursor={{ fill: "#00000008" }}
              formatter={(value) => formatMs(Number(value))}
            />
            <Bar dataKey="latency" radius={[0, 1, 1, 0]} barSize={11}>
              {latency.map((d) => (
                <Cell key={d.name} fill={d.ok ? SAGE : CLAY} fillOpacity={0.75} />
              ))}
            </Bar>
          </BarChart>
        </ResponsiveContainer>
      </Panel>

      <Panel label="latency vs token cost · most recent">
        <ResponsiveContainer width="100%" height={300}>
          <ScatterChart margin={{ top: 8, right: 20, bottom: 8, left: 8 }}>
            <CartesianGrid stroke={LINE} />
            <XAxis
              type="number"
              dataKey="latency"
              name="latency"
              tickFormatter={(v: number) => formatMs(v)}
              tick={TICK}
              tickLine={false}
              axisLine={{ stroke: LINE }}
            />
            <YAxis
              type="number"
              dataKey="tokens"
              name="tokens"
              tickFormatter={(v: number) => formatTokens(v)}
              tick={TICK}
              tickLine={false}
              axisLine={false}
              width={44}
            />
            <ZAxis range={[36, 36]} />
            <Tooltip contentStyle={TOOLTIP} cursor={{ stroke: LINE }} content={<CostTooltip />} />
            <Scatter data={cost} fill={CLAY} fillOpacity={0.55} />
          </ScatterChart>
        </ResponsiveContainer>
      </Panel>
    </section>
  );
}

function Panel({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <Eyebrow>{label}</Eyebrow>
      <div className="mt-4">{children}</div>
    </div>
  );
}

/// A scatter point names the scenario the default tooltip cannot.
function CostTooltip({
  active,
  payload,
}: {
  active?: boolean;
  payload?: Array<{ payload: Cost }>;
}) {
  if (!active || !payload?.length) return null;
  const point = payload[0].payload;
  return (
    <div style={TOOLTIP} className="px-2 py-1">
      <div className="text-ink">{point.name}</div>
      <div className="text-ink-faint">
        {formatMs(point.latency)} · {formatTokens(point.tokens)} tok
      </div>
    </div>
  );
}

interface Cost {
  name: string;
  latency: number;
  tokens: number;
}

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
