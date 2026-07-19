import initWasm, {
  Replica as WasmReplica,
  memRefConstruct,
  refNormalize,
  refScan,
  turnRefConstruct,
} from "@zuihitsu/wire/wasm/console_wasm.js";
import wasmUrl from "@zuihitsu/wire/wasm/console_wasm_bg.wasm?url";

import type {
  AgendaItem,
  ConversationDetail,
  DigestCheck,
  MemRefResolution,
  MemoryDetail,
  MergeProposalView,
  RefSegment,
} from "@zuihitsu/wire/wasm/console_wasm.js";

import type { BriefTrace } from "@zuihitsu/wire/types/BriefTrace.ts";
import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { MemoryView } from "@zuihitsu/wire/types/MemoryView.ts";
import type { NamespacedMemoryName } from "@zuihitsu/wire/types/NamespacedMemoryName.ts";
import type { RecurringEntry } from "@zuihitsu/wire/types/RecurringEntry.ts";
import type { RelationView } from "@zuihitsu/wire/types/RelationView.ts";
import type { TagVocabularyEntry } from "@zuihitsu/wire/types/TagVocabularyEntry.ts";

/// The wasm module initializes once per page; every replica shares it.
let wasmReady: Promise<unknown> | null = null;
function ensureWasm(): Promise<unknown> {
  wasmReady ??= initWasm({ module_or_path: wasmUrl });
  return wasmReady;
}

// The reference parser, crossing from `zuihitsu_core::message_refs` — the same definition the agent's
// `convo.turn` resolver reads, so what the console highlights, normalizes, and mints cannot drift from
// what the agent resolves. These are pure functions of their text, exported free rather than as
// `Replica` methods; they still require the wasm module, which is guaranteed initialized wherever they
// are called (every view renders under a `Replica` built by `fromEvents`, which awaits `ensureWasm`
// first). Core recognizes token syntax only; a deep-link URL is recognized by the frontend's own route
// matching (`lib/nav/refRoutes.ts`) and rewritten to a token before these see it.

/// Split text into prose spans, turn references, and memory references in one pass — the transcript's
/// pretty projection, so both reference-token vocabularies render as chips from a single wasm call.
/// Token syntax is parsed only in Rust; the caller dispatches on `kind`.
export function scanRefs(text: string): RefSegment[] {
  return refScan(text);
}

/// Rebuild text with every reference token — turn or memory — collapsed to its canonical form. The
/// composer runs this after the nav layer has rewritten any deep-link URL to a token, so a message that
/// leaves the console carries only canonical token syntax.
export function normalizeRefTokens(text: string): string {
  return refNormalize(text);
}

/// The canonical turn-reference token for a turn id — minted by the same constructor the agent's `ref`
/// field uses. Throws if `id` is not a valid id.
export function constructTurnRef(id: string): string {
  return turnRefConstruct(id);
}

/// The canonical memory-reference token for a memory id. Throws if `id` is not a valid id.
export function constructMemRef(id: string): string {
  return memRefConstruct(id);
}

/// A typed handle over the console-wasm `Replica`: an event log folded through the agent's own
/// materializer, queried for the graph-backed views. Methods returning a console-wasm DTO are typed
/// at the boundary, so they pass the value straight through; methods returning a core view type
/// cross as an untyped `JsValue` and are cast here — the one place those crossings are given their
/// `wire/types` shape, so the views stay fully typed.
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
    return this.#inner.memory(name) ?? null;
  }

  /// Resolve a memory reference to the memory the transcript chip should display for it,
  /// collapsed to its `same_as` class primary — the primary's id and handle, or `null` when the id
  /// names no memory at the current fold horizon (so the chip degrades to a muted token).
  resolveMemRef(id: string): MemRefResolution | null {
    return this.#inner.resolveMemRef(id) ?? null;
  }

  /// The id of the live memory a handle currently names, or `null` when none does — the composer's
  /// first step in turning a pasted state-view deep link into a token (route matching decodes the
  /// handle; this answers which memory it is). A plain graph lookup by name.
  memoryIdByName(name: string): string | null {
    return (this.#inner.memoryIdByName(name) ?? null) as string | null;
  }

  /// The id of the live memory that used to go by `name` under a since-changed handle, or `null` — the
  /// alias fallback behind a stale pasted state-view link normalizing after a rename. Consulted only
  /// after `memoryIdByName` misses.
  memoryIdForFormerName(name: string): string | null {
    return (this.#inner.memoryIdForFormerName(name) ?? null) as string | null;
  }

  tags(): TagVocabularyEntry[] {
    return this.#inner.tags() as TagVocabularyEntry[];
  }

  relations(): RelationView[] {
    return this.#inner.relations() as RelationView[];
  }

  /// Every live recurring entry, with the memory it belongs to — the graph's authority on which
  /// memories carry a recurring occurrence, so the state view badges them without re-folding the log.
  recurringEntries(): RecurringEntry[] {
    return this.#inner.recurringEntries() as RecurringEntry[];
  }

  /// Every cross-platform merge proposal in the folded log, in first-proposal order, each tagged with
  /// its resolution state (pending or merged) at the current fold cursor.
  mergeProposals(): MergeProposalView[] {
    return this.#inner.mergeProposals();
  }

  conversations(): ConversationDetail[] {
    return this.#inner.conversations();
  }

  /// The agent's upcoming agenda within `horizonDays` of `nowMs` — one-off and recurring occurrences
  /// merged and ordered soonest first, using the agent's own next-occurrence logic.
  agenda(nowMs: number, horizonDays: number): AgendaItem[] {
    return this.#inner.agenda(nowMs, horizonDays);
  }

  /// Verify every model call's recorded prompt against the digest stamped at send time — the
  /// reconstruction re-hashed with the recorder's own serialization. `verified` means the displayed
  /// prompt provably matches the wire request; `mismatch` means it must not be trusted silently.
  requestDigests(): DigestCheck[] {
    return this.#inner.requestDigests();
  }

  /// Re-derive a session's brief and the trace of how it was composed, against the graph at the
  /// current fold, with the brief settings folded from the log at the same horizon. `present`,
  /// `speakers`, `context`, and `workingSet` are memory ids; `nowMs` is the session start time.
  /// `speakers` is the `SessionStarted` payload's recorded initiators (whom the brief guarantees a
  /// full block); `workingSet` is its recorded working set (both empty for pre-capture sessions).
  brief(
    present: string[],
    speakers: string[],
    context: string | null,
    nowMs: number,
    workingSet: string[],
  ): BriefTrace {
    return this.#inner.brief(present, speakers, context, nowMs, workingSet) as BriefTrace;
  }

  /// The memory name a freshly minted `person/*` participant would receive — delegates to the
  /// graph's own name-resolution logic, so the optimistic preview matches the real turn.
  participantName(platform: string, platformUserId: string): string {
    return this.#inner.participant_name(platform, platformUserId) as string;
  }

  /// The platform user ids seen on a given platform — the bare handles a user can type in the "you
  /// are" field, sourced from `participant_identities` so the `@platform` disambiguation suffix
  /// never surfaces as a separate entry.
  participantIds(platform: string): string[] {
    return this.#inner.participant_ids(platform) as string[];
  }

  /// Decompose a memory name into its namespace and subject, or `null` if the name is in no known
  /// namespace (e.g. `self`). The parse runs in Rust, so the frontend never hardcodes the prefix
  /// strings.
  parseName(name: string): NamespacedMemoryName | null {
    const result = this.#inner.parse_name(name);
    return (result ?? null) as NamespacedMemoryName | null;
  }
}
