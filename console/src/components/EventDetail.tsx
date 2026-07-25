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
/// Every memory the event references becomes a link into the State view with the memory open,
/// resolved against the enclosing stream frame; outside a stream frame the references render as plain
/// names (the viewer stays usable frameless). A reference never mints a timeline cursor of its own —
/// it follows the stream's own pinned seq if the operator scrubbed, and the head otherwise.
/// `recordedAt`, when given, prints the wall-clock time the event was committed beneath the body,
/// alongside `source` — the authority that wrote it (spec §Trust model) — as faint provenance.
export function EventDetail({
  payload,
  nameById,
  conversationNameById,
  recordedAt,
  source,
}: {
  payload: EventPayload;
  nameById: Map<string, string>;
  conversationNameById: Map<string, string>;
  recordedAt?: number;
  source?: EventSource;
}) {
  const ctx: RenderContext = { payload, nameById, conversationNameById };
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
