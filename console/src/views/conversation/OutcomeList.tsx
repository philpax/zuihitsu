import { useState } from "react";

import { EventDetail } from "../../components/EventDetail.tsx";
import { LogEventRow } from "../../components/LogEventRow.tsx";

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
    <div className={"flex flex-col " + className}>
      {outcomes.map((outcome) => (
        <OutcomeRow
          key={outcome.seq}
          outcome={outcome}
          nameById={nameById}
          conversationNameById={conversationNameById}
        />
      ))}
    </div>
  );
}

/// One outcome row: the shared [`LogEventRow`] in its compact inline variant, holding its own
/// expansion state so each write in a turn's trail opens independently into the [`EventDetail`]
/// viewer the Events tab uses.
function OutcomeRow({
  outcome,
  nameById,
  conversationNameById,
}: {
  outcome: TurnOutcome;
  nameById: Map<string, string>;
  conversationNameById: Map<string, string>;
}) {
  const [open, setOpen] = useState(false);
  return (
    <LogEventRow
      compact
      type={outcome.type}
      category={outcome.category}
      summary={outcome.summary}
      open={open}
      onToggle={() => setOpen(!open)}
    >
      <EventDetail
        payload={outcome.payload}
        nameById={nameById}
        conversationNameById={conversationNameById}
        recordedAt={outcome.recordedAt}
        source={outcome.source}
      />
    </LogEventRow>
  );
}
