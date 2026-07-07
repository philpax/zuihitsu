import type { ModelInteraction } from "../../lib/model/interactions.ts";
import type { TurnModel } from "../../lib/model/conversation.ts";

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
