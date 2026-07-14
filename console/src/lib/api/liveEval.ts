import { useEffect, useState } from "react";

import type { PackageSummary } from "@zuihitsu/wire/types/PackageSummary.ts";
import type { RunRecord } from "@zuihitsu/wire/types/RunRecord.ts";
import type { Event } from "@zuihitsu/wire/types/Event.ts";
import {
  type InFlightGeneration,
  foldFrame,
  supersede,
  supersededConversation,
} from "../model/inflight.ts";
import type { LiveEvent } from "@zuihitsu/wire/types/LiveEvent.ts";

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

/// A live eval, folded from the harness's SSE stream into a lean [`PackageSummary`] — everything the
/// scoreboard and rail render, without the event logs (the deep-dive fetches one run's full record on
/// demand). `pkg` is the growing summary — seeded from the snapshot, then patched by each authoritative
/// `RunSummarized` — so the scoreboard fills in as runs land. `liveRuns` maps a `scenario:run` key to
/// the events of a run currently driving, streamed as they happen, so a deep-dive can watch it
/// deliberate.
export interface LiveEval {
  pkg: PackageSummary | null;
  status: LiveEvalStatus;
  liveRuns: ReadonlyMap<string, Event[]>;
  /// Each driving run's in-flight generations by conversation — the same token accumulation the
  /// live agent console folds (`lib/model/inflight.ts`), keyed by the run's `scenario:run` key.
  progress: ReadonlyMap<string, ReadonlyMap<string, InFlightGeneration>>;
}

const IDLE: LiveEval = {
  pkg: null,
  status: { status: "connecting" },
  liveRuns: new Map(),
  progress: new Map(),
};

/// What the eval frame hands its nested routes: the lean package summary, the runs currently driving
/// (keyed `scenario:run`, with their events-so-far), the live status (when watching a running eval),
/// and `getRun` — the seam a deep-dive fetches one run's full record through. A live context fetches
/// over the harness's run endpoint; a file-loaded context resolves synchronously from the retained
/// full package. Empty maps for a static, file-loaded package.
export interface EvalContext {
  pkg: PackageSummary;
  liveRuns: ReadonlyMap<string, Event[]>;
  live: LiveEvalStatus | null;
  /// In-flight token accumulations per driving run (empty for a static package).
  progress: ReadonlyMap<string, ReadonlyMap<string, InFlightGeneration>>;
  /// Fetch one run's full [`RunRecord`] — its event log and journal — resolving the deep-dive's views.
  getRun: (scenario: number, run: number) => Promise<RunRecord>;
}

/// Fetch one run's full record from a running harness's per-run endpoint. `baseUrl` may be `""` for the
/// embedded same-origin build. A non-OK response (a 404 for a stale link, or a run not yet complete)
/// rejects with a clear message the deep-dive surfaces.
export async function fetchRunRecord(
  baseUrl: string,
  scenario: number,
  run: number,
): Promise<RunRecord> {
  const response = await fetch(`${baseUrl}/eval/run/${scenario}/${run}`);
  if (!response.ok) {
    throw new Error(
      `could not fetch run ${scenario}:${run} — ${response.status} ${response.statusText}`,
    );
  }
  return (await response.json()) as RunRecord;
}

/// No run is driving — the live maps a static package carries.
export const NO_LIVE_RUNS: ReadonlyMap<string, Event[]> = new Map();
export const NO_PROGRESS: ReadonlyMap<string, ReadonlyMap<string, InFlightGeneration>> = new Map();

/// The current epoch time, re-read on `intervalMs` so an elapsed readout ticks without any data change.
/// Freezes when `active` is false (e.g. once the run has finished, there is nothing left to tick). A
/// coarse interval is plenty — the elapsed readout is to the minute.
export function useNow(intervalMs: number, active = true): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active || intervalMs <= 0) return;
    const timer = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(timer);
  }, [intervalMs, active]);
  return now;
}

/// A live run's projected finish as epoch milliseconds, or `null` when nothing has completed yet (no
/// per-run cost to extrapolate from). Each scenario's remaining runs are costed at that scenario's mean
/// drive wall-clock over its completed runs, falling back to the global mean where a scenario has none
/// done; the summed remaining cost is divided by the harness's concurrency and added to `nowMs`.
export function projectFinishMs(pkg: PackageSummary, nowMs: number): number | null {
  const completed = pkg.scenarios.flatMap((scenario) => scenario.runs);
  if (completed.length === 0) return null;
  const globalMean = mean(completed.map((run) => run.metrics.wall_clock_ms));
  const remainingMs = pkg.scenarios.reduce((acc, scenario) => {
    const remaining = Math.max(0, pkg.meta.runs_per_scenario - scenario.runs.length);
    if (remaining === 0) return acc;
    const scenarioMean =
      scenario.runs.length > 0
        ? mean(scenario.runs.map((run) => run.metrics.wall_clock_ms))
        : globalMean;
    return acc + remaining * scenarioMean;
  }, 0);
  const concurrency = Math.max(1, pkg.meta.concurrency);
  return nowMs + remainingMs / concurrency;
}

