import { type SessionModel } from "../../lib/model/conversation.ts";
import type { SessionEndCause } from "@zuihitsu/wire/types/SessionEndCause.ts";
import { formatDateTime } from "../../lib/format/format.ts";
import { LabeledDivider } from "../../components/primitives.tsx";

/// The seam label for a re-briefed session, named by how the *previous* session ended: a compaction
/// cut, an idle timeout, or a recovery close (issue #86). `null` — a pre-cause log — falls back to the
/// generic "continued".
function seamLabel(previousEndCause: SessionEndCause | null): string {
  // A pre-cause log records no cause — fall back to the generic "continued".
  if (previousEndCause === null) return "re-briefed · continued";
  switch (previousEndCause) {
    case "Compaction":
      return "re-briefed · compaction";
    case "Idle":
      return "re-briefed · idle gap";
    case "Recovery":
      return "re-briefed · recovered";
    default: {
      // Exhaustive over SessionEndCause: a new cause fails typecheck here until it is named.
      const unhandled: never = previousEndCause;
      return unhandled;
    }
  }
}

/// A divider between sessions in the transcript: a hairline rule with the session's start time and a
/// label that marks what kind of boundary it is. The first session in a conversation reads
/// "conversation" at the context's first and "new conversation" at each one after; a session that
/// reopened by re-segmenting the last (carrying a tail) reads "re-briefed · <seam>", the seam named by
/// the previous session's `endCause`.
export function SessionDivider({
  session,
  previousEndCause,
  first,
}: {
  session: SessionModel;
  previousEndCause: SessionEndCause | null;
  first: boolean;
}) {
  const label = session.seededFromTail
    ? seamLabel(previousEndCause)
    : first
      ? "conversation"
      : "new conversation";
  return (
    <LabeledDivider
      className={
        (first ? "mb-4 " : "my-4 ") + (session.seededFromTail ? "text-clay" : "text-ink-soft")
      }
    >
      <span className="tracking-widest uppercase">{label}</span>
      <span className="text-ink-faint">{formatDateTime(session.startedAt)}</span>
    </LabeledDivider>
  );
}
