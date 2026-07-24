import type { Message } from "@zuihitsu/wire/types/Message.ts";
import type { PromptSectionKind } from "@zuihitsu/wire/types/PromptSectionKind.ts";
import type { CacheVerdict } from "./cachePath.ts";
import type { ModelInteraction } from "./interactions.ts";
import { resolveSections } from "./promptSections.ts";

/// How a row's token count was obtained, most trusted first: measured from provider deltas,
/// apportioned by char share inside a measured lump, or estimated when no measurement exists.
export type TokenProvenance = "measured" | "apportioned" | "estimated";

export interface AttributedRow {
  /// A stable identity for the row: `section:<kind>`, `tools`, `prefix`, or `message:<index>`.
  key: string;
  label: string;
  tokens: number;
  provenance: TokenProvenance;
  /// Set on per-message rows: the index into the call's reconstructed `messages`.
  messageIndex?: number;
  /// Set on system-section rows.
  sectionKind?: PromptSectionKind;
}

export interface CallAttribution {
  rows: AttributedRow[];
  /// Equals `usage.prompt_tokens` whenever the provider reported it and the numbers were usable —
  /// the reconciliation guarantee. On the estimated fallback (usage absent, or a provider-reported
  /// total *smaller* than the prior call's), the total is the row sum instead, badged estimated.
  total: number;
  totalProvenance: TokenProvenance;
}

/// The `chars / 4` heuristic, the TS port of the compaction trigger's fallback estimator
/// (`estimate_tokens` in the agent): code-point counting, deliberately coarse.
export function estimateTokens(text: string): number {
  return Math.ceil([...text].length / 4);
}

/// Attribute each call's prompt tokens to rows, one `CallAttribution` per call, using the
/// measurement ladder: exact deltas within a warm chain, char-share apportionment inside a measured
/// lump, and the `chars / 4` estimate when no measurement exists. The displayed total always equals
/// the provider's `prompt_tokens` when reported.
export function attributeTokens(
  calls: ModelInteraction[],
  verdicts: CacheVerdict[],
): CallAttribution[] {
  return calls.map((call, index) => {
    const prior = index > 0 ? calls[index - 1] : null;
    const verdict = verdicts[index];
    const total = call.usage.prompt_tokens;

    if (total === null || total === undefined) {
      return estimated(call);
    }
    const priorTotal = prior?.usage.prompt_tokens;
    if (
      verdict?.path === "warm" &&
      prior !== null &&
      priorTotal !== null &&
      priorTotal !== undefined
    ) {
      // A negative delta is a provider inconsistency; estimates are honest, negative rows are not
      // — and apportioning the prefix to more than the total would break the reconciliation
      // guarantee. A zero delta stays measured (an identical retry re-reads the same prompt).
      const delta = total - priorTotal;
      if (delta < 0) {
        return estimated(call);
      }
      return measuredDelta(call, prior, total, delta);
    }
    return apportionedLump(call, total);
  });
}

/// A warm call (a `Continuation`, or a `Base` re-sending a warm prefix): the full part list, in send
/// order, with two measured lumps split within themselves — the shared prefix (the prior call's
/// total) apportioned across the system sections, the tools, and the earlier messages by char share,
/// and the appended slice (the delta) split over its messages with the prior completion pinning the
/// assistant message.
function measuredDelta(
  call: ModelInteraction,
  prior: ModelInteraction,
  total: number,
  delta: number,
): CallAttribution {
  // The index at and beyond which a message belongs to the appended slice. A warm verdict guarantees
  // (via cachePath's `extendsMessages`) that the prior call's messages are a prefix of this call's,
  // so the prior call's message count is exactly the prefix boundary. A `Continuation` records that
  // boundary in `appendedFrom`; a warm `Base` re-sends the whole prompt in a single step and carries
  // `appendedFrom = 0`, which would spray the delta across every message and collapse the ~28k prefix
  // onto the system rows — so its boundary is the prior call's message count instead. If the messages
  // somehow shrank (never on a warm verdict, but be honest), fall back to a plain apportioned lump.
  if (call.record === "base" && call.messages.length < prior.messages.length) {
    return apportionedLump(call, total);
  }
  const appendedFrom = call.record === "base" ? prior.messages.length : call.appendedFrom;
  const parts = lumpParts(call);
  const appendedPart = (part: { row: AttributedRow }) =>
    part.row.messageIndex !== undefined && part.row.messageIndex >= appendedFrom;
  const prefixParts = parts.filter((part) => !appendedPart(part));
  const appendedParts = parts.filter(appendedPart);

  const prefixShares = apportion(
    prefixParts.map((part) => part.chars),
    total - delta,
  );
  const rows: AttributedRow[] = prefixParts.map((part, i) => ({
    ...part.row,
    tokens: prefixShares[i] ?? 0,
  }));

  const completion = prior.usage.completion_tokens;
  const assistantAt = appendedParts.findIndex(
    (part) =>
      part.row.messageIndex !== undefined &&
      call.messages[part.row.messageIndex]?.role === "assistant",
  );
  let remainder = delta;
  let pinned: number | null = null;
  if (assistantAt !== -1 && completion !== null && completion !== undefined) {
    pinned = Math.min(completion, delta);
    remainder = delta - pinned;
  }
  const others = appendedParts.filter((_, i) => i !== assistantAt || pinned === null);
  const shares = apportion(
    others.map((part) => part.chars),
    remainder,
  );
  let shareIndex = 0;
  appendedParts.forEach((part, i) => {
    if (i === assistantAt && pinned !== null) {
      rows.push({ ...part.row, tokens: pinned, provenance: "measured" });
    } else {
      rows.push({ ...part.row, tokens: shares[shareIndex] ?? 0 });
      shareIndex += 1;
    }
  });
  return { rows, total, totalProvenance: "measured" };
}

