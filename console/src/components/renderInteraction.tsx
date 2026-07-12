import type { ReactNode } from "react";

import type { ConversationRef } from "../types/ConversationRef.ts";
import {
  completionSummary,
  isPrivate,
  tellerLabel,
  terminalCauseLabel,
  visibilityLabel,
} from "../lib/model/labels.ts";
import { formatMs } from "../lib/format/format.ts";
import { Lua } from "../components/Lua.tsx";
import { ThinkingMarkdown } from "../components/ThinkingMarkdown.tsx";
import { Fields, Field, Tree } from "./Tree.tsx";
import { Mono, Prose, Ref, RefList, ConversationRefLink } from "./eventDetailParts.tsx";
import type { RenderContext } from "./renderPayload.tsx";

/// Render the second half of payload cases: tags, links, config, Lua, model calls, and conversation
/// events. Returns `undefined` if the payload type does not match any case here (so the caller can
/// fall through to `renderMemoryPayload` or the default tree).
export function renderInteractionPayload(ctx: RenderContext): ReactNode {
  const { payload, nameById, conversationNameById, base, seq } = ctx;
  const ref = (id: string) => <Ref id={id} nameById={nameById} base={base} seq={seq} />;
  const refs = (ids: string[], empty?: string) => (
    <RefList ids={ids} nameById={nameById} base={base} seq={seq} empty={empty} />
  );
  const convRef = (value: ConversationRef) => (
    <ConversationRefLink
      value={value}
      nameById={nameById}
      conversationNameById={conversationNameById}
      base={base}
      seq={seq}
    />
  );
  switch (payload.type) {
    case "TagCreated":
      return (
        <Fields>
          <Field label="tag">#{payload.name}</Field>
          <Field label="purpose">{payload.description}</Field>
        </Fields>
      );

    case "TagDescriptionChanged":
      return (
        <Fields>
          <Field label="tag">#{payload.name}</Field>
          <Field label="purpose">{payload.new_description}</Field>
        </Fields>
      );

    case "TagAppliedToMemory":
      return (
        <Fields>
          <Field label="memory">{ref(payload.memory)}</Field>
          <Field label="tag">#{payload.tag}</Field>
        </Fields>
      );

    case "TagRemovedFromMemory":
      return (
        <Fields>
          <Field label="memory">{ref(payload.memory)}</Field>
          <Field label="tag">#{payload.tag}</Field>
        </Fields>
      );

    case "LinkTypeRegistered":
      return (
        <Fields>
          <Field label="relation">{payload.name}</Field>
          <Field label="inverse">{payload.inverse}</Field>
          <Field label="cardinality">
            {payload.from_card} → {payload.to_card}
          </Field>
          {(payload.symmetric || payload.reflexive) && (
            <Field label="flags">
              {[payload.symmetric && "symmetric", payload.reflexive && "reflexive"]
                .filter(Boolean)
                .join(", ")}
            </Field>
          )}
        </Fields>
      );

    case "LinkCreated":
      return (
        <Fields>
          <Field label="from">{ref(payload.from)}</Field>
          <Field label="relation">{payload.relation}</Field>
          <Field label="to">{ref(payload.to)}</Field>
          <Field label="source">{payload.source}</Field>
          {payload.told_by && (
            <Field label="told by">{tellerLabel(payload.told_by, nameById)}</Field>
          )}
          {payload.told_in && <Field label="told in">{convRef(payload.told_in)}</Field>}
          <Field label="visibility">
            <span className={isPrivate(payload.visibility) ? "text-clay" : undefined}>
              {visibilityLabel(payload.visibility, nameById)}
            </span>
          </Field>
        </Fields>
      );

    case "LinkRemoved":
      return (
        <Fields>
          <Field label="from">{ref(payload.from)}</Field>
          <Field label="relation">{payload.relation}</Field>
          <Field label="to">{ref(payload.to)}</Field>
        </Fields>
      );

    case "PromptTemplateRegistered":
      return (
        <div className="flex flex-col gap-2">
          <Fields>
            <Field label="template">{payload.name}</Field>
            <Field label="version">v{payload.version}</Field>
            <Field label="source">{payload.source}</Field>
          </Fields>
          <Prose>{payload.body}</Prose>
        </div>
      );

    case "ConfigSet":
      return (
        <div className="flex flex-col gap-2">
          <Fields>
            <Field label="source">{payload.source}</Field>
          </Fields>
          <Tree value={payload.settings} />
        </div>
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
              <Field label="touched">{refs(payload.touched)}</Field>
              <Field label="duration">{formatMs(Number(payload.duration_ms))}</Field>
            </Fields>
          )}
        </div>
      );

    case "ModelCallAborted":
      return (
        <Fields>
          <Field label="phase">{payload.phase}</Field>
          <Field label="attempt">{`${payload.attempt} (discarded)`}</Field>
          <Field label="cause">{payload.cause}</Field>
          {payload.partial_reasoning && (
            <Field label="discarded reasoning">
              <span className="text-ink-faint line-through">{payload.partial_reasoning}</span>
            </Field>
          )}
          {payload.partial_reply && (
            <Field label="discarded reply">
              <span className="text-ink-faint line-through">{payload.partial_reply}</span>
            </Field>
          )}
        </Fields>
      );

    case "ModelCalled":
      return (
        <Fields>
          <Field label="phase">{payload.phase}</Field>
          {payload.reasoning && (
            <Field label="reasoning">
              <div className="font-serif">
                <ThinkingMarkdown text={payload.reasoning} />
              </div>
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
            {/* Loose != null: a pre-capture event's raw JSON has no cache keys at all. */}
            {payload.usage.cache_read_tokens != null && (
              <span className="text-ink-faint"> · {payload.usage.cache_read_tokens} cached</span>
            )}
            {payload.usage.cache_write_tokens != null && (
              <span className="text-ink-faint">
                {" "}
                · {payload.usage.cache_write_tokens} cache-written
              </span>
            )}
          </Field>
          <Field label="duration">{formatMs(Number(payload.duration_ms))}</Field>
        </Fields>
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

    case "ConversationStarted":
      return (
        <Fields>
          <Field label="platform">{payload.locator.platform}</Field>
          <Field label="scope">{payload.locator.scope_path}</Field>
          <Field label="context">{ref(payload.context_memory)}</Field>
        </Fields>
      );

    case "ConversationEnded":
      return (
        <Fields>
          <Field label="conversation">
            <Mono>{payload.id}</Mono>
          </Field>
        </Fields>
      );

    case "SessionStarted":
      return (
        <div className="flex flex-col gap-2">
          <Fields>
            <Field label="present">{refs(payload.participants, "no one")}</Field>
            {payload.seeded_from_turn && (
              <Field label="seeded from">{convRef(payload.seeded_from_turn)}</Field>
            )}
            {/* Loose ?? []: a pre-capture event's raw JSON has no working_set key at all. */}
            {(payload.working_set ?? []).length > 0 && (
              <Field label="working set">{refs(payload.working_set, "none")}</Field>
            )}
          </Fields>
          <Prose>{payload.brief}</Prose>
        </div>
      );

    case "SessionEnded":
      return (
        <Fields>
          <Field label="session">
            <Mono>{payload.id}</Mono>
          </Field>
        </Fields>
      );

    case "ParticipantJoined":
      return (
        <Fields>
          <Field label="participant">{ref(payload.participant)}</Field>
          <Field label="at turn">{convRef(payload.at_turn)}</Field>
        </Fields>
      );

    case "ParticipantIdentified":
      return (
        <Fields>
          <Field label="memory">{ref(payload.memory)}</Field>
          <Field label="platform">{payload.platform}</Field>
          <Field label="user id">
            <Mono>{payload.platform_user_id}</Mono>
          </Field>
        </Fields>
      );

    default:
      return undefined;
  }
}
