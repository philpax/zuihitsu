import { useEffect, useState, type RefObject } from "react";

import type { Event } from "../../types/Event.ts";
import type { TurnProgress } from "../../types/TurnProgress.ts";
import {
  type InFlightGeneration,
  foldFrame,
  supersede,
  supersededConversation,
} from "../model/inflight.ts";
import { Replica } from "../replica/replica.ts";
import { errorMessage } from "./http.ts";
import { openEventStream } from "./liveStream.ts";

/// Where to reach a running agent's control surface, and the key to present. The base URL is
/// relative by default (`""`), so the console talks to its own origin — the same host that serves it
/// in production, and a dev proxy locally (see `vite.config.ts`). A loopback peer needs no key.
export interface LiveConnection {
  baseUrl: string;
  key: string | null;
}

/// The state of a live connection: still catching up, tailing the log, or stalled on an error (which
/// is transient — the tail keeps retrying, so a recovered agent returns to `live` on its own).
export type LiveStatus =
  | { status: "connecting" }
  | { status: "live" }
  | { status: "error"; message: string };

/// A live event log, tailed from a running agent. `replica` folds the same materializer the package
/// views use; `events` is the raw stream the Conversation and Events views render off; `head` is the
/// highest seq seen, which the timeline tracks as it grows. `progress` carries each conversation's
/// in-flight generation, populated only over the push channel — ephemeral display state, never part
/// of the log.
export interface LiveLog {
  replica: Replica | null;
  events: Event[];
  head: number;
  status: LiveStatus;
  progress: ReadonlyMap<string, InFlightGeneration>;
}

/// The polling cadence of the fallback tail, for a server (or proxy) whose event stream is
/// unavailable — the pre-push behaviour, kept whole.
const POLL_INTERVAL_MS = 1000;

/// How long to wait before reopening a dropped stream. A reconnect resumes `from=head + 1`, so a
/// drop costs latency, never events.
const STREAM_RETRY_MS = 2000;

/// Tail a running agent's event log: seed a [`Replica`] from an initial catch-up (`from=0`), then
/// follow the tail — over the server-sent-event stream (`/control/events/stream`), which also
/// carries the token-by-token turn progress, falling back to polling the catch-up endpoint when the
/// stream fails twice. `following` decides whether a new batch advances the fold horizon — held by
/// the parent so a batch arriving while the operator is time-travel pinned extends the log without
/// disturbing the graph they are inspecting (the fold is applied here, synchronously with the
/// append, so the views read a settled horizon on re-render).
export function useLiveLog(connection: LiveConnection, following: RefObject<boolean>): LiveLog {
  const [log, setLog] = useState<LiveLog>({
    replica: null,
    events: [],
    head: 0,
    status: { status: "connecting" },
    progress: new Map(),
  });

  // Re-fetch the whole log from scratch whenever the connection changes; a new agent is a new log.
  const { baseUrl, key } = connection;
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setInterval> | undefined;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let closeStream: (() => void) | undefined;
    let replica: Replica | null = null;
    let events: Event[] = [];
    let head = 0;
    // Stream failures seen since the last success; the second one demotes the tail to polling.
    let streamFailures = 0;
    // The in-flight generations by conversation id, mutated in place and re-wrapped per publish so
    // React sees a fresh reference.
    const progress = new Map<string, InFlightGeneration>();

    async function fetchFrom(from: number): Promise<Event[]> {
      // A bodyless GET, so just the bearer key — the effect depends on the primitive `key`/`baseUrl`
      // (not the `connection` object) so it re-runs only when the agent actually changes.
      const headers: HeadersInit = key ? { Authorization: `Bearer ${key}` } : {};
      const response = await fetch(`${baseUrl}/control/events?from=${from}`, { headers });
      if (!response.ok) throw new Error(await errorMessage(response));
      return (await response.json()) as Event[];
    }

    function publish(status: LiveStatus) {
      // A fresh handle so the views re-derive off the grown log without remounting (and losing the
      // open room); the underlying wasm replica is shared.
      setLog({
        replica: replica!.snapshot(),
        events,
        head,
        status,
        progress: new Map(progress),
      });
    }

    /// Fold a committed tail into the replica; returns whether anything arrived.
    function appendBatch(tail: Event[]): boolean {
      if (tail.length === 0) return false;
      replica!.append(tail);
      head = replica!.headSeq;
      if (following.current) replica!.foldTo(head);
      events = [...events, ...tail];
      for (const event of tail) {
        const superseded = supersededConversation(event);
        const current = superseded ? progress.get(superseded) : undefined;
        if (!superseded || !current) continue;
        const next = supersede(current, event);
        if (next) progress.set(superseded, next);
        else progress.delete(superseded);
      }
      return true;
    }

    function onProgress(frame: TurnProgress) {
      const next = foldFrame(progress.get(frame.conversation), frame);
      if (next) progress.set(frame.conversation, next);
      else progress.delete(frame.conversation);
      publish({ status: "live" });
    }

    function streamTail() {
      closeStream = openEventStream(baseUrl, key, head + 1, {
        // A successful open proves the endpoint works; only opens that never establish count
        // toward the polling demotion, so an idle-but-healthy stream dropped by a proxy timeout
        // reconnects forever rather than silently degrading.
        onOpen: () => {
          streamFailures = 0;
        },
        onEvents: (tail) => {
          if (cancelled) return;
          if (appendBatch(tail)) publish({ status: "live" });
        },
        onProgress: (frame) => {
          if (!cancelled) onProgress(frame);
        },
        onClose: (error) => {
          if (cancelled) return;
          // In-flight text is only trustworthy while connected; a dropped stream clears it rather
          // than freezing a stale fragment on screen.
          if (progress.size > 0) {
            progress.clear();
            publish({ status: "live" });
          }
          if (!error) {
            // A clean close is the server's deliberate reconnect cue (it ends the stream on
            // broadcast lag) or a proxy idle-timeout — never "done": resume from the horizon.
            retry = setTimeout(streamTail, STREAM_RETRY_MS);
            return;
          }
          streamFailures += 1;
          if (streamFailures >= 2) {
            // The stream keeps failing before establishing — a proxy that buffers SSE, an old
            // server. Poll instead; the log still tails, only the token progress is lost.
            timer = setInterval(poll, POLL_INTERVAL_MS);
          } else {
            retry = setTimeout(streamTail, STREAM_RETRY_MS);
          }
        },
      });
    }

    async function poll() {
      try {
        const tail = await fetchFrom(head + 1);
        if (cancelled) return;
        if (!appendBatch(tail)) {
          setLog((prev) =>
            prev.status.status === "live" ? prev : { ...prev, status: { status: "live" } },
          );
          return;
        }
        publish({ status: "live" });
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
        publish({ status: "live" });
        streamTail();
      } catch (cause) {
        if (!cancelled) setLog((prev) => ({ ...prev, status: errorStatus(cause) }));
      }
    })();

    return () => {
      cancelled = true;
      if (timer) clearInterval(timer);
      if (retry) clearTimeout(retry);
      closeStream?.();
    };
  }, [baseUrl, key, following]);

  return log;
}

function errorStatus(cause: unknown): LiveStatus {
  return { status: "error", message: cause instanceof Error ? cause.message : String(cause) };
}
