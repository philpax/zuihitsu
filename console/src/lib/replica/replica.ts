import initWasm, {
  Replica as WasmReplica,
  turnRefConstruct,
  turnRefExtract,
  turnRefNormalize,
  turnRefScan,
} from "@zuihitsu/wire/wasm/console_wasm.js";
import wasmUrl from "@zuihitsu/wire/wasm/console_wasm_bg.wasm?url";

import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { NamespacedMemoryName } from "@zuihitsu/wire/types/NamespacedMemoryName.ts";
import type { BriefTrace } from "../model/brief.ts";
import type {
  AgendaItem,
  ConversationDetail,
  MemoryDetail,
  MemoryView,
  MergeProposalView,
  RelationView,
  TagVocabularyEntry,
} from "../model/graph.ts";

/// The wasm module initializes once per page; every replica shares it.
let wasmReady: Promise<unknown> | null = null;
function ensureWasm(): Promise<unknown> {
  wasmReady ??= initWasm({ module_or_path: wasmUrl });
  return wasmReady;
}

/// One span of scanned turn-reference text: literal prose, or a reference resolved to its turn's
/// ULID. The wire shape of the wasm scanner's segments (see `RefSegment` in console-wasm).
export type TurnRefSegment = { kind: "prose"; text: string } | { kind: "ref"; id: string };

/// How one model call's recorded prompt compares against the digest stamped at send time (see
/// `DigestCheck` in console-wasm). `unverifiable` marks a structured synthesis call, whose response
/// format is not recorded; `unrecorded` marks a call whose request was not captured.
export type DigestStatus = "verified" | "mismatch" | "unverifiable" | "unrecorded";

export interface DigestCheck {
  seq: number;
  status: DigestStatus;
}

// The turn-reference parser, crossing from `zuihitsu_core::turn_ref` — the same definition the
// agent's `convo.turn` resolver reads, so what the console highlights, normalizes, and extracts
// cannot drift from what the agent resolves. These are pure functions of their text, exported free
// rather than as `Replica` methods; they still require the wasm module, which is guaranteed
// initialized wherever they are called (every view renders under a `Replica` built by `fromEvents`,
// which awaits `ensureWasm` first).

/// Split text into prose spans and turn references (`[turn:<ulid>]` tokens and `?turn=<ulid>` URLs).
export function scanTurnRefs(text: string): TurnRefSegment[] {
  return turnRefScan(text) as TurnRefSegment[];
}

/// Rebuild text with every turn reference collapsed to the canonical `[turn:<ulid>]` token — the
/// composer's send-time normalization.
export function normalizeTurnRefs(text: string): string {
  return turnRefNormalize(text);
}

/// Every turn id referenced in text, in order of appearance.
export function extractTurnRefIds(text: string): string[] {
  return turnRefExtract(text) as string[];
}

/// The canonical `[turn:<ulid>]` token for a turn id — minted by the same constructor the agent's
/// `ref` field uses. Throws if `id` is not a ULID.
export function constructTurnRef(id: string): string {
  return turnRefConstruct(id);
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

  /// Every cross-platform merge proposal in the folded log, in first-proposal order, each tagged with
  /// its resolution state (pending, merged, or rejected) at the current fold cursor.
  mergeProposals(): MergeProposalView[] {
    return this.#inner.mergeProposals() as MergeProposalView[];
  }

  conversations(): ConversationDetail[] {
    return this.#inner.conversations() as ConversationDetail[];
  }

  /// The agent's upcoming agenda within `horizonDays` of `nowMs` — one-off and recurring occurrences
  /// merged and ordered soonest first, using the agent's own next-occurrence logic.
  agenda(nowMs: number, horizonDays: number): AgendaItem[] {
    return this.#inner.agenda(nowMs, horizonDays) as AgendaItem[];
  }

  /// Verify every model call's recorded prompt against the digest stamped at send time — the
  /// reconstruction re-hashed with the recorder's own serialization. `verified` means the displayed
  /// prompt provably matches the wire request; `mismatch` means it must not be trusted silently.
  requestDigests(): DigestCheck[] {
    return this.#inner.requestDigests() as DigestCheck[];
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
