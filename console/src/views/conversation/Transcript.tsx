import { useState } from "react";
import { useReducedMotion } from "motion/react";

import type { Replica } from "../../lib/replica/replica.ts";
import { emptyTurn, type ConversationModel } from "../../lib/model/conversation.ts";
import { channelKey } from "./channelUtilities.tsx";
import { BriefBlock } from "./Brief.tsx";
import { SessionDivider } from "./channelUtilities.tsx";
import type { InFlightGeneration } from "../../lib/model/inflight.ts";
import { TurnItem } from "./Turn.tsx";

export function Transcript({
  replica,
  conversation,
  cursor,
  inflight,
}: {
  replica: Replica;
  conversation: ConversationModel;
  cursor: number;
  /// The room's in-flight generation (live mode). Attached to its turn's deliberation collapsible
  /// when that turn has committed steps; until the first commit, a quiet pending shell holds the
  /// tail position the real turn will take over — same place, so nothing shifts on commit.
  inflight?: InFlightGeneration | null;
}) {
  // Turns already present when this conversation first rendered are the "initial state" and sit still;
  // turns that arrive afterwards — a live run streaming in — fade and slide in to signal the new state.
  const reduce = useReducedMotion();
  const [freshAfter] = useState(cursor);
  // The room key each turn's timestamp anchor bakes into its URL, so the pasted link reopens
  // this room before scrolling to the turn.
  const roomKey = channelKey(conversation.platform, conversation.scopePath);
  // The in-flight generation belongs inside its own turn's collapsible. The fold materialises the
  // turn at its first committed deliberation event; before that the turn exists only as streamed
  // tokens, so the transcript starts it at the same point in the same lifecycle — `emptyTurn`, the
  // fold's own constructor — and holds the tail slot the materialised turn will take over. Rendered
  // through the same `TurnItem` under the same key, React preserves the component instance across
  // the handover, so the deliberation's own open state (per turn, in the turn view) survives
  // untouched — full visual continuity, no lifted state. Seq `0` because no event has committed:
  // the pending item is placed by hand at the tail, never ordered by seq.
  const inflightHasTurn =
    inflight != null && conversation.turns.some((turn) => turn.turnId === inflight.turnId);
  const pending =
    inflight && !inflightHasTurn ? (
      <TurnItem
        key={inflight.turnId}
        turn={emptyTurn(inflight.turnId, 0)}
        fresh={false}
        roomKey={roomKey}
        inflight={inflight}
      />
    ) : null;
  // The pending item must live in the SAME child array as the mapped turns: React matches keys only
  // among siblings within one array, so a `{pending}` expression beside `{turns.map(…)}` is a
  // different child slot, and the handover to the materialised turn would remount the item (dropping
  // the deliberation's open state) despite the identical key.
  if (conversation.sessions.length === 0) {
    return (
      <ol className="flex flex-col">
        {[
          ...conversation.turns.map((turn) => (
            <TurnItem
              key={turn.turnId}
              turn={turn}
              fresh={!reduce && turn.seq > freshAfter}
              roomKey={roomKey}
              inflight={inflight?.turnId === turn.turnId ? inflight : null}
            />
          )),
          pending,
        ]}
      </ol>
    );
  }
  return (
    <>
      {conversation.sessions.map((session, index) => {
        // Each session owns the turns from its open until the next session re-segments.
        const fromSeq = index === 0 ? 0 : session.seq;
        const toSeq = conversation.sessions[index + 1]?.seq ?? Infinity;
        const turns = conversation.turns.filter((turn) => turn.seq >= fromSeq && turn.seq < toSeq);
        return (
          <div key={session.id}>
            <SessionDivider session={session} first={index === 0} />
            <BriefBlock
              replica={replica}
              session={session}
              contextMemory={conversation.contextMemory}
            />
            <ol className="mt-2 flex flex-col">
              {[
                ...turns.map((turn) => (
                  <TurnItem
                    key={turn.turnId}
                    turn={turn}
                    fresh={!reduce && turn.seq > freshAfter}
                    roomKey={roomKey}
                    inflight={inflight?.turnId === turn.turnId ? inflight : null}
                  />
                )),
                index === conversation.sessions.length - 1 ? pending : null,
              ]}
            </ol>
          </div>
        );
      })}
    </>
  );
}