function mean(xs: number[]): number {
  return xs.reduce((sum, x) => sum + x, 0) / xs.length;
}

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

/// Watch a live eval over SSE, folding it into a growing package summary. Inert when `connection` is
/// null (so a caller can hold the hook unconditionally and toggle watching). The fold is authoritative
/// on `RunSummarized` — it carries the run's summary and the recomputed aggregate — so the scoreboard
/// converges no matter when the viewer connected or which live `RunEvent`s it happened to see; the one
/// open run's full event log is fetched separately over the run endpoint.
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

    // The snapshot is the current package summary whole; it both seeds and (on a reconnect) resets
    // the fold.
    source.addEventListener("snapshot", (message) => {
      const pkg = JSON.parse(message.data) as PackageSummary;
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
/// `RunSummarized` is authoritative — it folds the run's summary into the package and retires the live
/// buffer. `Manifest` only seeds via the snapshot, never as a delta. Exported for its tests: the
/// mark-not-delete supersede wiring lives here, and it is what keeps the pending turn mounted
/// through the replica's refold lag.
export function fold(state: LiveEval, event: LiveEvent): LiveEval {
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
      // A committed ModelCalled (or the agent's turn) supersedes that conversation's in-flight
      // text — marking it mid-turn, dropping it at the turn's end (see `supersede`).
      const superseded = supersededConversation(event.event);
      const runProgress = superseded ? state.progress.get(key) : undefined;
      const current = superseded ? runProgress?.get(superseded) : undefined;
      let progress = state.progress;
      if (superseded && current) {
        const updated = new Map(runProgress);
        const next = supersede(current, event.event);
        if (next) updated.set(superseded, next);
        else updated.delete(superseded);
        const nextProgress = new Map(state.progress);
        nextProgress.set(key, updated);
        progress = nextProgress;
      }
      return { ...state, liveRuns, progress };
    }
    case "run_progress": {
      const key = runningKey(event.scenario, event.run);
      const progress = new Map(state.progress);
      const runProgress = new Map(state.progress.get(key) ?? []);
      const next = foldFrame(runProgress.get(event.frame.conversation), event.frame);
      if (next) runProgress.set(event.frame.conversation, next);
      else runProgress.delete(event.frame.conversation);
      progress.set(key, runProgress);
      return { ...state, progress };
    }
    case "run_summarized": {
      const key = runningKey(event.scenario, event.run);
      const liveRuns = new Map(state.liveRuns);
      liveRuns.delete(key);
      const progress = new Map(state.progress);
      progress.delete(key);
      return { ...state, pkg: applyRunSummarized(state.pkg, event), liveRuns, progress };
    }
    case "finished": {
      const pkg = state.pkg
        ? { ...state.pkg, meta: { ...state.pkg.meta, finished_at_ms: event.finished_at_ms } }
        : state.pkg;
      return {
        ...state,
        pkg,
        status: { status: "finished" },
        liveRuns: new Map(),
        progress: new Map(),
      };
    }
    case "manifest":
      return state;
    default:
      // A frame kind this bundle does not know — a version skew between the console and the serving
      // binary in either direction (an old server's run_completed, or a newer server's future
      // frames). Dropped rather than crashing, at a real cost: a dropped completion frame means the
      // scoreboard does not converge for that run until the viewer reloads against a matching
      // server. The two are built together by cargo build, so the skew only arises against a stale
      // --serve binary.
      return state;
  }
}

/// Fold a finished run's summary into the package: replace any prior copy of the run, keep the
/// scenario's runs in index order, and adopt the freshly recomputed aggregate.
function applyRunSummarized(
  pkg: PackageSummary | null,
  event: Extract<LiveEvent, { kind: "run_summarized" }>,
): PackageSummary | null {
  if (!pkg) return pkg;
  const scenarios = pkg.scenarios.map((scenario, index) => {
    if (index !== event.scenario) return scenario;
    const runs = [
      ...scenario.runs.filter((run) => run.index !== event.summary.index),
      event.summary,
    ].sort((a, b) => a.index - b.index);
    return { ...scenario, runs, aggregate: event.aggregate };
  });
  return { ...pkg, scenarios };
}
