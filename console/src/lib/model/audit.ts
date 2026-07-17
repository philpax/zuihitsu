import type { Event } from "@zuihitsu/wire/types/Event.ts";

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
