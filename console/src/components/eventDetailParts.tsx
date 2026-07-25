import { Fragment, type ReactNode, useContext } from "react";

import type { ConversationRef } from "@zuihitsu/wire/types/ConversationRef.ts";
import { refName } from "../lib/model/events.ts";
import { Link } from "../lib/nav/history.tsx";
import { useOptionalStream } from "../lib/nav/useStreamLocation.ts";
import { Excerpt } from "../components/primitives.tsx";
import { TurnRefs } from "../lib/view/turnRefs.ts";
import { MemRefs } from "../lib/view/memRefs.ts";
import { EntryEvents } from "../lib/view/entryEvents.ts";
import { TurnRefChip } from "../views/conversation/TurnRefs.tsx";

// The shared reference family for event details — one component per referent kind, so every raw id an
// event payload carries renders as a link to where that thing lives rather than a bare ULID:
//   - memory by id: [`Ref`] / [`RefList`]; memory by name: [`MemoryNameLink`],
//   - content entry: [`EntryRef`] (snippet-labelled, into State with the entry highlighted),
//   - conversation or turn: [`ConversationRefLink`] (delegating to the transcript's chip in-view),
//   - log event by seq: [`EventRef`] (into the Events view, that event pinned).
// Each resolves against the folded log the console holds and degrades to plain text — never a broken
// link — outside a stream frame or when the referent is not in the loaded window. [`Mono`] and
// [`Prose`] are the primitive companions the same detail renderers reach for.

/// A memory reference: the memory's name, linking into the State view when the enclosing stream
/// frame is known and the id names a memory, plain text otherwise. Links never mint a cursor pin —
/// the stream's own pinned seq (a deliberate scrub) rides along, and nothing else does.
///
/// The reference resolves through its `same_as` class primary (the [`MemRefs`] resolver the
/// workspace fills at the current fold): the canonical handle renders first, and when the recorded
/// handle differs — a stub bound into a merged identity — it dims beside the canonical one, both
/// clickable to their own memories, matching the transcript speaker treatment. Outside a resolver
/// (no provider) the reference falls back to the plain recorded link.
export function Ref({ id, nameById }: { id: string; nameById: Map<string, string> }) {
  const recorded = refName(id, nameById);
  // Only link when the id resolves to a known memory name (nameById has it); the same known-name gate
  // guards the dimmed recorded handle below, so a paired render never links to a bare id.
  const known = nameById.has(id);
  const canonical = useContext(MemRefs).byId(id)?.handle;
  if (canonical && known && canonical !== recorded) {
    return (
      <>
        <MemoryNameLink name={canonical} />
        <span className="text-ink-faint"> · </span>
        <MemoryNameLink name={recorded} dim />
      </>
    );
  }
  // No class, or the recorded handle already is the class primary: the plain single link.
  if (!known) return <>{recorded}</>;
  return <MemoryNameLink name={recorded} />;
}

/// A conversation reference rendered as a link, styled like [`Ref`]: the room's name (resolved
/// from `conversationNameById`) as the label, linking to the conversation view at the turn. When
/// the `TurnRefs` context is available (inside the conversation view), delegates to `TurnRefChip`
/// for the full speaker-label + hover-preview chip.
export function ConversationRefLink({
  value,
  nameById,
  conversationNameById,
}: {
  value: ConversationRef;
  nameById: Map<string, string>;
  conversationNameById: Map<string, string>;
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
    return <Ref id={roomName} nameById={nameById} />;
  }
  return <>{roomName ?? value.conversation}</>;
}

