import type { InFlightGeneration } from "../../lib/model/inflight.ts";
import { ThinkingMarkdown } from "../../components/ThinkingMarkdown.tsx";
import { WorkingPulse } from "../../components/primitives.tsx";

/// The deliberation's streaming tail: the step currently generating, rendered inside the turn's
/// collapsible below the committed steps — the same list position it will occupy once its
/// `ModelCalled` lands, so nothing shifts when it commits. Reasoning only, in the deliberation's
/// faint register (plus a clay "attempt N" when the retry wrapper restarted it): the reply streams
/// into the turn's message body instead, mirroring the committed layout, where the deliberation
/// holds the thinking and the turn text holds the speech.
export function StreamingStep({ generation }: { generation: InFlightGeneration }) {
  return (
    <div className="text-xs">
      <div className="flex items-baseline gap-2 font-mono text-2xs uppercase tracking-widest text-ink-faint">
        <WorkingPulse className="self-center" />
        <span>
          {generation.phase === "Synthesis" ? "synthesising" : `step ${generation.step + 1}`}
        </span>
        {generation.restarts > 0 && (
          <span className="normal-case tracking-normal text-clay">
            · attempt {generation.restarts + 1}
          </span>
        )}
      </div>
      {generation.reasoning && (
        <div className="mt-1.5 text-ink-faint">
          <ThinkingMarkdown text={generation.reasoning} />
        </div>
      )}
    </div>
  );
}
