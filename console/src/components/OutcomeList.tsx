import type { TurnOutcome } from "../lib/conversation.ts";
import { CATEGORY_COLOR } from "../lib/events.ts";

/// The faint trail of graph-mutating events a turn's Lua committed — the consequence of its
/// deliberation, shown wherever a turn renders (the transcript and the imprint chat). The `className`
/// tunes the wrapper's spacing to the surface; each row reads the same.
export function OutcomeList({
  outcomes,
  className = "",
}: {
  outcomes: TurnOutcome[];
  className?: string;
}) {
  return (
    <ul className={"flex flex-col " + className}>
      {outcomes.map((outcome, index) => (
        <li key={index} className="flex items-baseline gap-2 font-mono text-2xs">
          <span className="text-ink-faint">↳</span>
          <span className={CATEGORY_COLOR[outcome.category]}>{outcome.type}</span>
          <span className="truncate text-ink-soft">{outcome.summary}</span>
        </li>
      ))}
    </ul>
  );
}
