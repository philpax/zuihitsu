import { useState } from "react";

import type { Event } from "../types/Event.ts";
import type { Replica } from "../lib/replica.ts";
import type { LiveConnection } from "../lib/live.ts";
import { Timeline } from "./Timeline.tsx";
import { StateView } from "../views/StateView.tsx";
import { ConversationView } from "../views/ConversationView.tsx";
import { EventsView } from "../views/EventsView.tsx";

/// The views over a single event stream — the debugging surface shared by the eval and agent
/// frames. A run's embedded log and a live agent's tailed log are the same shape (one stream of
/// events folding into one graph), so the same three views and the same global timeline serve both;
/// the Conversation view's sidebar reaches every conversation in the stream.
const STREAM_VIEWS = [
  { id: "state", label: "State" },
  { id: "conversation", label: "Conversation" },
  { id: "events", label: "Events" },
] as const;

export type StreamViewId = (typeof STREAM_VIEWS)[number]["id"];

/// What the agent frame passes so its Conversation view can also *speak*: the live connection and
/// the handle you converse under (lifted here so it survives view switches). Absent in the eval
/// frame, which is read-only. The workspace adds whether the cursor is at the head, since you may
/// speak into the present but a scrub back is read-only history.
export interface Participant {
  connection: LiveConnection;
  sender: string;
  setSender: (value: string) => void;
}

/// Drive the run-scoped views off one stream. Owns the view tab and the timeline cursor — `null`
/// follows the head (the latest state), a number pins an earlier seq — and folds the replica to the
/// cursor on a scrub. `head` is the stream's current head: fixed for an eval run, growing for a live
/// tail. `onFollowingChange` reports whether the cursor tracks the head, so a live tail can fold a
/// new batch to the head only while followed (and leave a pinned graph undisturbed). `participant`,
/// when present, makes the Conversation view interactive (the agent frame).
export function StreamWorkspace({
  replica,
  events,
  head,
  onFollowingChange,
  participant,
}: {
  replica: Replica;
  events: Event[];
  head: number;
  onFollowingChange?: (following: boolean) => void;
  participant?: Participant;
}) {
  const [view, setView] = useState<StreamViewId>("conversation");
  const [seq, setSeq] = useState<number | null>(null);
  const cursor = seq ?? head;

  function scrub(next: number) {
    replica.foldTo(next);
    const following = next >= head;
    onFollowingChange?.(following);
    setSeq(following ? null : next);
  }

  function reset() {
    replica.foldTo(head);
    onFollowingChange?.(true);
    setSeq(null);
  }

  return (
    <>
      <nav className="flex gap-7 border-b border-line text-sm">
        {STREAM_VIEWS.map((entry) => (
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
        {view === "state" && <StateView replica={replica} cursor={cursor} />}
        {view === "conversation" && (
          <ConversationView
            replica={replica}
            events={events}
            cursor={cursor}
            participate={participant && { ...participant, atHead: cursor >= head }}
          />
        )}
        {view === "events" && <EventsView replica={replica} events={events} cursor={cursor} />}
      </main>

      {head > 0 && (
        <Timeline head={head} seq={cursor} events={events} onScrub={scrub} onReset={reset} />
      )}
    </>
  );
}
