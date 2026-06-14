import { useRef, useState } from "react";

import type { LiveConnection } from "../lib/live.ts";
import { useLiveLog } from "../lib/live.ts";
import { Dot, Eyebrow } from "./primitives.tsx";
import { Timeline } from "./Timeline.tsx";
import { StateView } from "../views/StateView.tsx";
import { ConversationView } from "../views/ConversationView.tsx";
import { EventsView } from "../views/EventsView.tsx";

/// The views reachable in live mode — the run-scoped trio, applied to the tailed log. Live mode has
/// no package or scenario layer, so the package-scoped Scenarios view does not appear.
const LIVE_VIEWS = [
  { id: "state", label: "State" },
  { id: "conversation", label: "Conversation" },
  { id: "events", label: "Events" },
] as const;

type LiveViewId = (typeof LIVE_VIEWS)[number]["id"];

/// The live frame: tail a running agent's event log and drive the run-scoped views off it, exactly
/// as the package frame drives them off an embedded log. The same global timeline applies — but its
/// head grows as events arrive. Following the head (the cursor unpinned) keeps the views on the
/// latest state; scrubbing back pins them while new events keep extending the timeline behind the
/// scrubber.
export function LiveShell({
  connection,
  onClose,
}: {
  connection: LiveConnection;
  onClose: () => void;
}) {
  const [view, setView] = useState<LiveViewId>("conversation");
  // null means "follow the head" — the latest state. A number pins the cursor to an earlier seq.
  const [seq, setSeq] = useState<number | null>(null);
  // Mirrors `seq === null` for the poll loop, which folds a new batch to the head only while
  // following — a ref so the long-lived interval reads the current value without re-subscribing.
  const following = useRef(true);
  const log = useLiveLog(connection, following);

  const replica = log.replica;
  const head = log.head;
  const cursor = seq ?? head;

  function scrub(next: number) {
    replica?.foldTo(next);
    following.current = next >= head;
    setSeq(next >= head ? null : next);
  }

  function reset() {
    replica?.foldTo(head);
    following.current = true;
    setSeq(null);
  }

  return (
    <div className="mx-auto flex min-h-screen max-w-[76rem] flex-col px-8">
      <header className="flex items-baseline justify-between border-b border-line py-6">
        <div className="flex items-baseline gap-3">
          <span className="font-serif text-xl text-ink">zuihitsu</span>
          <Eyebrow>console · live</Eyebrow>
        </div>
        <div className="flex items-baseline gap-3 font-mono text-xs text-ink-soft">
          <ConnectionBadge status={log.status} />
          <Dot />
          <span>{head} events</span>
          <button
            onClick={onClose}
            className="ml-1 text-ink-faint transition-colors hover:text-clay"
            title="Disconnect"
          >
            ✕
          </button>
        </div>
      </header>

      <nav className="flex gap-7 border-b border-line text-sm">
        {LIVE_VIEWS.map((entry) => (
          <button
            key={entry.id}
            onClick={() => setView(entry.id)}
            className={
              "-mb-px border-b-2 py-3 transition-colors " +
              (entry.id === view
                ? "border-clay text-ink"
                : "border-transparent text-ink-soft hover:text-ink")
            }
          >
            {entry.label}
          </button>
        ))}
      </nav>

      <main className="flex-1 py-10">
        {!replica ? (
          <Pending status={log.status} />
        ) : (
          <>
            {view === "state" && <StateView replica={replica} cursor={cursor} />}
            {view === "conversation" && (
              // The replica is mutated in place as the log tails (not rebuilt as in package mode), so
              // key the query-bearing views by the cursor: a followed batch advances it and forces a
              // fresh read, while a pinned cursor holds steady and leaves the view undisturbed.
              <ConversationView
                key={cursor}
                replica={replica}
                events={log.events}
                cursor={cursor}
              />
            )}
            {view === "events" && (
              <EventsView key={cursor} replica={replica} events={log.events} cursor={cursor} />
            )}
          </>
        )}
      </main>

      {replica && head > 0 && (
        <Timeline head={head} seq={cursor} events={log.events} onScrub={scrub} onReset={reset} />
      )}
    </div>
  );
}

function ConnectionBadge({ status }: { status: ReturnType<typeof useLiveLog>["status"] }) {
  if (status.status === "error") {
    return (
      <span className="text-clay" title={status.message}>
        ● disconnected
      </span>
    );
  }
  if (status.status === "connecting") {
    return <span className="text-ink-faint">○ connecting</span>;
  }
  return <span className="text-sage">● live</span>;
}

function Pending({ status }: { status: ReturnType<typeof useLiveLog>["status"] }) {
  if (status.status === "error") {
    return (
      <div className="py-24 text-center text-sm text-clay">
        Could not reach the agent — {status.message}
      </div>
    );
  }
  return <div className="py-24 text-center text-sm text-ink-faint">Connecting to the agent…</div>;
}
