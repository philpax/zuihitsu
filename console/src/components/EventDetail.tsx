import type { EventPayload } from "../types/EventPayload.ts";
import type { EventSource } from "../types/EventSource.ts";
import { formatDateTime } from "../lib/format/format.ts";
import { sourceLabel } from "../lib/model/events.ts";
import { type RenderContext, renderPayload } from "./renderPayload.tsx";

/// Build a `conversationId → contextMemoryName` map from conversations, so `ConversationRef`
/// links can resolve the room name. The context memory name is what `nameById` holds for it
/// (e.g. `context/discord:book-club`), so the caller can then resolve it to a display name.
export function conversationNameById(
  conversations: { id: string; context_name: string | null }[],
): Map<string, string> {
  const map = new Map<string, string>();
  for (const conv of conversations) {
    if (conv.context_name) {
      map.set(conv.id, conv.context_name);
    }
  }
  return map;
}

/// The expanded view of a single event, rendered for its kind. Every payload gets a bespoke,
/// label-and-value layout — a Lua block highlighted, a model call's reasoning and token usage, an
/// entry's teller and visibility — and the handful with no dedicated case fall to a readable field
/// tree rather than a raw JSON dump. This is where the log stops being a stream of one-liners and
/// becomes inspectable.
///
/// When `base` (the stream's path) and `seq` (this event's seq) are given, every memory the event
/// references becomes a link into the State view folded to that seq with the memory open — so an
/// event's mention of a memory carries you to it at the point in the timeline it happened. Without
/// them the references render as plain names (the viewer is then usable outside a routed stream).
/// `recordedAt`, when given, prints the wall-clock time the event was committed beneath the body,
/// alongside `source` — the authority that wrote it (spec §Trust model) — as faint provenance.
export function EventDetail({
  payload,
  nameById,
  conversationNameById,
  base,
  seq,
  recordedAt,
  source,
}: {
  payload: EventPayload;
  nameById: Map<string, string>;
  conversationNameById: Map<string, string>;
  base?: string;
  seq?: number;
  recordedAt?: number;
  source?: EventSource;
}) {
  const ctx: RenderContext = { payload, nameById, conversationNameById, base, seq };
  return (
    <div className="flex flex-col gap-2">
      {renderPayload(ctx)}
      {(recordedAt != null || source) && (
        <p className="font-mono text-2xs text-ink-faint">
          {recordedAt != null && <>at {formatDateTime(recordedAt)}</>}
          {recordedAt != null && source && " · "}
          {source && <>by {sourceLabel(source)}</>}
        </p>
      )}
    </div>
  );
}
