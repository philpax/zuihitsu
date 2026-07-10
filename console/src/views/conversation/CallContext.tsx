import { useContext, useState } from "react";

import { Disclosure } from "../../components/primitives.tsx";
import { warmth } from "./turnUtilities.ts";
import { ContextHeading, ContextSectionList } from "./ContextSectionList.tsx";
import { ModelCalls } from "./ConversationView.tsx";

/// The one context display, shared by every surface that shows a call's context — each deliberation
/// step, and the turn's footer (its final call). A single disclosure whose heading carries the
/// measured cache warmth, tokens in and out, the re-prefill cost, the compaction fill, and the
/// digest verification on one line; expanding it shows the full breakdown. `tokensOut` overrides
/// the call's own completion count where the caller aggregates (a turn's total generation across
/// its steps).
export function CallContext({ seq, tokensOut }: { seq: number; tokensOut?: number | null }) {
  const { bySeq, verdictBySeq, attributionBySeq, denominatorsBySeq, digestBySeq } =
    useContext(ModelCalls);
  const [open, setOpen] = useState(false);
  const interaction = bySeq.get(seq);
  const verdict = verdictBySeq.get(seq);
  const attribution = attributionBySeq.get(seq);
  if (!interaction || !verdict || !attribution) return null;
  // The denominators in effect when this call was made — not at the cursor, so a later settings
  // change never repaints an earlier call's numbers.
  const denominators = denominatorsBySeq.get(seq) ?? { budget: null, contextLength: null };
  const reprefilled =
    interaction.usage.cache_read_tokens !== null && interaction.usage.prompt_tokens !== null
      ? Math.max(0, interaction.usage.prompt_tokens - interaction.usage.cache_read_tokens)
      : null;

  return (
    <div className="mt-1.5">
      <Disclosure
        open={open}
        onToggle={() => setOpen(!open)}
        label="context"
        summary={
          <ContextHeading
            warm={warmth(verdict, interaction.usage)}
            tokensIn={interaction.usage.prompt_tokens ?? attribution.total}
            tokensOut={tokensOut ?? interaction.usage.completion_tokens}
            reprefilled={reprefilled}
            budget={denominators.budget}
            digest={digestBySeq.get(seq)}
          />
        }
      />
      {open && (
        <ContextSectionList
          interaction={interaction}
          attribution={attribution}
          verdict={verdict}
          denominator={
            denominators.contextLength !== null
              ? { value: denominators.contextLength, label: "context window" }
              : { value: denominators.budget, label: "compaction budget" }
          }
        />
      )}
    </div>
  );
}
