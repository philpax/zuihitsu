import { useEffect, useState } from "react";

import type { Event } from "../types/Event.ts";
import { Replica } from "./replica.ts";

/// The lifecycle of folding a run's log into a replica: nothing selected, the wasm building it, the
/// replica ready, or a failure to surface.
export type ReplicaState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; replica: Replica }
  | { status: "error"; message: string };

/// Build a [`Replica`] whenever `events` changes, cancelling a superseded build. `null` events mean
/// no run is selected.
export function useReplica(events: Event[] | null): ReplicaState {
  const [state, setState] = useState<ReplicaState>({ status: "idle" });

  useEffect(() => {
    if (!events) {
      setState({ status: "idle" });
      return;
    }
    setState({ status: "loading" });
    let cancelled = false;
    Replica.fromEvents(events).then(
      (replica) => {
        if (!cancelled) setState({ status: "ready", replica });
      },
      (cause) => {
        if (!cancelled) {
          const message = cause instanceof Error ? cause.message : String(cause);
          setState({ status: "error", message });
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [events]);

  return state;
}
