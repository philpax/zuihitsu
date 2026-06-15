import { useState } from "react";

import type { TurnOutcome } from "../lib/conversation.ts";
import { CATEGORY_COLOR } from "../lib/events.ts";
import { useStreamBase } from "../lib/useStreamLocation.ts";
import { EventDetail } from "../views/EventDetail.tsx";

/// The trail of graph-mutating events a turn's Lua committed — the consequence of its deliberation,
/// shown wherever a turn renders. Each row is the one-line summary by default and expands, in place,
/// into the same specialized viewer the Events tab uses, so the exact write a turn made is one click
/// away in the transcript. `nameById` resolves the ids the viewer references; `className` tunes the
/// wrapper's spacing to the surface.
export function OutcomeList({
  outcomes,
  nameById,
  className = "",
}: {
  outcomes: TurnOutcome[];
  nameById: Map<string, string>;
  className?: string;
}) {
  return (
    <ul className={"flex flex-col " + className}>
      {outcomes.map((outcome) => (
        <OutcomeRow key={outcome.seq} outcome={outcome} nameById={nameById} />
      ))}
    </ul>
  );
}

function OutcomeRow({
  outcome,
  nameById,
}: {
  outcome: TurnOutcome;
  nameById: Map<string, string>;
}) {
  const [open, setOpen] = useState(false);
  const base = useStreamBase();
  return (
    <li className="font-mono text-2xs">
      <button
        onClick={() => setOpen(!open)}
        className="group flex w-full items-baseline gap-2 text-left"
        title={open ? "Collapse" : "Expand the event"}
      >
        <span className="text-ink-faint">↳</span>
        <span className={CATEGORY_COLOR[outcome.category]}>{outcome.type}</span>
        <span
          className={
            "truncate transition-colors " +
            (open ? "text-ink" : "text-ink-soft group-hover:text-ink")
          }
        >
          {outcome.summary}
        </span>
      </button>
      {open && (
        <div className="mb-1 ml-4 mt-1 border-l-2 border-line py-1 pl-3">
          <EventDetail
            payload={outcome.payload}
            nameById={nameById}
            base={base}
            seq={outcome.seq}
          />
        </div>
      )}
    </li>
  );
}
