import { useEffect, useState, type RefObject } from "react";

import type { Event } from "../types/Event.ts";
import { Replica } from "./replica.ts";

/// Where to reach a running agent's control surface, and the key to present. The base URL is
/// relative by default (`""`), so the console talks to its own origin — the same host that serves it
/// in production, and a dev proxy locally (see `vite.config.ts`). A loopback peer needs no key.
export interface LiveConnection {
  baseUrl: string;
  key: string | null;
}

/// The state of a live connection: still catching up, tailing the log, or stalled on an error (which
/// is transient — the poll keeps retrying, so a recovered agent returns to `live` on its own).
export type LiveStatus =
  | { status: "connecting" }
  | { status: "live" }
  | { status: "error"; message: string };

/// A live event log, tailed from a running agent. `replica` folds the same materializer the package
/// views use; `events` is the raw stream the Conversation and Events views render off; `head` is the
/// highest seq seen, which the timeline tracks as it grows.
export interface LiveLog {
  replica: Replica | null;
  events: Event[];
  head: number;
  status: LiveStatus;
}

/// The poll cadence. The control surface has no push channel yet (the store carries a `subscribe`
/// seam for a later SSE upgrade), so the console tails by polling the catch-up endpoint.
const POLL_INTERVAL_MS = 1000;

/// Tail a running agent's event log: seed a [`Replica`] from an initial catch-up (`from=0`), then
/// poll the tail (`from=head + 1`) and append what arrives. `following` decides whether a new batch
/// advances the fold horizon — held by the parent so a batch arriving while the operator is
/// time-travel pinned extends the log without disturbing the graph they are inspecting (the fold is
/// applied here, synchronously with the append, so the views read a settled horizon on re-render).
export function useLiveLog(connection: LiveConnection, following: RefObject<boolean>): LiveLog {
  const [log, setLog] = useState<LiveLog>({
    replica: null,
    events: [],
    head: 0,
    status: { status: "connecting" },
  });

  // Re-fetch the whole log from scratch whenever the connection changes; a new agent is a new log.
  const { baseUrl, key } = connection;
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setInterval> | undefined;
    let replica: Replica | null = null;
    let events: Event[] = [];
    let head = 0;

    async function fetchFrom(from: number): Promise<Event[]> {
      const headers: HeadersInit = key ? { Authorization: `Bearer ${key}` } : {};
      const response = await fetch(`${baseUrl}/control/events?from=${from}`, { headers });
      if (!response.ok) {
        throw new Error(`the agent answered ${response.status} ${response.statusText}`);
      }
      return (await response.json()) as Event[];
    }

    async function poll() {
      try {
        const tail = await fetchFrom(head + 1);
        if (cancelled) return;
        if (tail.length === 0) {
          setLog((prev) =>
            prev.status.status === "live" ? prev : { ...prev, status: { status: "live" } },
          );
          return;
        }
        replica!.append(tail);
        head = replica!.headSeq;
        if (following.current) replica!.foldTo(head);
        events = [...events, ...tail];
        setLog({ replica, events, head, status: { status: "live" } });
      } catch (cause) {
        if (!cancelled) setLog((prev) => ({ ...prev, status: errorStatus(cause) }));
      }
    }

    (async () => {
      try {
        const initial = await fetchFrom(0);
        if (cancelled) return;
        replica = await Replica.fromEvents(initial);
        if (cancelled) return;
        events = initial;
        head = replica.headSeq;
        setLog({ replica, events, head, status: { status: "live" } });
        timer = setInterval(poll, POLL_INTERVAL_MS);
      } catch (cause) {
        if (!cancelled) setLog((prev) => ({ ...prev, status: errorStatus(cause) }));
      }
    })();

    return () => {
      cancelled = true;
      if (timer) clearInterval(timer);
    };
  }, [baseUrl, key, following]);

  return log;
}

function errorStatus(cause: unknown): LiveStatus {
  return { status: "error", message: cause instanceof Error ? cause.message : String(cause) };
}
