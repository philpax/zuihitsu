// The tracked trend file (`eval/history.jsonl`) is one deterministic line per eval run over time.
// It is not part of the EvalPackage contract, so these mirror the harness's serialized shape by
// hand (see `history_line` in `crates/eval/src/main.rs`). The v2 record carries the run's `name`
// (correlating a line back to its `eval/<name>.json` package), real wall-clock stamps, the git
// state, and per-scenario the bar and per-criterion tallies. The retired v1 archive lives in
// `eval/history-v1.jsonl` and no longer parses here; a hand-loaded v1 line is skipped with a warning.

export interface CriterionStat {
  criterion: string;
  kind: string;
  passed: number;
  total: number;
}

export interface HistoryScenario {
  name: string;
  rate: number;
  gating_passed: boolean;
  runs: number;
  bar: string;
  wall_clock_p50_ms: number;
  latency_p50_ms: number;
  steps_p50: number;
  total_tokens_mean: number;
  criteria: CriterionStat[];
  // Retired in v2; kept optional so the token-split chart tolerates their absence and older lines.
  prompt_tokens_mean?: number;
  completion_tokens_mean?: number;
}

export interface HistoryEntry {
  name: string;
  started_at_ms: number;
  finished_at_ms: number;
  git_sha: string | null;
  git_dirty: boolean;
  model_id: string;
  runs_per_scenario: number;
  // Absent for a full-suite run; the `--scenario` substring filter when the run was targeted.
  scenario_filter?: string;
  scenarios: HistoryScenario[];
}

/// Parse a history file (JSON Lines — one entry per line) into v2 entries, ordered oldest-first. A
/// pre-v2 line (no `name` field — the retired archive shape) is skipped with a console warning rather
/// than crashing the view, so a hand-loaded old file degrades gracefully.
export function parseHistory(text: string): HistoryEntry[] {
  const entries: HistoryEntry[] = [];
  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (trimmed.length === 0) continue;
    const record = JSON.parse(trimmed) as Partial<HistoryEntry>;
    if (typeof record.name !== "string") {
      console.warn("trends: skipping a pre-v2 history record without a name field");
      continue;
    }
    entries.push(record as HistoryEntry);
  }
  return entries.sort((a, b) => a.finished_at_ms - b.finished_at_ms);
}
