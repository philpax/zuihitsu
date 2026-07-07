import { useContext, useState } from "react";

import type { Message } from "../../types/Message.ts";
import type { DeliberationStep } from "../../lib/model/conversation.ts";
import { type ModelInteraction } from "../../lib/model/interactions.ts";
import { completionSummary, terminalCauseLabel } from "../../lib/model/labels.ts";
import { formatMs, formatTokens } from "../../lib/format/format.ts";
import { Disclosure, Eyebrow, Excerpt, Meter } from "../../components/primitives.tsx";
import { Lua } from "../../components/Lua.tsx";
import { ThinkingMarkdown } from "../../components/ThinkingMarkdown.tsx";
import { ModelCalls } from "./ConversationView.tsx";

export function Deliberation({ steps }: { steps: DeliberationStep[] }) {
  const [open, setOpen] = useState(false);
  const total = steps.reduce((sum, step) => sum + step.durationMs, 0);

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
              <ModelStep key={index} step={step} />
            ) : (
              <LuaStep key={index} step={step} />
            ),
          )}
        </div>
      )}
    </div>
  );
}

function ModelStep({ step }: { step: Extract<DeliberationStep, { kind: "model" }> }) {
  const { bySeq, budget } = useContext(ModelCalls);
  const interaction = bySeq.get(step.seq);
  const [showPrompt, setShowPrompt] = useState(false);
  return (
    <div>
      <div className="flex items-baseline gap-2 font-mono text-2xs text-ink-faint">
        <span className="lowercase">{step.phase}</span>
        <span className="text-ink-faint/45">·</span>
        <span>{completionSummary(step.completion)}</span>
        <span className="text-ink-faint/45">·</span>
        <span>{formatMs(step.durationMs)}</span>
      </div>
      {interaction && <ContextBar usage={interaction.usage} budget={budget} />}
      {step.reasoning && (
        <div className="mt-1 font-serif">
          <ThinkingMarkdown text={step.reasoning} />
        </div>
      )}
      {interaction && (interaction.system || interaction.messages.length > 0) && (
        <div className="mt-1.5">
          <Disclosure
            open={showPrompt}
            onToggle={() => setShowPrompt(!showPrompt)}
            label="prompt"
            summary={`· ${interaction.messages.length} message${
              interaction.messages.length === 1 ? "" : "s"
            }`}
          />
          {showPrompt && <Prompt interaction={interaction} />}
        </div>
      )}
    </div>
  );
}

/// How much of the context budget this call's prompt consumed — a slim bar filling toward the budget,
/// sage with headroom and clay as it nears it (where compaction looms), with the token counts.
function ContextBar({ usage, budget }: { usage: ModelInteraction["usage"]; budget: number }) {
  if (usage.prompt_tokens === null) return null;
  const fraction = budget > 0 ? usage.prompt_tokens / budget : 0;
  return (
    <div className="mt-1.5 flex items-center gap-3">
      <Meter fraction={fraction} className="w-24" />
      <span className="font-mono text-2xs text-ink-faint">
        {formatTokens(usage.prompt_tokens)} / {formatTokens(budget)} · {Math.round(fraction * 100)}%
        {usage.completion_tokens !== null && ` · +${formatTokens(usage.completion_tokens)} out`}
      </span>
    </div>
  );
}

/// The full prompt the model saw, reconstructed from the delta-encoded request: the system prompt,
/// the messages, and the tools offered.
function Prompt({ interaction }: { interaction: ModelInteraction }) {
  return (
    <div className="mt-2 flex flex-col gap-3 border-l border-line pl-4">
      <div>
        <Eyebrow>system</Eyebrow>
        <Block text={interaction.system || "(none)"} />
      </div>
      <div>
        <Eyebrow>messages</Eyebrow>
        <div className="mt-1 flex flex-col gap-2">
          {interaction.messages.map((message, index) => (
            <MessageRow key={index} message={message} />
          ))}
        </div>
      </div>
      {interaction.tools.length > 0 && (
        <p className="font-mono text-2xs text-ink-faint">
          tools · {interaction.tools.map((tool) => tool.name).join(", ")}
        </p>
      )}
    </div>
  );
}

function MessageRow({ message }: { message: Message }) {
  return (
    <div>
      <span className="font-mono text-2xs uppercase tracking-widest text-ink-faint">
        {message.role}
      </span>
      {message.content && <Block text={message.content} />}
      {message.tool_calls.length > 0 && (
        <p className="mt-1 font-mono text-xs text-clay">
          → {message.tool_calls.map((call) => call.name).join(", ")}
        </p>
      )}
    </div>
  );
}

function Block({ text }: { text: string }) {
  return <Excerpt className="mt-1">{text}</Excerpt>;
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
