import { useState } from "react";

import type { DeliberationStep } from "../../lib/model/conversation.ts";
import { completionSummary, terminalCauseLabel } from "../../lib/model/labels.ts";
import { formatMs } from "../../lib/format/format.ts";
import { Disclosure } from "../../components/primitives.tsx";
import { Lua } from "../../components/Lua.tsx";
import { ThinkingMarkdown } from "../../components/ThinkingMarkdown.tsx";
import { CallContext } from "./CallContext.tsx";

export function Deliberation({ steps }: { steps: DeliberationStep[] }) {
  const [open, setOpen] = useState(false);
  const total = steps.reduce((sum, step) => sum + step.durationMs, 0);
  // The turn's footer shows the final Step-phase call's context (the state the turn ended with),
  // so that step skips its own here — the two would be identical. A trailing synthesis call keeps
  // its context: its prompt is a different, unrelated request.
  const lastModelIndex = steps.findLastIndex(
    (step) => step.kind === "model" && step.phase === "Step",
  );

  return (
    <div className="mt-3">
      <Disclosure
        open={open}
        onToggle={() => setOpen(!open)}
        label="deliberation"
        summary={`· ${steps.length} step${steps.length > 1 ? "s" : ""} · ${formatMs(total)}`}
      />
      {open && (
        <div className="mt-3 flex flex-col gap-3 border-l border-line pl-4">
          {steps.map((step, index) =>
            step.kind === "model" ? (
              <ModelStep key={index} step={step} showContext={index !== lastModelIndex} />
            ) : (
              <LuaStep key={index} step={step} />
            ),
          )}
        </div>
      )}
    </div>
  );
}

function ModelStep({
  step,
  showContext,
}: {
  step: Extract<DeliberationStep, { kind: "model" }>;
  showContext: boolean;
}) {
  return (
    <div>
      <div className="flex items-baseline gap-2 font-mono text-2xs text-ink-faint">
        <span className="lowercase">{step.phase}</span>
        <span className="text-ink-faint/45">·</span>
        <span>{completionSummary(step.completion)}</span>
        <span className="text-ink-faint/45">·</span>
        <span>{formatMs(step.durationMs)}</span>
      </div>
      {step.reasoning && (
        <div className="mt-1 font-serif">
          <ThinkingMarkdown text={step.reasoning} />
        </div>
      )}
      {showContext && <CallContext seq={step.seq} />}
    </div>
  );
}

function LuaStep({ step }: { step: Extract<DeliberationStep, { kind: "lua" }> }) {
  const error = step.terminalCause;
  return (
    <div>
      <Lua code={step.script} />
      {error ? (
        <p className="mt-1 font-mono text-xs text-clay">{terminalCauseLabel(error)}</p>
      ) : (
        step.result && (
          <p className="mt-1 whitespace-pre-wrap font-mono text-xs text-ink-soft">
            → {step.result}
          </p>
        )
      )}
    </div>
  );
}
