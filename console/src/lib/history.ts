// The tracked trend file (`eval/history.jsonl`) is one deterministic line per eval run over time.
// It is not part of the EvalPackage contract, so these mirror the harness's serialized shape by
// hand. TODO: a ts-rs derive on the harness's history type would make these generated too.

export interface HistoryScenario {
  name: string;
  rate: number;
  gating_passed: boolean;
  wall_clock_p50_ms: number;
  latency_p50_ms: number;
  total_tokens_mean: number;
  // The input/output split, added later — optional so rows written before it still parse.
  prompt_tokens_mean?: number;
  completion_tokens_mean?: number;
}

export interface HistoryEntry {
  ts_ms: number;
  git_sha: string | null;
  model_id: string;
  runs_per_scenario: number;
  scenarios: HistoryScenario[];
}

/// Parse a history file (JSON Lines — one entry per line) into entries, ordered oldest-first.
export function parseHistory(text: string): HistoryEntry[] {
  const entries = text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .map((line) => JSON.parse(line) as HistoryEntry);
  return entries.sort((a, b) => a.ts_ms - b.ts_ms);
}
