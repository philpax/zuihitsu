import type { EventPayload } from "@zuihitsu/wire/types/EventPayload.ts";
import type { EventSource } from "@zuihitsu/wire/types/EventSource.ts";
import { formatDateTime } from "../lib/format/format.ts";
import { sourceLabel } from "../lib/model/events.ts";
import { type RenderContext, renderPayload } from "./renderPayload.tsx";

/// The expanded view of a single event, rendered for its kind. Every payload gets a bespoke,
/// label-and-value layout — a Lua block highlighted, a model call's reasoning and token usage, an
/// entry's teller and visibility — and the handful with no dedicated case fall to a readable field
/// tree rather than a raw JSON dump. This is where the log stops being a stream of one-liners and
/// becomes inspectable.
///
/// When `seq` (this event's seq) is given and the detail renders inside a stream frame, every memory
/// the event references becomes a link into the State view folded to that seq with the memory open —
/// so an event's mention of a memory carries you to it at the point in the timeline it happened.
/// Without a seq, or outside a stream frame, the references render as plain names (the viewer is then
/// usable frameless). `recordedAt`, when given, prints the wall-clock time the event was committed
/// beneath the body, alongside `source` — the authority that wrote it (spec §Trust model) — as faint
/// provenance.
export function EventDetail({
  payload,
  nameById,
  conversationNameById,
  seq,
  recordedAt,
  source,
}: {
  payload: EventPayload;
  nameById: Map<string, string>;
  conversationNameById: Map<string, string>;
  seq?: number;
  recordedAt?: number;
  source?: EventSource;
}) {
  const ctx: RenderContext = { payload, nameById, conversationNameById, seq };
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
