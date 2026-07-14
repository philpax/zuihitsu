import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { Completion } from "@zuihitsu/wire/types/Completion.ts";
import type { Message } from "@zuihitsu/wire/types/Message.ts";
import type { ModelPhase } from "@zuihitsu/wire/types/ModelPhase.ts";
import type { PromptSectionSpan } from "@zuihitsu/wire/types/PromptSectionSpan.ts";
import type { ToolSpec } from "@zuihitsu/wire/types/ToolSpec.ts";
import type { Usage } from "@zuihitsu/wire/types/Usage.ts";

/// How a call's request was recorded: a `Base` (the full prompt), a `Continuation` (an append-only
/// delta), or `missing` (captured under a level that dropped the request, or an orphaned
/// continuation whose base fell outside the log).
export type RecordKind = "base" | "continuation" | "missing";

/// One model call, with the **full prompt the model actually saw** reconstructed from the
/// delta-encoded request (spec §Observability → the model-interaction record): the `Base` of its
/// `(turn_id, phase)` group plus every `Continuation` delta up to it, in `seq` order.
export interface ModelInteraction {
  seq: number;
  conversation: string;
  turnId: string;
  phase: ModelPhase;
  system: string;
  /// The recorded section spans of `system` (byte offsets into its UTF-8 encoding). Empty for
  /// records written before sections were captured — consumers fall back to a heuristic parse.
  systemSections: PromptSectionSpan[];
  messages: Message[];
  /// The index into `messages` where this call's own appended slice begins: `0` for a base call
  /// (everything is new), the prior length for a continuation.
  appendedFrom: number;
  record: RecordKind;
  tools: ToolSpec[];
  completion: Completion;
  reasoning: string | null;
  finishReason: string | null;
  usage: Usage;
  durationMs: number;
}

/// The model calls up to the cursor, each with its full reconstructed prompt, in `seq` order. A call
/// recorded under the `Off` capture level carries no request, so its prompt reconstructs as empty.
export function buildInteractions(events: Event[], cursor: number): ModelInteraction[] {
  const groups = new Map<
    string,
    { system: string; sections: PromptSectionSpan[]; tools: ToolSpec[]; messages: Message[] }
  >();
  const out: ModelInteraction[] = [];
  for (const event of [...events].sort((a, b) => a.seq - b.seq)) {
    if (event.seq > cursor) continue;
    const payload = event.payload;
    if (payload.type !== "ModelCalled") continue;

    const key = `${payload.turn_id} ${payload.phase}`;
    let group = groups.get(key);
    const request = payload.request;
    let record: RecordKind = "missing";
    let appendedFrom = 0;
    if (request && "Base" in request) {
      record = "base";
      group = {
        system: request.Base.system,
        // Pre-capture records deserialize with the key absent; default to empty spans.
        sections: request.Base.system_sections ?? [],
        tools: request.Base.tools,
        messages: [...request.Base.messages],
      };
      groups.set(key, group);
    } else if (request && "Continuation" in request && group) {
      record = "continuation";
      appendedFrom = group.messages.length;
      group.messages = [...group.messages, ...request.Continuation.appended_messages];
    }

    // A request-less call reconstructs nothing: showing the group's earlier state would present a
    // prompt this call was never proven to have sent (its digest is all that was recorded).
    const snapshot = record === "missing" ? undefined : group;
    out.push({
      seq: event.seq,
      conversation: payload.conversation,
      turnId: payload.turn_id,
      phase: payload.phase,
      system: snapshot?.system ?? "",
      systemSections: snapshot?.sections ?? [],
      messages: snapshot ? [...snapshot.messages] : [],
      appendedFrom,
      record,
      tools: snapshot?.tools ?? [],
      completion: payload.completion,
      reasoning: payload.reasoning,
      finishReason: payload.finish_reason,
      usage: payload.usage,
      durationMs: Number(payload.duration_ms),
    });
  }
  return out;
}

/// The two denominators a prompt's size reads against, and their honesty: the compaction token
/// budget the agent re-segments at, and the model's stated context window. `null` means the log
/// does not carry the value — "unknown", never a fabricated default.
export interface ContextDenominators {
  budget: number | null;
  contextLength: number | null;
}

/// The denominators in effect at the cursor — the latest `ConfigSet` at or before it wins. A log
/// with no `ConfigSet` (recorded before genesis wrote one) yields nulls, rendered as "budget
/// unknown" rather than a stale hardcoded number.
export function contextDenominatorsAt(events: Event[], cursor: number): ContextDenominators {
  let denominators: ContextDenominators = { budget: null, contextLength: null };
  let at = -1;
  for (const event of events) {
    if (event.seq > cursor || event.seq <= at) continue;
    if (event.payload.type === "ConfigSet") {
      const compaction = event.payload.settings.compaction;
      denominators = {
        budget: compaction.token_budget,
        // Pre-capture snapshots deserialize with the key absent.
        contextLength: compaction.context_length ?? null,
      };
      at = event.seq;
    }
  }
  return denominators;
}
