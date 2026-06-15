/// The URL shapes for an eval package's deep views, in one place so every link — the scenario
/// overview, the run frame, the scenario sidebar, an event's jump into State — builds the same path.
/// The scenario name is the only free-form segment, so it is the only one that needs encoding.

/// A run's path without a view — the prefix the view and `?seq` cursor hang off.
export function runBase(scenario: string, run: number): string {
  return `/eval/${encodeURIComponent(scenario)}/${run}`;
}

/// A run opened at a particular view (the conversation by default — the payoff view).
export function runPath(scenario: string, run: number, view = "conversation"): string {
  return `${runBase(scenario, run)}/${view}`;
}

/// The State view of a stream, folded to `seq` and opened on `memory` — the target an event's memory
/// ref jumps to, landing on the memory as it stood at the point in the timeline that event occurred.
/// `base` is the stream's path (a run's base, or `/live`); the memory name rides in the query, encoded
/// against its slashes.
export function statePath(base: string, seq: number, memory: string): string {
  const query = new URLSearchParams({ seq: String(seq), memory });
  return `${base}/state?${query.toString()}`;
}