/// A clickable memory name that navigates to the State view, rendered as a semantic
/// `<Link>`. Handles virtual nodes (collapsed `same_as` classes ending in " (N)") as plain text.
/// Shared by the event detail panels, the relations view, the join brief, and the merge proposals.
/// `dim` renders it in faint ink rather than clay — the register [`Ref`] gives a recorded handle
/// paired beside its canonical primary.
export function MemoryNameLink({ name, dim = false }: { name: string; dim?: boolean }) {
  const stream = useOptionalStream();
  // A collapsed virtual node id ends in " (N)": a same_as class, named for its representative
  // member. It still links — to that representative's memory, where the class peers are listed —
  // keeping the sage class tint and the member count.
  const virtual = /^(.*) \(\d+\)$/.exec(name);
  const tint = dim ? "text-ink-faint" : "text-clay";
  if (virtual && stream) {
    return (
      <Link
        to={stream.link.state(virtual[1], stream.seq != null ? { seq: stream.seq } : undefined)}
        title={`Open ${virtual[1]} in State`}
        className={
          (dim ? "text-ink-faint" : "text-sage") +
          " underline-offset-2 transition-colors hover:text-ink hover:underline"
        }
      >
        {name}
      </Link>
    );
  }
  if (virtual) {
    return <span className={dim ? "text-ink-faint" : "text-sage"}>{name}</span>;
  }
  // Link anywhere inside a stream frame. The URL carries a cursor only when the stream itself is
  // pinned (the operator deliberately scrubbed): a link never introduces a pin of its own, so
  // following references keeps the world at the head unless the operator chose otherwise.
  if (!stream) return <>{name}</>;
  const pinned = stream.seq;
  return (
    <Link
      to={stream.link.state(name, pinned != null ? { seq: pinned } : undefined)}
      title={`Open ${name} in State`}
      className={tint + " underline-offset-2 transition-colors hover:text-ink hover:underline"}
    >
      {name}
    </Link>
  );
}

/// A comma-separated list of memory references, each a link under the same rules as [`Ref`].
export function RefList({
  ids,
  nameById,
  empty = "—",
}: {
  ids: string[];
  nameById: Map<string, string>;
  empty?: string;
}) {
  if (ids.length === 0) return <>{empty}</>;
  return (
    <>
      {ids.map((id, index) => (
        <Fragment key={index}>
          {index > 0 && ", "}
          <Ref id={id} nameById={nameById} />
        </Fragment>
      ))}
    </>
  );
}

/// A reference to a content entry by its id, resolved through the [`EntryEvents`] context to the
/// `MemoryContentAppended` that created it. With a hit inside a stream frame, it renders a clay link
/// labelled with a short clamped snippet of the entry's text — human-meaningful over the raw ULID —
/// navigating to the State view with the owning memory selected and the entry highlighted; the ULID
/// stays in the tooltip for precision. The link deliberately pins no seq: the log is append-only, so
/// current state always still holds the entry — live, or in the superseded/retracted sections — and
/// freezing the cursor at the referencing event would only lock the view into the past. With no hit
/// (the append predates the loaded window), no stream frame, or an unresolvable memory name, it
/// degrades to the bare id, exactly as an unresolved reference reads today.
export function EntryRef({ id, nameById }: { id: string; nameById: Map<string, string> }) {
  const index = useContext(EntryEvents);
  const stream = useOptionalStream();
  const target = index.get(id);
  const memoryName = target ? nameById.get(target.memoryId) : undefined;
  if (!target || !stream || !memoryName) return <Mono>{id}</Mono>;
  return (
    <Link
      to={stream.link.state(memoryName, { entry: id })}
      title={id}
      className="text-clay underline-offset-2 transition-colors hover:text-ink hover:underline"
    >
      {clampSnippet(target.snippet)}
    </Link>
  );
}

/// A reference to a single log event by its sequence number, linking into the Events view with that
/// event pinned via `?event=<seq>` — the anchor pages to the row and expands it. Renders inside a
/// stream frame only; frameless, or for the `Seq(0)` "before any event" sentinel (which names no real
/// event), it degrades to the bare number, exactly as an unresolved reference reads. Like [`EntryRef`]
/// it pins no timeline cursor: the target seq is reached from the head so any referenced event stays
/// in reach rather than being hidden behind an earlier pin.
export function EventRef({ seq }: { seq: number }) {
  const stream = useOptionalStream();
  if (!stream || seq <= 0) return <Mono>{seq}</Mono>;
  return (
    <Link
      to={stream.link.events({ event: seq })}
      title={`Open event ${seq} in the log`}
      className="text-clay underline-offset-2 transition-colors hover:text-ink hover:underline"
    >
      {seq}
    </Link>
  );
}

export function Mono({ children }: { children: ReactNode }) {
  return <span className="break-all text-ink-soft">{children}</span>;
}

/// A long text body (a brief, a prompt template) — the content itself, not a JSON dump.
export function Prose({ children }: { children: string }) {
  return <Excerpt>{children}</Excerpt>;
}

/// Clamp an entry's text to a quoted one-line snippet (~40 characters), collapsing whitespace so a
/// multi-line entry reads as a single label.
function clampSnippet(text: string): string {
  const collapsed = text.trim().replace(/\s+/g, " ");
  const clamped = collapsed.length > 40 ? `${collapsed.slice(0, 40).trimEnd()}…` : collapsed;
  return `"${clamped}"`;
}
