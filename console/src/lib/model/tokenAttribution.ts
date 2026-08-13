import type { Message } from "@zuihitsu/wire/types/Message.ts";
import type { PromptSectionKind } from "@zuihitsu/wire/types/PromptSectionKind.ts";
import type { CacheVerdict } from "./cachePath.ts";
import type { ModelInteraction } from "./interactions.ts";
import { estimateTokensFromChars } from "../replica/replica.ts";
import { resolveSections } from "./promptSections.ts";

/// What a row costs, and how firmly that is known.
///
/// `measured` is the provider's own arithmetic: a call's total, the growth between two calls, or the
/// slice a cache boundary isolates. `share` is a row inside a measured block that no measurement
/// separates — a system section, or one of several messages that entered together. Its percentage is
/// by character, which is a ratio and is shown as one; the tokens it would imply are never stated.
export type RowCost =
  | { kind: "measured"; tokens: number }
  | { kind: "share"; percent: number; block: string; blockTokens: number };

/// How a row's cost was obtained, for the badge the view renders.
export type TokenProvenance = "measured" | "share" | "estimated";

export interface AttributedRow {
  /// A stable identity for the row: `section:<kind>`, `tools`, or `message:<index>`.
  key: string;
  label: string;
  cost: RowCost;
  /// The block this row sits inside.
  block: string;
  /// Set on per-message rows: the index into the call's reconstructed `messages`.
  messageIndex?: number;
  /// Set on system-section rows.
  sectionKind?: PromptSectionKind;
}

/// One measured region of the prompt: what the provider charged for it, and the rows inside it.
export interface AttributionBlock {
  key: string;
  label: string;
  tokens: number;
  rows: string[];
}

export interface CallAttribution {
  rows: AttributedRow[];
  /// The measured regions this call's rows sit in, in prompt order.
  blocks: AttributionBlock[];
  /// Equals `usage.prompt_tokens` whenever the provider reported it. Without a report the call falls
  /// back to the character estimate, the one estimated path left.
  total: number;
  totalProvenance: TokenProvenance;
}

/// The shared fallback estimator used by the agent and the console's WASM boundary.
export { estimateTokens } from "../replica/replica.ts";

/// Attribute each call's prompt to rows, one `CallAttribution` per call.
///
/// A prompt is the opening block — the first call's whole prompt, which nothing separates into
/// system, tools, and first messages — plus one measured block per later call, holding the messages
/// that entered there. So a message is priced by the growth its own arrival caused, and only
/// messages that arrived together share a block.
///
/// Two provider numbers bound a block: the prompt totals of the calls bracketing it, and the cache
/// boundary. Where a server caches what it just generated, `cache_read - prior total` isolates the
/// assistant message from whatever entered beside it. `completion_tokens` is deliberately not used
/// for that: it counts reasoning the prompt never carries, so pinning to it overstates the reply and
/// robs its neighbour.
export function attributeTokens(
  calls: ModelInteraction[],
  verdicts: CacheVerdict[],
): CallAttribution[] {
  const entries = bracketMessages(calls, verdicts);
  return calls.map((call, index) => {
    const total = call.usage.prompt_tokens;
    if (total === null || total === undefined) return estimated(call);
    return attributeCall(call, index, calls, entries[index] ?? new Map(), total);
  });
}

/// Where one message entered the prompt, and what that entry cost when a boundary isolates it.
interface MessageEntry {
  block: number;
  tokens: number | null;
}

