import initWasm, { Replica as WasmReplica } from "../wasm/console_wasm.js";
import wasmUrl from "../wasm/console_wasm_bg.wasm?url";

import type { Event } from "../types/Event.ts";
import type {
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
}
