import type { Usage } from "@zuihitsu/wire/types/Usage.ts";
import type { CacheVerdict } from "../../lib/model/cachePath.ts";
import type { ModelInteraction } from "../../lib/model/interactions.ts";
import type { TurnModel } from "../../lib/model/conversation.ts";
import { sectionLabel } from "../../lib/model/tokenAttribution.ts";
import { formatTokens } from "../../lib/format/format.ts";

/// A turn's measured token cost. `context` is the *peak* prompt across its model calls — the largest
/// context the model read this turn. It is cumulative by nature (each step re-sends the whole growing
/// buffer, which itself carries every prior turn), and it is exactly what the compaction trigger
/// weighs against the budget (server/platform.rs: a turn compacts when its peak prompt crosses
/// `token_budget`). `output` is the *sum* of completions — the tokens the agent generated, which is
/// additive with no overlap. Both are 0 for a participant or system turn (no model call): a
/// participant message's own tokens are not measured, only folded into the next agent prompt.
export function turnTokens(
  turn: TurnModel,
  bySeq: Map<number, ModelInteraction>,
): { context: number; output: number } {
  let context = 0;
  let output = 0;
  for (const step of turn.deliberation) {
    if (step.kind !== "model") continue;
    const usage = bySeq.get(step.seq)?.usage;
    context = Math.max(context, usage?.prompt_tokens ?? 0);
    output += usage?.completion_tokens ?? 0;
  }
  return { context, output };
}

/// The fading arrival wash for the deep-linked turn, as a class suffix (see `turn-arrival` in
/// app.css) — appended so the wash rides whichever turn shape (system or spoken) the link lands on.
export function linkedClass(linked: boolean): string {
  return linked ? " turn-linked" : "";
}

/// How warm a call's cache actually was, as a fraction — the honest unit, since every call
/// re-prefills *something* (even a perfect continuation re-encodes its appended messages, and a
/// compaction seam still reuses the shared head). Measured from the provider's read count when
/// reported; the structural verdict word stands in when no numbers exist.
export interface Warmth {
  label: string;
  tone: "sage" | "soft" | "clay";
  title: string;
}

export function warmth(verdict: CacheVerdict, usage: Usage): Warmth {
  const read = usage.cache_read_tokens;
  const prompt = usage.prompt_tokens;
  if (read !== null && prompt !== null && prompt > 0) {
    const pct = Math.round((read / prompt) * 100);
    const cause = verdict.cause ? ` What broke the rest: ${causeLabel(verdict)}.` : "";
    return {
      label: `${pct}% warm`,
      tone: pct >= 90 ? "sage" : pct >= 50 ? "soft" : "clay",
      title: `The provider served ${formatTokens(read)} of the ${formatTokens(prompt)} prompt tokens from its prefix cache; the rest was encoded fresh. There is no binary hit or miss — a prefix cache reuses the longest shared head.${cause}`,
    };
  }
  return verdict.path === "warm"
    ? {
        label: "warm",
        tone: "sage",
        title:
          "The previous call's entire prompt is a strict prefix of this one — maximal possible reuse; only the appended slice is fresh. The provider reported no counts to measure it.",
      }
    : {
        label: "cold",
        tone: "clay",
        title: `The prompt does not extend the previous call's (${causeLabel(verdict)}), so reuse is partial at best — and the provider reported no counts to measure it.`,
      };
}

/// A cold cause in plain words, for tooltips and the breakdown's cache-break marker.
export function causeLabel(verdict: CacheVerdict): string {
  switch (verdict.cause) {
    case "first-call":
      return "the first call of the conversation";
    case "system-changed":
      return verdict.divergence?.sectionKind
        ? `the ${sectionLabel(verdict.divergence.sectionKind)} section changed`
        : "the system prompt changed";
    case "tools-changed":
      return "the tool set changed";
    case "tool-ids-reminted":
      return "only re-minted tool-call ids changed — the wire tokens barely move, so trust the measured warmth";
    case "new-session":
      return "a new session opened";
    case "buffer-rewritten":
      return "the buffer was rebuilt";
    case undefined:
      return "nothing changed";
  }
}
