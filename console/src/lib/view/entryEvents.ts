import { createContext } from "react";

import type { Event } from "@zuihitsu/wire/types/Event.ts";

// The non-component half of entry references: the lookup context the workspace fills from the log,
// so a raw entry id shown in an event detail (a consolidation's sources, a retraction's entry) can
// link back to the `MemoryContentAppended` that created it. The shared detail renderers have no
// access to the event list by design, so they read this context, degrading to the bare id when it
// is empty — a detail rendered outside a stream frame, or an append that predates the loaded window.

/// Where the append that created an entry sits in the log: the `MemoryContentAppended`'s seq (the
/// Events view's deep-link anchor), the memory it landed in, and the entry's text (clamped to a
/// human-meaningful snippet at display).
export interface EntryEventTarget {
  seq: number;
  memoryId: string;
  snippet: string;
}

export type EntryEventIndex = ReadonlyMap<string, EntryEventTarget>;

/// Every entry id by the `MemoryContentAppended` that created it, filled by the workspace from the
/// stream's events — so a reference resolves against exactly the log the console holds, and an id
/// whose append is outside the loaded window reads as unknown. The default resolves nothing, for a
/// detail rendered without a provider.
export const EntryEvents = createContext<EntryEventIndex>(new Map());

/// Index the appends in an event stream by the entry id each created. An entry id is unique to its
/// append, so a later occurrence never overwrites an earlier one.
export function buildEntryEvents(events: readonly Event[]): EntryEventIndex {
  const index = new Map<string, EntryEventTarget>();
  for (const event of events) {
    if (event.payload.type === "MemoryContentAppended") {
      index.set(event.payload.entry_id, {
        seq: event.seq,
        memoryId: event.payload.id,
        snippet: event.payload.text,
      });
    }
  }
  return index;
}