/// A base call's single measured lump, split over the system sections, the tools, and the initial
/// messages by char share, scaled to sum exactly to the measured total.
function apportionedLump(call: ModelInteraction, total: number): CallAttribution {
  const parts = lumpParts(call);
  const shares = apportion(
    parts.map((part) => part.chars),
    total,
  );
  const rows = parts.map((part, i) => ({ ...part.row, tokens: shares[i] ?? 0 }));
  return { rows, total, totalProvenance: "measured" };
}

/// No measurement at all: every row is the raw `chars / 4` estimate, and so is the total.
function estimated(call: ModelInteraction): CallAttribution {
  const rows = lumpParts(call).map((part) => ({
    ...part.row,
    // The char counts are already counted, so the estimator's division applies directly.
    tokens: Math.ceil(part.chars / 4),
    provenance: "estimated" as const,
  }));
  return {
    rows,
    total: rows.reduce((sum, row) => sum + row.tokens, 0),
    totalProvenance: "estimated",
  };
}

/// The apportionable parts of a whole prompt: one row per resolved system section, a tools row when
/// tools are present, and one row per message.
function lumpParts(call: ModelInteraction): Array<{ chars: number; row: AttributedRow }> {
  const parts: Array<{ chars: number; row: AttributedRow }> = [];
  for (const section of resolveSections(call.system, call.systemSections)) {
    parts.push({
      chars: section.end - section.start,
      row: {
        key: `section:${section.kind}`,
        label: sectionLabel(section.kind),
        tokens: 0,
        provenance: "apportioned",
        sectionKind: section.kind,
      },
    });
  }
  if (call.tools.length > 0) {
    parts.push({
      chars: JSON.stringify(call.tools).length,
      row: { key: "tools", label: "tools", tokens: 0, provenance: "apportioned" },
    });
  }
  call.messages.forEach((message, index) => {
    parts.push({
      chars: messageChars(message),
      row: {
        key: `message:${index}`,
        label: messageLabel(message),
        tokens: 0,
        provenance: "apportioned",
        messageIndex: index,
      },
    });
  });
  return parts;
}

/// Integer allocation of `total` across `weights`, largest-remainder adjusted on the last non-zero
/// weight so the rows always sum exactly to the total.
function apportion(weights: number[], total: number): number[] {
  const sum = weights.reduce((a, b) => a + b, 0);
  if (sum === 0 || total <= 0) return weights.map(() => 0);
  const shares = weights.map((weight) => Math.round((total * weight) / sum));
  const drift = total - shares.reduce((a, b) => a + b, 0);
  if (drift !== 0) {
    // Land the rounding drift on the largest share, which absorbs it least visibly. Rounding
    // bounds |drift| by half the row count, so the largest share only clamps at zero when the
    // total is a handful of tokens spread over many rows — the exact-sum property can be off by
    // at most that clamp in the degenerate case.
    const largest = shares.indexOf(Math.max(...shares));
    shares[largest] = Math.max(0, shares[largest] + drift);
  }
  return shares;
}

/// Message objects are shared along a group's reconstruction prefix, so the serialized size is
/// computed once per distinct message rather than once per call that carries it.
const messageCharsCache = new WeakMap<Message, number>();

function messageChars(message: Message): number {
  const cached = messageCharsCache.get(message);
  if (cached !== undefined) return cached;
  const chars = JSON.stringify(message).length;
  messageCharsCache.set(message, chars);
  return chars;
}

function messageLabel(message: Message): string {
  const head = message.content.split("\n")[0] ?? "";
  const excerpt = head.length > 48 ? `${head.slice(0, 48)}…` : head;
  return excerpt.length > 0 ? `${message.role}: ${excerpt}` : message.role;
}

/// A section kind's display name, exhaustive over the typed enum — shared with the view layer so a
/// new kind fails the build everywhere it needs a label.
export function sectionLabel(kind: PromptSectionKind): string {
  switch (kind) {
    case "Scaffold":
      return "scaffold";
    case "Identity":
      return "identity";
    case "ApiReference":
      return "API reference";
    case "Vocabulary":
      return "vocabulary";
    case "Brief":
      return "brief";
    case "CurrentTime":
      return "current time";
  }
}
