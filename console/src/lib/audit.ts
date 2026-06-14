import type { Event } from "../types/Event.ts";

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
