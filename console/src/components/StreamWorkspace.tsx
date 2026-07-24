import { lazy, Suspense, useState, type ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { Replica } from "../lib/replica/replica.ts";
import type { StepRecord } from "@zuihitsu/wire/types/StepRecord.ts";
import type { LiveConnection } from "../lib/api/live.ts";
import { STREAM_VIEWS, type AgentViewId, type ViewId } from "../lib/nav/streamViews.ts";
import type { InFlightGeneration } from "../lib/model/inflight.ts";
import { DockContext } from "../lib/nav/dock.ts";
import { ScrollContainer } from "../lib/nav/scrollContainer.ts";
import {
  designatePrimary,
  editSelf,
  confirmMerge,
  retractEntry,
  unmerge,
} from "../lib/api/operator.ts";
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
import { conversationNameById } from "../lib/model/conversationNameById.ts";
import { channelKey } from "../views/conversation/channelUtilities.ts";
import { type TurnRefTarget, TurnRefs } from "../lib/view/turnRefs.ts";
import { type MemRefResolver, MemRefs } from "../lib/view/memRefs.ts";

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
/// frame, which is read-only. Whether the cursor is at the head — the gate on speaking into the
/// present — rides the view's own `atHead` prop, since the eval frame follows the tail at its head too.
export interface Participant {
  connection: LiveConnection;
  sender: string;
  setSender: (value: string) => void;
}

/// An agent-only view appended to the nav — a live tool that is not timeline-scoped (the operator Lua
/// console), so the scrubber steps aside while it is open. The eval frame passes none.
export interface ExtraView {
  id: AgentViewId;
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
  view: ViewId;
  onSelectView: (view: ViewId) => void;
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

  // Fold the graph to the cursor before anything queries it — the shared maps just below and the
  // views further down. The replica is an external mutable store, not React state, so syncing it here
  // (rather than in an effect that runs after the queries) keeps every read this render makes
  // consistent with the cursor: the conversation existence set the shared maps derive is gated on the
  // graph, so a stale horizon would flicker conversations in and out during a time-travel scrub. The
  // guard makes it idempotent — a no-op on renders where the cursor did not move.
  if (replica.foldedSeq !== cursor) replica.foldTo(cursor);

  // Shared context maps built once per render, available to all views so `ConversationRefLink`
  // and `TurnRefChip` work everywhere — not just inside the Conversation view.
  const names = nameById(replica.memories(""));
  const conversationList = replica.conversations();
  const convNames = conversationNameById(conversationList);
  // The graph's live conversations: a room deleted via `delete-memory` is already dropped from the
  // projection, so the transcript shows only conversations the graph still holds.
  const liveConversationIds = new Set(conversationList.map((conv) => conv.id));
  const conversations = buildConversations(
    events.filter((event) => event.seq <= cursor),
    names,
    liveConversationIds,
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
  // A memory-reference resolver over the replica at the current fold — the transcript's `MemRefChip`
  // resolves each reference through this, so a chip reads against exactly what the timeline shows (the
  // replica is already folded to the cursor above). `byId` collapses a scanned reference's id to its
  // class primary; `byHandle` resolves a matched State-view URL's handle to an id (by current name, then
  // a former-name alias, matching the composer) and collapses that through the same class primary, so a
  // handle and an id for the same memory land on one chip.
  const memRefs: MemRefResolver = {
    byId: (id) => replica.resolveMemRef(id),
    byHandle: (handle) => {
      const id = replica.memoryIdByName(handle) ?? replica.memoryIdForFormerName(handle);
      return id === null ? null : replica.resolveMemRef(id);
    },
    // The chip's hover preview: called when a preview opens, not per render, since the full memory
    // read composes several graph queries. Returns the memory's description and its most recent few
    // entries (in commit order), or `null` when the handle names no memory at this fold horizon.
    preview: (handle) => {
      const detail = replica.memory(handle);
      if (detail === null) return null;
      return {
        description: detail.memory.description,
        entries: detail.entries
          .slice(-3)
          .map((entry) => ({ id: entry.entry_id, text: entry.text })),
      };
    },
  };

  // The bottom dock a view can float its controls into (the conversation composer), held as state
  // rather than a ref so the provider re-renders its consumers once the element lands.
  const [dock, setDock] = useState<HTMLDivElement | null>(null);
  // The scrolling content well — the `<main>` below, offered to the views that manage their own
  // scroll (the Conversation transcript and the Events virtualizer). Held as state, not a ref, so a
  // consumer's effects re-run once the element mounts and the context value changes from null to it.
  const [well, setWell] = useState<HTMLElement | null>(null);
  // Which way the next view slides: +1 from the right when moving rightward along the tabs, -1 from
  // the left when moving leftward. Computed at the switch (not from a lagging ref) so it stays within
  // the Rules of React; reduced-motion flattens the slide to a quick fade.
  const [direction, setDirection] = useState(1);
  const reduce = useReducedMotion();
  const shift = reduce ? 0 : 36;

  const tabs = [...STREAM_VIEWS.map((entry) => entry.id), ...extraViews.map((entry) => entry.id)];
  const extra = extraViews.find((entry) => entry.id === view);

  function selectView(next: ViewId) {
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
    <ScrollContainer.Provider value={well}>
      <nav className="flex shrink-0 gap-7 overflow-x-auto border-b border-line text-sm">
        {[...STREAM_VIEWS, ...extraViews].map((entry) => (
          <button
            key={entry.id}
            onClick={() => selectView(entry.id)}
            className={
              "-mb-px shrink-0 border-b-2 py-3 whitespace-nowrap transition-colors " +
              (entry.id === view
                ? "border-clay font-medium text-ink"
                : "border-transparent text-ink-soft hover:text-ink")
            }
          >
            {entry.label}
          </button>
        ))}
      </nav>

      <main ref={setWell} className="relative min-h-0 flex-1 overflow-x-clip overflow-y-auto py-4">
        <Names.Provider value={names}>
          <ConversationNames.Provider value={convNames}>
            <TurnRefs.Provider value={refTargets}>
              <MemRefs.Provider value={memRefs}>
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
                          onRetract={
                            participant && cursor >= head
                              ? (memory, entry, reason) =>
                                  retractEntry(participant.connection, memory, entry, reason)
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
                                    resolve: (from, to) =>
                                      confirmMerge(participant.connection, from, to),
                                    unmerge: (from, to) =>
                                      unmerge(participant.connection, from, to),
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
                          atHead={cursor >= head}
                          participate={participant}
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
                      {view === "compare" && (
                        <DiffView events={events} cursor={cursor} head={head} />
                      )}
                      {extra?.node}
                    </motion.div>
                  </AnimatePresence>
                </DockContext.Provider>
              </MemRefs.Provider>
            </TurnRefs.Provider>
          </ConversationNames.Provider>
        </Names.Provider>
      </main>

      {/* The fixed bottom chrome: the dock (a view's floating controls, the conversation composer)
          stacked over the global timeline, in one region so they never fight for the same edge. It
          sits below the scrolling well as a non-shrinking flex child, so it stays put while the view
          scrolls between it and the nav. */}
      <footer className="shrink-0 border-t border-line bg-paper/95 backdrop-blur-sm">
        <div ref={setDock} />
        {head > 0 && !extra && (
          <Timeline head={head} seq={cursor} events={events} onScrub={scrub} onReset={reset} />
        )}
      </footer>
    </ScrollContainer.Provider>
  );
}
