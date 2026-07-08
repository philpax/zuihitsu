import type { EventPayload } from "../types/EventPayload.ts";
import { formatDateTime } from "../lib/format/format.ts";
import { type RenderContext, renderPayload } from "./renderPayload.tsx";

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
/// `recordedAt`, when given, prints the wall-clock time the event was committed beneath the body.
export function EventDetail({
  payload,
  nameById,
  base,
  seq,
  recordedAt,
}: {
  payload: EventPayload;
  nameById: Map<string, string>;
  base?: string;
  seq?: number;
  recordedAt?: number;
}) {
  const ctx: RenderContext = { payload, nameById, base, seq };
  return (
    <div className="flex flex-col gap-2">
      {renderPayload(ctx)}
      {recordedAt != null && (
        <p className="font-mono text-2xs text-ink-faint">at {formatDateTime(recordedAt)}</p>
      )}
    </div>
  );
}
