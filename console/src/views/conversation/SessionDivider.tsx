import { type SessionModel } from "../../lib/model/conversation.ts";
import { formatDateTime } from "../../lib/format/format.ts";
import { LabeledDivider } from "../../components/primitives.tsx";

/// A divider between sessions in the transcript: a hairline rule with the session's start time and a
/// label that marks what kind of boundary it is. The first session in a conversation opened, reading
/// "conversation" at the context's first, "new conversation" at each one after, and "re-briefed ·
/// compaction" when a session reopened by re-segmenting the last rather than starting fresh.
export function SessionDivider({ session, first }: { session: SessionModel; first: boolean }) {
  const label = session.compaction
    ? "re-briefed · compaction"
    : first
      ? "conversation"
      : "new conversation";
  return (
    <LabeledDivider
      className={(first ? "mb-4 " : "my-4 ") + (session.compaction ? "text-clay" : "text-ink-soft")}
    >
      <span className="tracking-widest uppercase">{label}</span>
      <span className="text-ink-faint">{formatDateTime(session.startedAt)}</span>
    </LabeledDivider>
  );
}