/// Walk the chain, assigning every message to the block it entered in. A cold call restarts the
/// walk: nothing brackets its prompt against a prefix the provider no longer shares.
function bracketMessages(
  calls: ModelInteraction[],
  verdicts: CacheVerdict[],
): Map<number, MessageEntry>[] {
  const entries: Map<number, MessageEntry>[] = [];
  let current = new Map<number, MessageEntry>();
  calls.forEach((call, index) => {
    const prior = index > 0 ? calls[index - 1] : null;
    const priorTotal = prior?.usage.prompt_tokens;
    const total = call.usage.prompt_tokens;
    const warm =
      verdicts[index]?.path === "warm" &&
      prior !== null &&
      priorTotal !== null &&
      priorTotal !== undefined &&
      total !== null &&
      total !== undefined &&
      total >= priorTotal &&
      call.messages.length >= prior.messages.length;

    if (!warm) {
      current = new Map();
      call.messages.forEach((_, messageIndex) =>
        current.set(messageIndex, { block: index, tokens: null }),
      );
      entries.push(new Map(current));
      return;
    }

    const delta = (total as number) - (priorTotal as number);
    const appended = call.messages
      .map((_, messageIndex) => messageIndex)
      .filter((messageIndex) => !current.has(messageIndex));
    // Tokens read from cache beyond the prior prompt are the ones the server generated and kept —
    // the assistant message that entered, priced without touching `completion_tokens`.
    const cached = call.usage.cache_read_tokens;
    const generated =
      cached === null || cached === undefined
        ? 0
        : Math.max(0, Math.min(cached - (priorTotal as number), delta));
    const pinnedAt = generated > 0 ? appended[0] : undefined;
    const rest = appended.filter((messageIndex) => messageIndex !== pinnedAt);
    for (const messageIndex of appended) {
      if (messageIndex === pinnedAt) {
        current.set(messageIndex, { block: index, tokens: generated });
      } else {
        current.set(messageIndex, {
          block: index,
          tokens: rest.length === 1 ? delta - generated : null,
        });
      }
    }
    entries.push(new Map(current));
  });
  return entries;
}

/// One call's rows: the opening block's parts as shares of it, then each later block's messages.
function attributeCall(
  call: ModelInteraction,
  index: number,
  calls: ModelInteraction[],
  entry: Map<number, MessageEntry>,
  total: number,
): CallAttribution {
  const openingIndex = entry.get(0)?.block ?? index;
  const openingTotal = calls[openingIndex]?.usage.prompt_tokens ?? total;
  const blocks: AttributionBlock[] = [];
  const rows: AttributedRow[] = [];

  const parts = openingParts(call, entry, openingIndex);
  const openingChars = parts.reduce((sum, part) => sum + part.chars, 0);
  const opening: AttributionBlock = {
    key: "block:opening",
    label: "opening prompt",
    tokens: openingTotal,
    rows: parts.map((part) => part.row.key),
  };
  blocks.push(opening);
  for (const part of parts) {
    rows.push({
      ...part.row,
      block: opening.key,
      cost: {
        kind: "share",
        percent: openingChars > 0 ? (part.chars / openingChars) * 100 : 0,
        block: opening.key,
        blockTokens: openingTotal,
      },
    });
  }

  const later = [...new Set([...entry.values()].map((value) => value.block))]
    .filter((block) => block !== openingIndex)
    .sort((a, b) => a - b);
  for (const block of later) {
    const priorTotal = calls[block - 1]?.usage.prompt_tokens ?? 0;
    const blockTokens = (calls[block]?.usage.prompt_tokens ?? 0) - priorTotal;
    const members = [...entry.entries()]
      .filter(([, value]) => value.block === block)
      .sort(([a], [b]) => a - b);
    const chars = members.reduce(
      (sum, [messageIndex]) => sum + messageChars(call.messages[messageIndex]),
      0,
    );
    const definition: AttributionBlock = {
      key: `block:${block}`,
      label: "appended",
      tokens: blockTokens,
      rows: members.map(([messageIndex]) => `message:${messageIndex}`),
    };
    blocks.push(definition);
    for (const [messageIndex, value] of members) {
      const message = call.messages[messageIndex];
      rows.push({
        key: `message:${messageIndex}`,
        label: messageLabel(message),
        messageIndex,
        block: definition.key,
        cost:
          value.tokens === null
            ? {
                kind: "share",
                percent: chars > 0 ? (messageChars(message) / chars) * 100 : 0,
                block: definition.key,
                blockTokens,
              }
            : { kind: "measured", tokens: value.tokens },
      });
    }
  }

  return { rows, blocks, total, totalProvenance: "measured" };
}

