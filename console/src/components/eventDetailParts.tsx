import { Fragment, type ReactNode, useContext } from "react";

import type { ConversationRef } from "@zuihitsu/wire/types/ConversationRef.ts";
import { refName } from "../lib/model/events.ts";
import { Link } from "../lib/nav/history.tsx";
import { useOptionalStream } from "../lib/nav/useStreamLocation.ts";
import { Excerpt } from "../components/primitives.tsx";
import { TurnRefs } from "../lib/view/turnRefs.ts";
import { TurnRefChip } from "../views/conversation/TurnRefs.tsx";

/// A memory reference: the memory's name, a link into the State view at this event's seq when the
/// enclosing stream frame and the `seq` are known and the id names a memory, plain text otherwise.
export function Ref({
  id,
  nameById,
  seq,
}: {
  id: string;
  nameById: Map<string, string>;
  seq?: number;
}) {
  const name = refName(id, nameById);
  // Only link when the id resolves to a known memory name (nameById has it).
  if (!nameById.has(id)) return <>{name}</>;
  return <MemoryNameLink name={name} seq={seq} />;
}

/// A conversation reference rendered as a link, styled like [`Ref`]: the room's name (resolved
/// from `conversationNameById`) as the label, linking to the conversation view at the turn. When
/// the `TurnRefs` context is available (inside the conversation view), delegates to `TurnRefChip`
/// for the full speaker-label + hover-preview chip.
export function ConversationRefLink({
  value,
  nameById,
  conversationNameById,
  seq,
}: {
  value: ConversationRef;
  nameById: Map<string, string>;
  conversationNameById: Map<string, string>;
  seq?: number;
}) {
  const targets = useContext(TurnRefs);
  const stream = useOptionalStream();
  if (value.turn) {
    // Inside the conversation view with the turn in the folded set, use the full chip
    // (speaker label, hover preview).
    if (targets.has(value.turn)) {
      return <TurnRefChip id={value.turn} />;
    }
    // Outside the conversation view, or the turn is not in the folded set (a background-pass
    // turn, or past the timeline cursor): link to the conversation view with the turn pinned.
    const roomName = conversationNameById.get(value.conversation) ?? value.conversation;
    // No room segment: a background-pass turn, or one past the cursor, is deep-linked by turn alone,
    // and the Conversation view resolves the room that holds it. Outside a stream frame there is
    // nowhere to link to, so the room name renders as plain text.
    return stream ? (
      <Link
        to={stream.link.conversation({ turn: value.turn })}
        title={`Open this turn in ${roomName}`}
        className="text-clay underline-offset-2 transition-colors hover:text-ink hover:underline"
      >
        {roomName}
      </Link>
    ) : (
      <>{roomName}</>
    );
  }
  // No turn: the reference is the room itself — render as a memory Ref if the context
  // memory is known, otherwise plain text.
  const roomName = conversationNameById.get(value.conversation);
  if (roomName && nameById.has(roomName)) {
    return <Ref id={roomName} nameById={nameById} seq={seq} />;
  }
  return <>{roomName ?? value.conversation}</>;
}

/// A clickable memory name that navigates to the State view at the cursor, rendered as a semantic
/// `<Link>`. Handles virtual nodes (collapsed `same_as` classes ending in " (N)") as plain text.
/// Shared by the event detail panels, the relations view, the join brief, and the merge proposals.
export function MemoryNameLink({ name, seq }: { name: string; seq?: number }) {
  const stream = useOptionalStream();
  // A collapsed virtual node id ends in " (N)" — it is a class, not a single memory to open.
  if (/\(\d+\)$/.test(name)) {
    return <span className="text-sage">{name}</span>;
  }
  // Link only inside a stream frame and at a pinned seq; outside one the name renders as plain text.
  if (!stream || seq == null) return <>{name}</>;
  return (
    <Link
      to={stream.link.state(name, { seq })}
      title={`Open ${name} in State`}
      className="text-clay underline-offset-2 transition-colors hover:text-ink hover:underline"
    >
      {name}
    </Link>
  );
}

/// A comma-separated list of memory references, each a link under the same rules as [`Ref`].
export function RefList({
  ids,
  nameById,
  seq,
  empty = "—",
}: {
  ids: string[];
  nameById: Map<string, string>;
  seq?: number;
  empty?: string;
}) {
  if (ids.length === 0) return <>{empty}</>;
  return (
    <>
      {ids.map((id, index) => (
        <Fragment key={index}>
          {index > 0 && ", "}
          <Ref id={id} nameById={nameById} seq={seq} />
        </Fragment>
      ))}
    </>
  );
}

export function Mono({ children }: { children: ReactNode }) {
  return <span className="break-all text-ink-soft">{children}</span>;
}

/// A long text body (a brief, a prompt template) — the content itself, not a JSON dump.
export function Prose({ children }: { children: string }) {
  return <Excerpt>{children}</Excerpt>;
}
