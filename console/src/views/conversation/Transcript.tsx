import { useState } from "react";
import { useReducedMotion } from "motion/react";

import type { Replica } from "../../lib/replica/replica.ts";
import type { ConversationModel } from "../../lib/model/conversation.ts";
import { channelKey } from "./channelUtilities.tsx";
import { BriefBlock } from "./Brief.tsx";
import { SessionDivider } from "./channelUtilities.tsx";
import { TurnItem } from "./Turn.tsx";

export function Transcript({
  replica,
  conversation,
  cursor,
}: {
  replica: Replica;
  conversation: ConversationModel;
  cursor: number;
}) {
  // Turns already present when this conversation first rendered are the "initial state" and sit still;
  // turns that arrive afterwards — a live run streaming in — fade and slide in to signal the new state.
  const reduce = useReducedMotion();
  const [freshAfter] = useState(cursor);
  // The room key each turn's timestamp anchor bakes into its URL, so the pasted link reopens
  // this room before scrolling to the turn.
  const roomKey = channelKey(conversation.platform, conversation.scopePath);
  if (conversation.sessions.length === 0) {
    return (
      <ol className="flex flex-col">
        {conversation.turns.map((turn) => (
          <TurnItem
            key={turn.turnId}
            turn={turn}
            fresh={!reduce && turn.seq > freshAfter}
            roomKey={roomKey}
          />
        ))}
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
              {turns.map((turn) => (
                <TurnItem
                  key={turn.turnId}
                  turn={turn}
                  fresh={!reduce && turn.seq > freshAfter}
                  roomKey={roomKey}
                />
              ))}
            </ol>
          </div>
        );
      })}
    </>
  );
}
