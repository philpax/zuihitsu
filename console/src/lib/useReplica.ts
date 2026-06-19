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

type Resolved = { events: Event[]; state: ReplicaState };

/// Build a [`Replica`] whenever `events` changes, cancelling a superseded build. `null` events mean
/// no run is selected. Idle and loading are derived during render rather than written synchronously
/// in the effect; only the async resolution writes state (the Rules-of-React-friendly shape).
export function useReplica(events: Event[] | null): ReplicaState {
  const [resolved, setResolved] = useState<Resolved | null>(null);

  useEffect(() => {
    if (!events) return;
    let cancelled = false;
    Replica.fromEvents(events).then(
      (replica) => {
        if (!cancelled) setResolved({ events, state: { status: "ready", replica } });
      },
      (cause) => {
        if (!cancelled) {
          const message = cause instanceof Error ? cause.message : String(cause);
          setResolved({ events, state: { status: "error", message } });
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [events]);

  if (!events) return { status: "idle" };
  if (resolved?.events === events) return resolved.state;
  // A re-fold is in flight. If it is the same run growing — the first event is shared by reference, as
  // a live run's log only ever appends — keep the last replica on screen so the stream updates smoothly
  // instead of flashing "loading"; a different selection (navigating runs) shows loading as before.
  if (
    resolved?.state.status === "ready" &&
    resolved.events.length > 0 &&
    events.length > 0 &&
    resolved.events[0] === events[0]
  ) {
    return resolved.state;
  }
  return { status: "loading" };
}
