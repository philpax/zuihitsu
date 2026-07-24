import { useState } from "react";
import { useReducedMotion } from "motion/react";

import type { Replica } from "../../lib/replica/replica.ts";
import { emptyTurn, type ConversationModel, type TurnModel } from "../../lib/model/conversation.ts";
import { channelKey } from "./channelUtilities.ts";
import { BriefBlock } from "./Brief.tsx";
import { SessionDivider } from "./SessionDivider.tsx";
import type { InFlightGeneration } from "../../lib/model/inflight.ts";
import type { TurnWindow } from "./transcriptWindowUtilities.ts";
import { TurnItem } from "./Turn.tsx";

export function Transcript({
  replica,
  conversation,
  cursor,
  inflight,
  window: win,
  topRef,
  bottomRef,
}: {
  replica: Replica;
  conversation: ConversationModel;
  cursor: number;
  /// The room's in-flight generation (live mode). Attached to its turn's deliberation collapsible
  /// when that turn has committed steps; until the first commit, a quiet pending shell holds the
  /// tail position the real turn will take over — same place, so nothing shifts on commit.
  inflight?: InFlightGeneration | null;
  /// The render window over `conversation.turns` — only this index range is rendered, so a
  /// thousand-turn history never lands in the DOM at once. `null` (or absent) renders every turn, the
  /// behaviour a read-only eval run and the handover tests rely on.
  window?: TurnWindow | null;
  /// Sentinels the scroll hook observes to page the window in and out: `topRef` mounts at the loaded
  /// range's head (rendered only when older turns remain above), `bottomRef` at its foot (rendered
  /// only when newer turns remain below).
  topRef?: (node: HTMLElement | null) => void;
  bottomRef?: (node: HTMLElement | null) => void;
}) {
  // Turns already present when this conversation first rendered are the "initial state" and sit still;
  // turns that arrive afterwards — a live run streaming in — fade and slide in to signal the new state.
  const reduce = useReducedMotion();
  const [freshAfter] = useState(cursor);
  // The room key each turn's timestamp anchor bakes into its URL, so the pasted link reopens
  // this room before scrolling to the turn.
  const roomKey = channelKey(conversation.platform, conversation.scopePath);
  const total = conversation.turns.length;
  // The active window over the flat turns array. Absent (eval, tests) means "the whole transcript".
  const range: TurnWindow = win ?? { start: 0, end: total };
  const atTail = range.end >= total;
  // Each turn's index in the flat, seq-ordered array — the coordinate the window is expressed in, so a
  // session's turns can be filtered against it without re-deriving positions per session.
  const indexById = new Map(conversation.turns.map((turn, index) => [turn.turnId, index]));
  const inWindow = (turn: TurnModel) => {
    const index = indexById.get(turn.turnId) ?? 0;
    return index >= range.start && index < range.end;
  };

  const topSentinel =
    range.start > 0 && topRef ? (
      <li key="top-sentinel" aria-hidden ref={topRef} className="h-px" />
    ) : null;
  const bottomSentinel =
    !atTail && bottomRef ? (
      <li key="bottom-sentinel" aria-hidden ref={bottomRef} className="h-px" />
    ) : null;

  // The in-flight generation belongs inside its own turn's collapsible. The fold materialises the
  // turn at its first committed deliberation event; before that the turn exists only as streamed
  // tokens, so the transcript starts it at the same point in the same lifecycle — `emptyTurn`, the
  // fold's own constructor — and holds the tail slot the materialised turn will take over. Rendered
  // through the same `TurnItem` under the same key, React preserves the component instance across
  // the handover, so the deliberation's own open state (per turn, in the turn view) survives
  // untouched — full visual continuity, no lifted state. Seq `0` because no event has committed:
  // the pending item is placed by hand at the tail, never ordered by seq. It shows only when the tail
  // is in the window (a reader scrolled up into history has paged it out).
  const inflightHasTurn =
    inflight != null && conversation.turns.some((turn) => turn.turnId === inflight.turnId);
  const pending =
    inflight && !inflightHasTurn && atTail ? (
      <TurnItem
        key={inflight.turnId}
        turn={emptyTurn(inflight.turnId, 0)}
        fresh={false}
        roomKey={roomKey}
        inflight={inflight}
      />
    ) : null;

  const item = (turn: TurnModel) => (
    <TurnItem
      key={turn.turnId}
      turn={turn}
      fresh={!reduce && turn.seq > freshAfter}
      roomKey={roomKey}
      inflight={inflight?.turnId === turn.turnId ? inflight : null}
    />
  );

  // The pending item must live in the SAME child array as the mapped turns: React matches keys only
  // among siblings within one array, so a `{pending}` expression beside `{turns.map(…)}` is a
  // different child slot, and the handover to the materialised turn would remount the item (dropping
  // the deliberation's open state) despite the identical key.
  if (conversation.sessions.length === 0) {
    return (
      <ol className="flex flex-col">
        {[topSentinel, ...conversation.turns.filter(inWindow).map(item), pending, bottomSentinel]}
      </ol>
    );
  }
  return (
    <>
      {topSentinel && <ol className="flex flex-col">{topSentinel}</ol>}
      {conversation.sessions.map((session, index) => {
        // Each session owns the turns from its open until the next session re-segments.
        const fromSeq = index === 0 ? 0 : session.seq;
        const toSeq = conversation.sessions[index + 1]?.seq ?? Infinity;
        const isLast = index === conversation.sessions.length - 1;
        const turns = conversation.turns.filter(
          (turn) => turn.seq >= fromSeq && turn.seq < toSeq && inWindow(turn),
        );
        // Skip a session with no turns in the window, so paging out its turns pages out its divider and
        // brief too — except the last session at the tail, which must render to home the pending item.
        if (turns.length === 0 && !(isLast && atTail && pending)) return null;
        return (
          <div key={session.id}>
            <SessionDivider
              session={session}
              previousEndCause={conversation.sessions[index - 1]?.endCause ?? null}
              first={index === 0}
            />
            <BriefBlock
              replica={replica}
              session={session}
              contextMemory={conversation.contextMemory}
            />
            <ol className="mt-2 flex flex-col">
              {[...turns.map(item), isLast && atTail ? pending : null]}
            </ol>
          </div>
        );
      })}
      {bottomSentinel && <ol className="flex flex-col">{bottomSentinel}</ol>}
    </>
  );
}
