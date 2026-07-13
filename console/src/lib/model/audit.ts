import type { Event } from "../../types/Event.ts";
import type { TemporalRef } from "../../types/TemporalRef.ts";

/// One belief the agent reconciled: the competing entries it weighed and the one-line statement it
/// wrote to settle them (spec §Write path → arbitration). The audit answer to "why does it believe
/// this," derived from the log — `BeliefArbitrated` is log-only, so the materialized graph does not
/// hold it.
export interface Arbitration {
  statement: string;
  competing: number;
}

/// The arbitrations recorded for one memory, oldest first, up to the timeline cursor.
export function arbitrationsFor(events: Event[], memoryId: string, cursor: number): Arbitration[] {
  const out: Arbitration[] = [];
  for (const event of events) {
    if (event.seq > cursor) continue;
    const payload = event.payload;
    if (payload.type === "BeliefArbitrated" && payload.memory === memoryId) {
      out.push({
        statement: payload.resolution.statement,
        competing: payload.competing_entries.length,
      });
    }
  }
  return out;
}

/// One live entry that recurs: the text and the raw RRULE it carries (spec §Recurring
/// materialization). The recurring half of the agent's calendar — the operator's view of
/// `calendar.recurring()`.
export interface RecurringItem {
  text: string;
  rrule: string;
}

/// The live recurring entries per memory, up to the cursor — derived from the log by tracking each
/// entry's occurrence through its append, any temporal re-resolution, and supersession, then keeping
/// those that resolve to a `Recurring` rule and were not superseded. Deleted memories drop out
/// naturally: the caller only looks these up for memories the folded graph still holds.
export function recurringByMemory(events: Event[], cursor: number): Map<string, RecurringItem[]> {
  const entries = new Map<
    string,
    { memory: string; text: string; occurred: TemporalRef | null; superseded: boolean }
  >();
  for (const event of events) {
    if (event.seq > cursor) continue;
    const payload = event.payload;
    if (payload.type === "MemoryContentAppended") {
      entries.set(payload.entry_id, {
        memory: payload.id,
        text: payload.text,
        occurred: payload.occurred_at,
        superseded: false,
      });
    } else if (payload.type === "EntryTemporalResolved") {
      const entry = entries.get(payload.entry_id);
      if (entry) entry.occurred = payload.occurred_at;
    } else if (payload.type === "MemorySuperseded" || payload.type === "EntryRetracted") {
      // A retraction tombstones an entry exactly as a supersession does, so a retracted recurring
      // entry drops from the live recurring list too.
      const entry = entries.get(payload.entry);
      if (entry) entry.superseded = true;
    }
  }

  const byMemory = new Map<string, RecurringItem[]>();
  for (const entry of entries.values()) {
    if (entry.superseded || !entry.occurred || !("recurring" in entry.occurred)) continue;
    const items = byMemory.get(entry.memory) ?? [];
    items.push({ text: entry.text, rrule: entry.occurred.recurring });
    byMemory.set(entry.memory, items);
  }
  return byMemory;
}

/// A friendly cadence read off an RRULE's `FREQ`/`INTERVAL` (the supported subset), e.g. "every 2
/// weeks"; the raw rule rides along as a tooltip for the rest.
export function rruleLabel(rrule: string): string {
  const fields = new Map(
    rrule
      .split(";")
      .map((part) => part.split("="))
      .filter((pair) => pair.length === 2)
      .map(([key, value]) => [key.toUpperCase(), value.toUpperCase()] as const),
  );
  const unit = { DAILY: "day", WEEKLY: "week", MONTHLY: "month", YEARLY: "year" }[
    fields.get("FREQ") ?? ""
  ];
  if (!unit) return rrule;
  const interval = Number(fields.get("INTERVAL") ?? "1");
  return interval > 1 ? `every ${interval} ${unit}s` : `every ${unit}`;
}
