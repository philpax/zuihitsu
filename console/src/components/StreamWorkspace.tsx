import { lazy, Suspense, useState, type ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import type { Event } from "../types/Event.ts";
import type { Replica } from "../lib/replica/replica.ts";
import type { StepRecord } from "../types/StepRecord.ts";
import type { LiveConnection } from "../lib/api/live.ts";
import { STREAM_VIEWS } from "../lib/nav/streamViews.ts";
import type { InFlightGeneration } from "../lib/model/inflight.ts";
import { DockContext } from "../lib/nav/dock.ts";
import { designatePrimary, editSelf, resolveMerge, unmerge } from "../lib/api/operator.ts";
import { Timeline } from "./Timeline.tsx";
import { StateView } from "../views/state/StateView.tsx";
import {
  ConversationNames,
  ConversationView,
  Names,
} from "../views/conversation/ConversationView.tsx";
import { EventsView } from "../views/EventsView.tsx";
import { AgendaView } from "../views/AgendaView.tsx";
import { BackgroundView } from "../views/BackgroundView.tsx";
import { DiffView } from "../views/DiffView.tsx";
import { nameById } from "../lib/model/labels.ts";
import { buildConversations } from "../lib/model/conversation.ts";
import { conversationNameById } from "./EventDetail.tsx";
import { channelKey } from "../views/conversation/channelUtilities.tsx";
import { type TurnRefTarget, TurnRefs } from "../lib/view/turnRefs.ts";

// The relations view pulls a force-graph/canvas library, so it loads only when the Relations tab is opened.
const RelationsView = lazy(() =>
  import("../views/relations/RelationsView.tsx").then((module) => ({
    default: module.RelationsView,
  })),
);

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
  journal,
  resumedFromStep,
  progress,
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
  // The eval run's step journal, forwarded to the Events view; a live tail passes none, so its stream
  // renders without step markers. `resumedFromStep`, when set, marks a resumed run's live boundary.
  journal?: readonly StepRecord[];
  resumedFromStep?: number | null;
  /// Each conversation's in-flight generation from the live push channel — ephemeral display state
  /// the Conversation view renders at the transcript tail. Absent in the read-only viewers.
  progress?: ReadonlyMap<string, InFlightGeneration>;
}) {
  const cursor = seq ?? head;

  // Shared context maps built once per render, available to all views so `ConversationRefLink`
  // and `TurnRefChip` work everywhere — not just inside the Conversation view.
  const names = nameById(replica.memories(""));
  const convNames = conversationNameById(replica.conversations());
  const conversations = buildConversations(
    events.filter((event) => event.seq <= cursor),
    names,
  );
  const refTargets = new Map<string, TurnRefTarget>();
  for (const conversation of conversations) {
    const roomKey = channelKey(conversation.platform, conversation.scopePath);
    conversation.turns.forEach((turn, index) => {
      refTargets.set(turn.turnId, {
        turn,
        roomKey,
        window: conversation.turns.slice(Math.max(0, index - 2), index + 3),
        focusIndex: Math.min(index, 2),
      });
    });
  }

  // The bottom dock a view can float its controls into (the conversation composer), held as state
  // rather than a ref so the provider re-renders its consumers once the element lands.
  const [dock, setDock] = useState<HTMLDivElement | null>(null);
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
                ? "border-clay font-medium text-ink"
                : "border-transparent text-ink-soft hover:text-ink")
            }
          >
            {entry.label}
          </button>
        ))}
      </nav>

      <main className="relative flex-1 overflow-x-clip py-4">
        <Names.Provider value={names}>
          <ConversationNames.Provider value={convNames}>
            <TurnRefs.Provider value={refTargets}>
              <DockContext.Provider value={dock}>
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
                        onEditSelf={
                          participant && cursor >= head
                            ? (text, supersedes) =>
                                editSelf(participant.connection, text, supersedes).then(() => {})
                            : undefined
                        }
                      />
                    )}
                    {view === "relations" && (
                      <Suspense
                        fallback={
                          <div className="py-16 text-center text-sm text-ink-faint">
                            Loading relations…
                          </div>
                        }
                      >
                        <RelationsView
                          key={cursor}
                          replica={replica}
                          cursor={cursor}
                          merge={
                            participant && cursor >= head
                              ? {
                                  resolve: (from, to, accept) =>
                                    resolveMerge(participant.connection, from, to, accept),
                                  unmerge: (from, to) => unmerge(participant.connection, from, to),
                                  designatePrimary: (memory, designated) =>
                                    designatePrimary(participant.connection, memory, designated),
                                }
                              : undefined
                          }
                        />
                      </Suspense>
                    )}
                    {view === "conversation" && (
                      <ConversationView
                        replica={replica}
                        events={events}
                        cursor={cursor}
                        participate={participant && { ...participant, atHead: cursor >= head }}
                        progress={progress}
                      />
                    )}
                    {view === "agenda" && (
                      <AgendaView replica={replica} events={events} cursor={cursor} />
                    )}
                    {view === "background" && (
                      <BackgroundView replica={replica} events={events} cursor={cursor} />
                    )}
                    {view === "events" && (
                      <EventsView
                        replica={replica}
                        events={events}
                        cursor={cursor}
                        journal={journal}
                        resumedFromStep={resumedFromStep}
                      />
                    )}
                    {view === "compare" && <DiffView events={events} cursor={cursor} head={head} />}
                    {extra?.node}
                  </motion.div>
                </AnimatePresence>
              </DockContext.Provider>
            </TurnRefs.Provider>
          </ConversationNames.Provider>
        </Names.Provider>
      </main>

      {/* The sticky bottom chrome: the dock (a view's floating controls) stacked over the global
          timeline, in one region so they never fight for the same edge. */}
      <footer className="sticky bottom-0 z-10 bg-paper/95 backdrop-blur">
        <div ref={setDock} />
        {head > 0 && !extra && (
          <Timeline head={head} seq={cursor} events={events} onScrub={scrub} onReset={reset} />
        )}
      </footer>
    </>
  );
}
