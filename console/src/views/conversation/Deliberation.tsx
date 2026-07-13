import { useState } from "react";

import type { DeliberationStep } from "../../lib/model/conversation.ts";
import type { InFlightGeneration } from "../../lib/model/inflight.ts";
import { StreamingStep } from "./InFlightDeliberation.tsx";
import { completionSummary, terminalCauseLabel } from "../../lib/model/labels.ts";
import { formatMs } from "../../lib/format/format.ts";
import { Disclosure } from "../../components/primitives.tsx";
import { Lua } from "../../components/Lua.tsx";
import { ThinkingMarkdown } from "../../components/ThinkingMarkdown.tsx";
import { CallContext } from "./CallContext.tsx";

export function Deliberation({
  steps,
  inflight,
}: {
  steps: DeliberationStep[];
  /// The step currently generating (live mode): appended below the committed steps inside the
  /// collapsible — new steps grow downward, exactly as committed ones do. Collapsed by default,
  /// streaming included, so tokens never shift the layout unless the reader opted in by opening.
  /// The open state is this component's own; continuity across the pending → committed handover
  /// comes from the transcript rendering both through the same `TurnItem` under the same key, so
  /// React preserves the instance and the state simply survives.
  inflight?: InFlightGeneration | null;
}) {
  const [open, setOpen] = useState(false);
  const total = steps.reduce((sum, step) => sum + ("durationMs" in step ? step.durationMs : 0), 0);
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
        summary={
          inflight
            ? `· ${steps.length} step${steps.length === 1 ? "" : "s"} · generating…`
            : `· ${steps.length} step${steps.length > 1 ? "s" : ""} · ${formatMs(total)}`
        }
      />
      {open && (
        <div className="mt-3 flex flex-col gap-3 border-l border-line pl-4">
          {steps.map((step, index) =>
            step.kind === "model" ? (
              <ModelStep key={index} step={step} showContext={index !== lastModelIndex} />
            ) : step.kind === "aborted" ? (
              <AbortedStep key={index} step={step} />
            ) : step.kind === "ambient" ? (
              <AmbientStep key={index} step={step} />
            ) : (
              <LuaStep key={index} step={step} />
            ),
          )}
          {/* A superseded accumulation keeps the turn's transcript slot but yields its display:
              the committed step is the durable record, and showing both would read as a duplicate. */}
          {inflight && !inflight.superseded && <StreamingStep generation={inflight} />}
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

/// The pre-turn ambient recall hint — memories the frozen brief did not carry, surfaced by the
/// lexical pass and shown to the model as a system note before it generated. Rendered as quiet system
/// material (faint ink), the hint text verbatim. A token-only hint (a `[turn:<id>]` pointer with no
/// lexical hit) carries no memories, so the count is shown only when there are hits to count.
function AmbientStep({ step }: { step: Extract<DeliberationStep, { kind: "ambient" }> }) {
  return (
    <div>
      <div className="flex items-baseline gap-2 font-mono text-2xs text-ink-faint">
        <span className="lowercase">ambient recall</span>
        {step.memories.length > 0 && (
          <>
            <span className="text-ink-faint/45">·</span>
            <span>
              {step.memories.length} {step.memories.length === 1 ? "memory" : "memories"}
            </span>
          </>
        )}
      </div>
      <div className="mt-1 whitespace-pre-wrap font-mono text-xs text-ink-soft">{step.text}</div>
    </div>
  );
}

/// A discarded streaming attempt: the retry wrapper re-drove the call after a transient
/// mid-generation failure, and this is what was thrown away — rendered faint and struck through,
/// so the deliberation shows the restart without confusing the discarded text for the record.
function AbortedStep({ step }: { step: Extract<DeliberationStep, { kind: "aborted" }> }) {
  return (
    <div className="text-xs">
      <div className="font-mono text-2xs uppercase tracking-widest text-clay">
        attempt {step.attempt} discarded
        <span className="ml-2 normal-case tracking-normal text-ink-faint">· {step.cause}</span>
      </div>
      {(step.partialReasoning || step.partialReply) && (
        <div className="mt-1 text-ink-faint line-through opacity-60">
          {step.partialReasoning && <span>{step.partialReasoning} </span>}
          {step.partialReply && <span>{step.partialReply}</span>}
        </div>
      )}
    </div>
  );
}
