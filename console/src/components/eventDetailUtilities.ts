import type { TemporalRef } from "@zuihitsu/wire/types/TemporalRef.ts";
import type { ProducedBy } from "@zuihitsu/wire/types/ProducedBy.ts";
import { formatDateTime } from "../lib/format/format.ts";
import { rruleLabel } from "../lib/model/audit.ts";

/// A human label for an entry's resolved time, across the temporal-reference variants.
export function temporalRefLabel(ref: TemporalRef): string {
  if ("instant" in ref) return formatDateTime(ref.instant);
  if ("day" in ref) return ref.day;
  if ("range" in ref)
    return `${formatDateTime(ref.range.start)} – ${formatDateTime(ref.range.end)}`;
  if ("approx" in ref) return `~${formatDateTime(ref.approx.center)} (±${ref.approx.fuzz_days}d)`;
  if ("recurring" in ref) return rruleLabel(ref.recurring);
  return `${ref.before_after.dir} ${ref.before_after.anchor}`;
}

/// Who produced a derived event — the model and prompt template behind it.
export function producedByLabel(by: ProducedBy): string {
  return `${by.model_id} · ${by.template_name} v${by.template_version}`;
}
