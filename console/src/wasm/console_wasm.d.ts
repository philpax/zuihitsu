/* tslint:disable */
/* eslint-disable */

/**
 * A materializing read replica: an event log plus the graph state it folds into. The log is
 * retained so the graph can be re-folded to any earlier `Seq` for time-travel.
 */
export class Replica {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Every durable conversation up to the current fold horizon, each with its sessions — the
     * structure behind the Conversation view, with the `context/*` room name and the per-session
     * participant handles resolved from ids the raw log only carries opaquely.
     */
    conversations(): any;
    /**
     * Re-fold the graph to reflect only events with `seq <= up_to` — the time-travel scrub. Folding
     * from zero each time is fine at a run's scale; caching checkpoints is a later optimization.
     */
    foldTo(up_to: number): void;
    /**
     * Every memory at the current fold horizon, as `MemoryView[]`, ordered by name. Pass a `prefix`
     * (e.g. `"person/"`) to scope by namespace, or an empty string for all.
     */
    memories(prefix: string): any;
    /**
     * The full State-view detail for one memory by name, or `null` if there is no such memory at
     * the current fold horizon. Bundles its live entries, its history, its links, and its `same_as`
     * class so the frontend opens a memory in a single call.
     */
    memory(name: string): any;
    /**
     * Build a replica from a JSON-encoded `Event[]` — a run's embedded log, or a live catch-up
     * batch. The events are folded through the real materializer up to their head.
     */
    constructor(events_json: Uint8Array);
    /**
     * The registered link relations at the current fold horizon, as `RelationView[]`.
     */
    relations(): any;
    /**
     * The tag vocabulary at the current fold horizon, as `TagVocabularyEntry[]` (name, purpose, and
     * live-use count).
     */
    tags(): any;
    /**
     * The number of events in the log (independent of the fold horizon).
     */
    readonly eventCount: number;
    /**
     * The `Seq` currently folded into the graph (what the queries below reflect).
     */
    readonly foldedSeq: number;
    /**
     * The highest `Seq` in the log — the upper bound of the time-travel range.
     */
    readonly headSeq: number;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_replica_free: (a: number, b: number) => void;
    readonly replica_conversations: (a: number) => [number, number, number];
    readonly replica_eventCount: (a: number) => number;
    readonly replica_foldTo: (a: number, b: number) => [number, number];
    readonly replica_foldedSeq: (a: number) => number;
    readonly replica_headSeq: (a: number) => number;
    readonly replica_memories: (a: number, b: number, c: number) => [number, number, number];
    readonly replica_memory: (a: number, b: number, c: number) => [number, number, number];
    readonly replica_new: (a: number, b: number) => [number, number, number];
    readonly replica_relations: (a: number) => [number, number, number];
    readonly replica_tags: (a: number) => [number, number, number];
    readonly rust_sqlite_wasm_abort: () => void;
    readonly rust_sqlite_wasm_assert_fail: (a: number, b: number, c: number, d: number) => void;
    readonly rust_sqlite_wasm_calloc: (a: number, b: number) => number;
    readonly rust_sqlite_wasm_free: (a: number) => void;
    readonly rust_sqlite_wasm_getentropy: (a: number, b: number) => number;
    readonly rust_sqlite_wasm_localtime: (a: number) => number;
    readonly rust_sqlite_wasm_malloc: (a: number) => number;
    readonly rust_sqlite_wasm_realloc: (a: number, b: number) => number;
    readonly sqlite3_os_end: () => number;
    readonly sqlite3_os_init: () => number;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
