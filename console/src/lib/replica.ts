import initWasm, { Replica as WasmReplica } from "../wasm/console_wasm.js";
import wasmUrl from "../wasm/console_wasm_bg.wasm?url";

import type { Event } from "../types/Event.ts";
import type { BriefTrace } from "./brief.ts";
import type {
  AgendaItem,
  ConversationDetail,
  MemoryDetail,
  MemoryView,
  RelationView,
  TagVocabularyEntry,
} from "./graph.ts";

/// The wasm module initializes once per page; every replica shares it.
let wasmReady: Promise<unknown> | null = null;
function ensureWasm(): Promise<unknown> {
  wasmReady ??= initWasm({ module_or_path: wasmUrl });
  return wasmReady;
}

/// A typed handle over the console-wasm `Replica`: an event log folded through the agent's own
/// materializer, queried for the graph-backed views. The wasm methods return `JsValue`; this is the
/// one place those crossings are cast to the `graph.ts` shapes, so the views stay fully typed.
export class Replica {
  readonly #inner: WasmReplica;

  private constructor(inner: WasmReplica) {
    this.#inner = inner;
  }

  /// Fold a run's event log into a replica. Async only because the wasm module loads on first use.
  static async fromEvents(events: Event[]): Promise<Replica> {
    await ensureWasm();
    const bytes = new TextEncoder().encode(JSON.stringify(events));
    return new Replica(new WasmReplica(bytes));
  }

  get eventCount(): number {
    return this.#inner.eventCount;
  }

  /// Append a live catch-up batch to the log without re-folding — the tail a `/control/events` poll
  /// returned. The fold horizon is left where it is; the caller advances it with `foldTo` to follow
  /// the head, or holds it to stay time-travel pinned.
  append(events: Event[]): void {
    const bytes = new TextEncoder().encode(JSON.stringify(events));
    this.#inner.append(bytes);
  }

  /// A fresh handle over the same underlying wasm replica — a new object identity, no rebuild. Live
  /// mode mutates one replica in place as the log tails; handing the views a fresh handle per batch
  /// lets React's memoization re-derive (so a new participant's name resolves) without remounting
  /// the views and losing their local state, such as the open room.
  snapshot(): Replica {
    return new Replica(this.#inner);
  }

  /// The highest seq in the log — the upper bound of the time-travel range.
  get headSeq(): number {
    return this.#inner.headSeq;
  }

  /// The seq currently folded into the graph (what the queries below reflect).
  get foldedSeq(): number {
    return this.#inner.foldedSeq;
  }

  /// Re-fold the graph to reflect only events with `seq <= upTo` (time-travel).
  foldTo(upTo: number): void {
    this.#inner.foldTo(upTo);
  }

  memories(prefix = ""): MemoryView[] {
    return this.#inner.memories(prefix) as MemoryView[];
  }

  memory(name: string): MemoryDetail | null {
    return this.#inner.memory(name) as MemoryDetail | null;
  }

  tags(): TagVocabularyEntry[] {
    return this.#inner.tags() as TagVocabularyEntry[];
  }

  relations(): RelationView[] {
    return this.#inner.relations() as RelationView[];
  }

  conversations(): ConversationDetail[] {
    return this.#inner.conversations() as ConversationDetail[];
  }

  /// The agent's upcoming agenda within `horizonDays` of `nowMs` — one-off and recurring occurrences
  /// merged and ordered soonest first, using the agent's own next-occurrence logic.
  agenda(nowMs: number, horizonDays: number): AgendaItem[] {
    return this.#inner.agenda(nowMs, horizonDays) as AgendaItem[];
  }

  /// Re-derive a session's brief and the trace of how it was composed, against the graph at the
  /// current fold. `present` and `context` are memory ids; `nowMs` is the session start time.
  brief(present: string[], context: string | null, nowMs: number): BriefTrace {
    return this.#inner.brief(present, context, nowMs) as BriefTrace;
  }
}