/// The opening block's parts: the system sections, the tools, and the messages that entered with
/// them.
function openingParts(
  call: ModelInteraction,
  entry: Map<number, MessageEntry>,
  openingIndex: number,
): Array<{ chars: number; row: Omit<AttributedRow, "cost" | "block"> }> {
  const parts: Array<{ chars: number; row: Omit<AttributedRow, "cost" | "block"> }> = [];
  for (const section of resolveSections(call.system, call.systemSections)) {
    parts.push({
      chars: section.end - section.start,
      row: {
        key: `section:${section.kind}`,
        label: sectionLabel(section.kind),
        sectionKind: section.kind,
      },
    });
  }
  if (call.tools.length > 0) {
    parts.push({
      chars: JSON.stringify(call.tools).length,
      row: { key: "tools", label: "tools" },
    });
  }
  call.messages.forEach((message, messageIndex) => {
    if (entry.get(messageIndex)?.block !== openingIndex) return;
    parts.push({
      chars: messageChars(message),
      row: { key: `message:${messageIndex}`, label: messageLabel(message), messageIndex },
    });
  });
  return parts;
}

/// No measurement at all: one estimated block over the whole prompt, its parts shares of it. The
/// provider reported no usage, so this is the only place a token count is invented.
function estimated(call: ModelInteraction): CallAttribution {
  const entry = new Map<number, MessageEntry>();
  call.messages.forEach((_, messageIndex) => entry.set(messageIndex, { block: 0, tokens: null }));
  const parts = openingParts(call, entry, 0);
  const chars = parts.reduce((sum, part) => sum + part.chars, 0);
  const total = estimateTokensFromChars(chars);
  const block: AttributionBlock = {
    key: "block:opening",
    label: "whole prompt",
    tokens: total,
    rows: parts.map((part) => part.row.key),
  };
  return {
    rows: parts.map((part) => ({
      ...part.row,
      block: block.key,
      cost: {
        kind: "share" as const,
        percent: chars > 0 ? (part.chars / chars) * 100 : 0,
        block: block.key,
        blockTokens: total,
      },
    })),
    blocks: [block],
    total,
    totalProvenance: "estimated",
  };
}

/// The tokens a row implies — its own measurement, or its share of its block. For the stacked bar,
/// which is a shape rather than a claim: a share row's slice is proportionate, and the number behind
/// it is never printed.
export function rowWeight(cost: RowCost): number {
  return cost.kind === "measured" ? cost.tokens : (cost.percent / 100) * cost.blockTokens;
}

/// A row's provenance badge.
export function rowProvenance(cost: RowCost, totalProvenance: TokenProvenance): TokenProvenance {
  if (cost.kind === "measured") return "measured";
  return totalProvenance === "estimated" ? "estimated" : "share";
}

/// Message objects are shared along a group's reconstruction prefix, so the serialized size is
/// computed once per distinct message rather than once per call that carries it.
const messageCharsCache = new WeakMap<Message, number>();

/// How much of the prompt a message occupies, in characters — the weight a block's shares are cut
/// by. An image part serialises to its address and media type, some ninety characters, where the
/// model is billed for the picture; it is charged [`IMAGE_CHARS`] so it does not read as free.
function messageChars(message: Message | undefined): number {
  if (!message) return 0;
  const cached = messageCharsCache.get(message);
  if (cached !== undefined) return cached;
  const images = message.images ?? [];
  const chars = JSON.stringify({ ...message, images: [] }).length + images.length * IMAGE_CHARS;
  messageCharsCache.set(message, chars);
  return chars;
}

/// What one image is charged when cutting a block's shares. A vision backend bills an image at a few
/// hundred to a couple of thousand tokens by its dimensions; this is the middle of that band at four
/// characters per token.
const IMAGE_CHARS = 4_000;

/// A message row's label. The role alone: every message row renders its content immediately beneath
/// itself, so an excerpt in the heading is the same text twice.
function messageLabel(message: Message | undefined): string {
  return message ? message.role : "message";
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
