import { useState } from "react";

import type { Event } from "../types/Event.ts";
import type { Replica } from "../lib/replica.ts";
import { completionSummary } from "../lib/labels.ts";
import { type DeliberationStep, type TurnModel, buildConversations } from "../lib/conversation.ts";
import { formatMs } from "../lib/format.ts";
import { Eyebrow } from "../components/primitives.tsx";
import { Lua } from "../components/Lua.tsx";

/// The Conversation view: a run's rooms, each session's frozen brief, and the transcript — with
/// every agent turn openable to the reasoning and Lua that produced it. "What was the agent
/// thinking," made literal (spec §Observability).
export function ConversationView({
  replica,
  events,
  cursor,
}: {
  replica: Replica;
  events: Event[];
  cursor: number;
}) {
  const nameById = new Map(replica.memories("").map((memory) => [memory.id, memory.name]));
  const conversations = buildConversations(
    events.filter((event) => event.seq <= cursor),
    nameById,
  );
  const [room, setRoom] = useState(0);

  if (conversations.length === 0) {
    return (
      <div className="py-24 text-center text-sm text-ink-faint">No conversations in this run.</div>
    );
  }

  const conversation = conversations[Math.min(room, conversations.length - 1)];

  return (
    <div className="mx-auto max-w-prose">
      {conversations.length > 1 && (
        <div className="mb-8 flex gap-5 text-sm">
          {conversations.map((entry, index) => (
            <button
              key={entry.id}
              onClick={() => setRoom(index)}
              className={
                "transition-colors " +
                (index === room ? "text-ink" : "text-ink-faint hover:text-ink-soft")
              }
            >
              {entry.contextName ?? `${entry.platform}:${entry.scopePath}`}
            </button>
          ))}
        </div>
      )}

      <header className="mb-8">
        <h2 className="font-serif text-2xl text-ink">
          {conversation.contextName ?? "Conversation"}
        </h2>
        <p className="mt-1 font-mono text-2xs uppercase tracking-widest text-ink-faint">
          {conversation.platform} · {conversation.scopePath}
        </p>
      </header>

      {conversation.sessions.map((session) => (
        <BriefBlock key={session.id} brief={session.brief} participants={session.participants} />
      ))}

      <ol className="mt-2 flex flex-col">
        {conversation.turns.map((turn) => (
          <TurnItem key={turn.turnId} turn={turn} />
        ))}
      </ol>
    </div>
  );
}

function BriefBlock({ brief, participants }: { brief: string; participants: string[] }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="mb-6 border-b border-line pb-6">
      <button
        onClick={() => setOpen(!open)}
        className="flex items-baseline gap-3 text-left transition-colors hover:text-ink"
      >
        <Eyebrow>{open ? "▾ brief" : "▸ brief"}</Eyebrow>
        <span className="font-mono text-2xs text-ink-faint">
          {participants.join(", ") || "no participants"}
        </span>
      </button>
      {open && (
        <pre className="mt-4 max-h-96 overflow-auto whitespace-pre-wrap border-l border-line bg-oat/40 px-4 py-3 font-mono text-2xs leading-relaxed text-ink-soft">
          {brief}
        </pre>
      )}
    </div>
  );
}

function TurnItem({ turn }: { turn: TurnModel }) {
  if (turn.role === "System") {
    return (
      <li className="py-3 text-center">
        <span className="font-mono text-2xs text-ink-faint">{turn.text || "(system)"}</span>
      </li>
    );
  }

  const isAgent = turn.role === "Agent";
  return (
    <li className="border-b border-line/70 py-5 last:border-b-0">
      <div className="mb-1.5 flex items-baseline gap-2">
        <span
          className={
            "font-mono text-2xs uppercase tracking-widest " + (isAgent ? "text-sage" : "text-clay")
          }
        >
          {isAgent ? "the agent" : (turn.speaker ?? "someone")}
        </span>
        {turn.initiation === "Initiated" && (
          <span className="font-mono text-2xs text-ink-faint">· unprompted</span>
        )}
      </div>
      {turn.text ? (
        <p className="text-base leading-relaxed text-ink">{turn.text}</p>
      ) : (
        <p className="text-sm italic text-ink-faint">stayed silent</p>
      )}
      {turn.deliberation.length > 0 && <Deliberation steps={turn.deliberation} />}
    </li>
  );
}

function Deliberation({ steps }: { steps: DeliberationStep[] }) {
  const [open, setOpen] = useState(false);
  const total = steps.reduce((sum, step) => sum + step.durationMs, 0);

  return (
    <div className="mt-3">
      <button
        onClick={() => setOpen(!open)}
        className="font-mono text-2xs text-ink-faint transition-colors hover:text-ink-soft"
      >
        {open ? "▾" : "▸"} deliberation · {steps.length} step{steps.length > 1 ? "s" : ""} ·{" "}
        {formatMs(total)}
      </button>
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
        <p className="mt-1 font-serif text-sm italic leading-relaxed text-ink-soft">
          {step.reasoning}
        </p>
      )}
    </div>
  );
}

function LuaStep({ step }: { step: Extract<DeliberationStep, { kind: "lua" }> }) {
  const error = step.terminalCause;
  return (
    <div>
      <Lua code={step.script} />
      {error ? (
        <p className="mt-1 font-mono text-2xs text-clay">
          {"Error" in error ? `error: ${error.Error}` : `aborted: ${error.Aborted}`}
        </p>
      ) : (
        step.result && (
          <p className="mt-1 whitespace-pre-wrap font-mono text-2xs text-ink-soft">
            → {step.result}
          </p>
        )
      )}
    </div>
  );
}
