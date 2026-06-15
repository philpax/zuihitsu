import { useState, type ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import type { Event } from "../types/Event.ts";
import type { Replica } from "../lib/replica.ts";
import type { LiveConnection } from "../lib/live.ts";
import { STREAM_VIEWS } from "../lib/streamViews.ts";
import { Timeline } from "./Timeline.tsx";
import { StateView } from "../views/StateView.tsx";
import { MemoryGraphView } from "../views/MemoryGraphView.tsx";
import { ConversationView } from "../views/ConversationView.tsx";
import { EventsView } from "../views/EventsView.tsx";
import { AgendaView } from "../views/AgendaView.tsx";
import { DiffView } from "../views/DiffView.tsx";

/// The views over a single event stream — the debugging surface shared by the eval and agent
/// frames. A run's embedded log and a live agent's tailed log are the same shape (one stream of
/// events folding into one graph), so the same views and the same global timeline serve both; the
/// Conversation view's sidebar reaches every conversation in the stream. The view list itself lives
/// in `lib/streamViews.ts`, so the eval frame can validate a URL against it without importing this
/// component.

/// What the agent frame passes so its Conversation view can also *speak*: the live connection and
/// the handle you converse under (lifted here so it survives view switches). Absent in the eval
/// frame, which is read-only. The workspace adds whether the cursor is at the head, since you may
/// speak into the present but a scrub back is read-only history.
export interface Participant {
  connection: LiveConnection;
  sender: string;
  setSender: (value: string) => void;
}

/// An agent-only view appended to the nav — a live tool that is not timeline-scoped (the operator Lua
/// console), so the scrubber steps aside while it is open. The eval frame passes none.
export interface ExtraView {
  id: string;
  label: string;
  node: ReactNode;
}

/// Drive the run-scoped views off one stream. The active view and the timeline cursor are *owned by
/// the caller* — both the eval and agent frames hold them in the URL, so browser back and forward
/// move between views and timeline positions the same way in either. `seq` is `null` to follow the
/// head (the latest state) or a number to pin an earlier seq; `head` is the stream's current head,
/// fixed for an eval run and growing for a live tail. `onFollowingChange` reports whether the cursor
/// tracks the head, so a live tail can fold a new batch to the head only while followed (and leave a
/// pinned graph undisturbed). `participant`, when present, makes the Conversation view interactive.
export function StreamWorkspace({
  replica,
  events,
  head,
  view,
  onSelectView,
  seq,
  onSeq,
  onFollowingChange,
  participant,
  extraViews = [],
}: {
  replica: Replica;
  events: Event[];
  head: number;
  view: string;
  onSelectView: (view: string) => void;
  seq: number | null;
  onSeq: (seq: number | null) => void;
  onFollowingChange?: (following: boolean) => void;
  participant?: Participant;
  extraViews?: ExtraView[];
}) {
  const cursor = seq ?? head;
  // The memory the Events view is pinned to, set by the State view's "events touching this" jump.
  const [eventFocus, setEventFocus] = useState<{ id: string; name: string } | null>(null);
  // Which way the next view slides: +1 from the right when moving rightward along the tabs, -1 from
  // the left when moving leftward. Computed at the switch (not from a lagging ref) so it stays within
  // the Rules of React; reduced-motion flattens the slide to a quick fade.
  const [direction, setDirection] = useState(1);
  const reduce = useReducedMotion();
  const shift = reduce ? 0 : 36;

  // Fold the graph to the cursor before the views below query it. The replica is an external mutable
  // store, not React state, so syncing it here — rather than in an effect that runs after the views
  // have already queried — keeps every query this render makes consistent with the cursor, with no
  // stale flash when a back/forward jump moves the cursor without a remount. The guard makes it
  // idempotent, so it is a no-op on the renders where the cursor did not move.
  if (replica.foldedSeq !== cursor) replica.foldTo(cursor);

  const tabs = [...STREAM_VIEWS.map((entry) => entry.id), ...extraViews.map((entry) => entry.id)];
  const extra = extraViews.find((entry) => entry.id === view);

  function selectView(next: string) {
    setDirection(tabs.indexOf(next) >= tabs.indexOf(view) ? 1 : -1);
    onSelectView(next);
  }

  // Jump from a memory in the State view to the events touching it, pinning the Events filter.
  function showEvents(id: string, name: string) {
    setEventFocus({ id, name });
    selectView("events");
  }

  function scrub(next: number) {
    const following = next >= head;
    onFollowingChange?.(following);
    onSeq(following ? null : next);
  }

  function reset() {
    onFollowingChange?.(true);
    onSeq(null);
  }

  return (
    <>
      <nav className="flex gap-7 overflow-x-auto border-b border-line text-sm">
        {[...STREAM_VIEWS, ...extraViews].map((entry) => (
          <button
            key={entry.id}
            onClick={() => selectView(entry.id)}
            className={
              "-mb-px shrink-0 whitespace-nowrap border-b-2 py-3 transition-colors " +
              (entry.id === view
                ? "border-clay text-ink"
                : "border-transparent text-ink-soft hover:text-ink")
            }
          >
            {entry.label}
          </button>
        ))}
      </nav>

      <main className="relative flex-1 overflow-x-clip py-6 sm:py-7">
        <AnimatePresence mode="popLayout" custom={direction} initial={false}>
          <motion.div
            key={view}
            custom={direction}
            initial={{ x: direction * shift, opacity: 0 }}
            animate={{ x: 0, opacity: 1 }}
            exit={{ x: direction * -shift, opacity: 0 }}
            transition={{ duration: reduce ? 0.12 : 0.3, ease: [0.32, 0.72, 0, 1] }}
          >
            {view === "state" && (
              <StateView
                replica={replica}
                events={events}
                cursor={cursor}
                onShowEvents={showEvents}
              />
            )}
            {view === "graph" && <MemoryGraphView key={cursor} replica={replica} cursor={cursor} />}
            {view === "conversation" && (
              <ConversationView
                replica={replica}
                events={events}
                cursor={cursor}
                participate={participant && { ...participant, atHead: cursor >= head }}
              />
            )}
            {view === "agenda" && <AgendaView replica={replica} events={events} cursor={cursor} />}
            {view === "events" && (
              <EventsView
                replica={replica}
                events={events}
                cursor={cursor}
                focus={eventFocus}
                onClearFocus={() => setEventFocus(null)}
              />
            )}
            {view === "compare" && <DiffView events={events} cursor={cursor} head={head} />}
            {extra?.node}
          </motion.div>
        </AnimatePresence>
      </main>

      {head > 0 && !extra && (
        <Timeline head={head} seq={cursor} events={events} onScrub={scrub} onReset={reset} />
      )}
    </>
  );
}
