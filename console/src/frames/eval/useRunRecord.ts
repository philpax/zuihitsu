import { useEffect, useState } from "react";

import type { RunRecord } from "@zuihitsu/wire/types/RunRecord.ts";

/// The lifecycle of fetching one run's full record for the deep-dive: not fetching (a live run, whose
/// events stream in instead), the fetch in flight, the record ready, or a failure to surface.
export type RunRecordState =
  | { status: "idle" }
  | { status: "connecting" }
  | { status: "ready"; record: RunRecord }
  | { status: "error"; message: string };

/// Fetch a completed run's full record through the context seam, cancelling a superseded fetch on a run
/// change. `enabled` is false for a run still driving live (its events arrive over the live buffer, not
/// this endpoint); when false, or when the run is unresolved, the fetch stays idle. Idle and connecting
/// are derived during render (the Rules-of-React-friendly shape used by `useReplica`); only the async
/// resolution writes state, keyed by its `(scenario, run)` so a stale resolution never leaks across a
/// run change.
export function useRunRecord(
  getRun: (scenario: number, run: number) => Promise<RunRecord>,
  scenario: number,
  run: number | null,
  enabled: boolean,
): RunRecordState {
  const [resolved, setResolved] = useState<{
    scenario: number;
    run: number;
    state: RunRecordState;
  } | null>(null);

  useEffect(() => {
    if (!enabled || run === null || scenario < 0) return;
    let ignore = false;
    getRun(scenario, run).then(
      (record) => {
        if (!ignore) setResolved({ scenario, run, state: { status: "ready", record } });
      },
      (cause) => {
        if (!ignore) {
          const message = cause instanceof Error ? cause.message : String(cause);
          setResolved({ scenario, run, state: { status: "error", message } });
        }
      },
    );
    return () => {
      ignore = true;
    };
  }, [getRun, scenario, run, enabled]);

  if (!enabled || run === null || scenario < 0) return { status: "idle" };
  if (resolved && resolved.scenario === scenario && resolved.run === run) return resolved.state;
  return { status: "connecting" };
}
