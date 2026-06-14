import type { ReactNode } from "react";

import type { EventPayload } from "../types/EventPayload.ts";
import { refName } from "../lib/events.ts";
import {
  completionSummary,
  isPrivate,
  tellerLabel,
  terminalCauseLabel,
  visibilityLabel,
} from "../lib/labels.ts";
import { formatMs } from "../lib/format.ts";
import { Lua } from "../components/Lua.tsx";
import { Json } from "../components/Json.tsx";

/// The expanded view of a single event, rendered for its kind: the high-value payloads get a
/// bespoke layout (a Lua block highlighted, a model call's reasoning and token usage, an entry's
/// teller and visibility), and everything else falls back to its pretty-printed JSON. This is where
/// the log stops being a stream of one-liners and becomes inspectable.
export function EventDetail({
  payload,
  nameById,
}: {
  payload: EventPayload;
  nameById: Map<string, string>;
}) {
  const ref = (id: string) => refName(id, nameById);

  switch (payload.type) {
    case "MemoryContentAppended":
      return (
        <Fields>
          <Field label="memory">{ref(payload.id)}</Field>
          <Field label="text">
            <span className="text-ink">{payload.text}</span>
          </Field>
          <Field label="told by">{tellerLabel(payload.told_by, nameById)}</Field>
          <Field label="visibility">
            <span className={isPrivate(payload.visibility) ? "text-clay" : undefined}>
              {visibilityLabel(payload.visibility, nameById)}
            </span>
          </Field>
          {payload.told_in && <Field label="told in">{ref(payload.told_in)}</Field>}
        </Fields>
      );

    case "ModelCalled":
      return (
        <Fields>
          <Field label="phase">{payload.phase}</Field>
          {payload.reasoning && (
            <Field label="reasoning">
              <span className="font-serif italic text-ink-soft">{payload.reasoning}</span>
            </Field>
          )}
          <Field label="completion">{completionSummary(payload.completion)}</Field>
          {payload.finish_reason && <Field label="finish">{payload.finish_reason}</Field>}
          <Field label="tokens">
            {payload.usage.total_tokens ?? "—"}
            <span className="text-ink-faint">
              {" "}
              ({payload.usage.prompt_tokens ?? "?"} in · {payload.usage.completion_tokens ?? "?"}{" "}
              out)
            </span>
          </Field>
          <Field label="duration">{formatMs(Number(payload.duration_ms))}</Field>
        </Fields>
      );

    case "LuaExecuted":
      return (
        <div className="flex flex-col gap-2">
          <Lua code={payload.script} />
          {payload.terminal_cause ? (
            <p className="font-mono text-2xs text-clay">
              {terminalCauseLabel(payload.terminal_cause)}
            </p>
          ) : (
            payload.result && (
              <Fields>
                <Field label="result">
                  <span className="whitespace-pre-wrap">{payload.result}</span>
                </Field>
              </Fields>
            )
          )}
          {payload.touched.length > 0 && (
            <Fields>
              <Field label="touched">{payload.touched.map(ref).join(", ")}</Field>
              <Field label="duration">{formatMs(Number(payload.duration_ms))}</Field>
            </Fields>
          )}
        </div>
      );

    case "ConversationTurn":
      return (
        <Fields>
          <Field label="role">{payload.role}</Field>
          {payload.participant && <Field label="speaker">{ref(payload.participant)}</Field>}
          <Field label="text">
            <span className="text-ink">{payload.text || "(silent)"}</span>
          </Field>
          {payload.initiation === "Initiated" && <Field label="initiation">unprompted</Field>}
        </Fields>
      );

    case "LinkCreated":
      return (
        <Fields>
          <Field label="from">{ref(payload.from)}</Field>
          <Field label="relation">{payload.relation}</Field>
          <Field label="to">{ref(payload.to)}</Field>
          <Field label="source">{payload.source}</Field>
        </Fields>
      );

    case "TagCreated":
      return (
        <Fields>
          <Field label="tag">#{payload.name}</Field>
          <Field label="purpose">{payload.description}</Field>
        </Fields>
      );

    case "SessionStarted":
      return (
        <div className="flex flex-col gap-2">
          <Fields>
            <Field label="present">{payload.participants.map(ref).join(", ") || "no one"}</Field>
          </Fields>
          <pre className="max-h-72 overflow-auto whitespace-pre-wrap border-l border-line bg-oat/40 px-3 py-2 font-mono text-2xs leading-relaxed text-ink-soft">
            {payload.brief}
          </pre>
        </div>
      );

    case "BeliefArbitrated":
      return (
        <Fields>
          <Field label="memory">{ref(payload.memory)}</Field>
          <Field label="statement">
            <span className="text-ink">{payload.resolution.statement}</span>
          </Field>
          <Field label="competing">{payload.competing_entries.length} entries</Field>
        </Fields>
      );

    case "MemoryDescriptionRegenerated":
      return (
        <Fields>
          <Field label="memory">{ref(payload.id)}</Field>
          <Field label="description">
            <span className="font-serif text-ink-soft">{payload.new_text}</span>
          </Field>
        </Fields>
      );

    default:
      return <Json value={payload} />;
  }
}

function Fields({ children }: { children: ReactNode }) {
  return <div className="flex flex-col font-mono text-2xs text-ink-soft">{children}</div>;
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[6rem_1fr] gap-3 py-0.5">
      <span className="text-ink-faint">{label}</span>
      <span className="leading-relaxed">{children}</span>
    </div>
  );
}
