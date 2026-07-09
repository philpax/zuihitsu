import { EventRow } from "../../components/EventRow.tsx";

import type { TurnOutcome } from "../../lib/model/conversation.ts";

/// The trail of graph-mutating events a turn's Lua committed — the consequence of its deliberation,
/// shown wherever a turn renders. Each row is the one-line summary by default and expands, in place,
/// into the same specialized viewer the Events tab uses, so the exact write a turn made is one click
/// away in the transcript. `nameById` resolves the ids the viewer references; `className` tunes the
/// wrapper's spacing to the surface.
export function OutcomeList({
  outcomes,
  nameById,
  conversationNameById,
  className = "",
}: {
  outcomes: TurnOutcome[];
  nameById: Map<string, string>;
  conversationNameById: Map<string, string>;
  className?: string;
}) {
  return (
    <ul className={"flex flex-col " + className}>
      {outcomes.map((outcome) => (
        <EventRow
          key={outcome.seq}
          row={outcome}
          nameById={nameById}
          conversationNameById={conversationNameById}
        />
      ))}
    </ul>
  );
}
