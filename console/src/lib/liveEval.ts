import { useEffect, useState } from "react";

import type { EvalPackage } from "../types/EvalPackage.ts";
import type { Event } from "../types/Event.ts";
import type { LiveEvent } from "../types/LiveEvent.ts";

/// Where a running `eval --serve` is reachable. The base URL is the harness's address (e.g.
/// `http://localhost:7878`); the stream is `${baseUrl}/eval/stream`.
export interface LiveEvalConnection {
  baseUrl: string;
}

/// The state of watching a live eval: opening the stream, tailing it, the run finished (the harness is
/// still serving the final state), or stalled on an error. `EventSource` reconnects on its own, and the
/// server answers a reconnect with a fresh snapshot, so an error is transient.
export type LiveEvalStatus =
  | { status: "connecting" }
  | { status: "streaming" }
  | { status: "finished" }
  | { status: "error"; message: string };

/// A live eval, folded from the harness's SSE stream into the same [`EvalPackage`] the static viewer
/// renders. `pkg` is the growing package — seeded from the snapshot, then patched by each authoritative
/// `RunCompleted` — so the scoreboard fills in as runs land. `liveRuns` maps a `scenario:run` key to the
/// events of a run currently driving, streamed as they happen, so a deep-dive can watch it deliberate.
export interface LiveEval {
  pkg: EvalPackage | null;
  status: LiveEvalStatus;
  liveRuns: ReadonlyMap<string, Event[]>;
}

const IDLE: LiveEval = { pkg: null, status: { status: "connecting" }, liveRuns: new Map() };

/// What the eval frame hands its nested routes: the package, the runs currently driving (keyed
/// `scenario:run`, with their events-so-far), and the live status (when watching a running eval).
/// Empty for a static, file-loaded package.
export interface EvalContext {
  pkg: EvalPackage;
  liveRuns: ReadonlyMap<string, Event[]>;
  live: LiveEvalStatus | null;
}

/// No run is driving — the live map a static package carries.
export const NO_LIVE_RUNS: ReadonlyMap<string, Event[]> = new Map();

/// The key a driving run is tracked by.
export function runningKey(scenario: number, run: number): string {
  return `${scenario}:${run}`;
}

/// The scenario indices with a run currently driving — what the scoreboard and rail mark as active.
export function activeScenarios(liveRuns: ReadonlyMap<string, Event[]>): ReadonlySet<number> {
  return new Set([...liveRuns.keys()].map((key) => Number(key.split(":")[0])));
}

/// The run index currently driving for a scenario, if any — what a running scenario links into so its
/// in-flight deep-dive can open. (With serial runs there is at most one; the first is taken otherwise.)
export function liveRunOf(liveRuns: ReadonlyMap<string, Event[]>, scenario: number): number | null {
  for (const key of liveRuns.keys()) {
    const [s, r] = key.split(":").map(Number);
    if (s === scenario) return r;
  }
  return null;
}

/// Watch a live eval over SSE, folding it into a growing package. Inert when `connection` is null (so a
/// caller can hold the hook unconditionally and toggle watching). The fold is authoritative on
/// `RunCompleted` — it carries the run's whole record — so the result converges on the canonical package
/// no matter when the viewer connected or which live `RunEvent`s it happened to see.
export function useLiveEval(connection: LiveEvalConnection | null): LiveEval {
  const [state, setState] = useState<LiveEval>(IDLE);

  const baseUrl = connection?.baseUrl;
  // Reset the fold the moment the address changes — a new connection is a new eval — at render time
  // (React's reset-on-prop-change pattern), so the effect below only ever sets state from callbacks.
  const [tracked, setTracked] = useState(baseUrl);
  if (baseUrl !== tracked) {
    setTracked(baseUrl);
    setState(IDLE);
  }

  useEffect(() => {
    if (baseUrl === undefined) return;
    const source = new EventSource(`${baseUrl}/eval/stream`);

    // The snapshot is the current package whole; it both seeds and (on a reconnect) resets the fold.
    source.addEventListener("snapshot", (message) => {
      const pkg = JSON.parse(message.data) as EvalPackage;
      setState((prev) => ({ ...prev, pkg, status: { status: "streaming" }, liveRuns: new Map() }));
    });
    source.addEventListener("live", (message) => {
      const event = JSON.parse(message.data) as LiveEvent;
      setState((prev) => fold(prev, event));
    });
    // A drop is transient: EventSource reconnects and the server replies with a fresh snapshot.
    source.onerror = () => {
      setState((prev) =>
        prev.status.status === "finished"
          ? prev
          : {
              ...prev,
              status: { status: "error", message: "the live stream dropped; reconnecting…" },
            },
      );
    };

    return () => source.close();
  }, [baseUrl]);

  return state;
}

/// Apply one live delta. `RunStarted` opens a live run; `RunEvent` appends to it as the run deliberates;
/// `RunCompleted` is authoritative — it folds the run's record into the package and retires the live
/// buffer. `Manifest` only seeds via the snapshot, never as a delta.
function fold(state: LiveEval, event: LiveEvent): LiveEval {
  switch (event.kind) {
    case "run_started": {
      const liveRuns = new Map(state.liveRuns);
      liveRuns.set(runningKey(event.scenario, event.run), []);
      return { ...state, liveRuns };
    }
    case "run_event": {
      const key = runningKey(event.scenario, event.run);
      const liveRuns = new Map(state.liveRuns);
      // Append to a fresh array (a new reference) so the deep-dive re-folds on each event; keep the
      // earlier elements by reference, so the same run reads as one growing log, not a different one.
      liveRuns.set(key, [...(state.liveRuns.get(key) ?? []), event.event]);
      return { ...state, liveRuns };
    }
    case "run_completed": {
      const liveRuns = new Map(state.liveRuns);
      liveRuns.delete(runningKey(event.scenario, event.run));
      return { ...state, pkg: applyRunCompleted(state.pkg, event), liveRuns };
    }
    case "finished": {
      const pkg = state.pkg
        ? { ...state.pkg, meta: { ...state.pkg.meta, finished_at_ms: event.finished_at_ms } }
        : state.pkg;
      return { ...state, pkg, status: { status: "finished" }, liveRuns: new Map() };
    }
    case "manifest":
      return state;
  }
}

/// Fold a finished run into the package: replace any prior copy of the run, keep the scenario's runs in
/// index order, and adopt the freshly recomputed aggregate.
function applyRunCompleted(
  pkg: EvalPackage | null,
  event: Extract<LiveEvent, { kind: "run_completed" }>,
): EvalPackage | null {
  if (!pkg) return pkg;
  const scenarios = pkg.scenarios.map((scenario, index) => {
    if (index !== event.scenario) return scenario;
    const runs = [
      ...scenario.runs.filter((run) => run.index !== event.record.index),
      event.record,
    ].sort((a, b) => a.index - b.index);
    return { ...scenario, runs, aggregate: event.aggregate };
  });
  return { ...pkg, scenarios };
}
