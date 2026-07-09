import type { ReactNode } from "react";

import type { EventPayload } from "../types/EventPayload.ts";
import { isPrivate, tellerLabel, visibilityLabel } from "../lib/model/labels.ts";
import { formatDateTime } from "../lib/format/format.ts";
import { Fields, Field, Tree } from "./Tree.tsx";
import { Mono, Ref, ConversationRefLink } from "./eventDetailParts.tsx";
import { producedByLabel, temporalRefLabel } from "./eventDetailUtilities.ts";
import { renderInteractionPayload } from "./renderInteraction.tsx";

/// The shared context the per-payload render functions receive — the event's payload, the name map,
/// and the `ref`/`refs` closures bound to the stream's base and seq (or `null` outside a routed
/// stream, where references render as plain names). `conversationNameById` maps conversation ids
/// to their room display name, so `ConversationRef` links can label the room.
export interface RenderContext {
  payload: EventPayload;
  nameById: Map<string, string>;
  conversationNameById: Map<string, string>;
  base?: string;
  seq?: number;
}

/// Render the first half of payload cases: genesis, memory lifecycle, and entry-level events.
export function renderMemoryPayload(ctx: RenderContext): ReactNode {
  const { payload, nameById, conversationNameById, base, seq } = ctx;
  const ref = (id: string) => <Ref id={id} nameById={nameById} base={base} seq={seq} />;
  switch (payload.type) {
    case "GenesisCompleted":
      return (
        <Fields>
          <Field label="manifest">
            <Mono>{payload.manifest_hash}</Mono>
          </Field>
          <Field label="templates">
            {Object.entries(payload.template_versions)
              .map(([name, version]) => `${name} v${version}`)
              .join(", ") || "none"}
          </Field>
        </Fields>
      );

    case "MemoryCreated":
      return (
        <Fields>
          <Field label="memory">{ref(payload.id)}</Field>
        </Fields>
      );

    case "MemoryRenamed":
      return (
        <Fields>
          <Field label="memory">{ref(payload.id)}</Field>
          <Field label="from">{payload.old_name}</Field>
          <Field label="to">{payload.new_name}</Field>
        </Fields>
      );

    case "MemoryDeleted":
      return (
        <Fields>
          <Field label="memory">{ref(payload.id)}</Field>
        </Fields>
      );

    case "MemoryContentAppended":
      return (
        <Fields>
          <Field label="memory">{ref(payload.id)}</Field>
          <Field label="text">
            <span className="text-ink">{payload.text}</span>
          </Field>
          {payload.occurred_at && (
            <Field label="occurred">{temporalRefLabel(payload.occurred_at)}</Field>
          )}
          <Field label="told by">{tellerLabel(payload.told_by, nameById)}</Field>
          <Field label="visibility">
            <span className={isPrivate(payload.visibility) ? "text-clay" : undefined}>
              {visibilityLabel(payload.visibility, nameById)}
            </span>
          </Field>
          {payload.told_in && (
            <Field label="told in">
              <ConversationRefLink
                value={payload.told_in}
                nameById={nameById}
                conversationNameById={conversationNameById}
                base={base}
                seq={seq}
              />
            </Field>
          )}
        </Fields>
      );

    case "MemorySuperseded":
      return (
        <Fields>
          <Field label="memory">{ref(payload.id)}</Field>
          <Field label="entry">
            <Mono>{payload.entry}</Mono>
          </Field>
          <Field label="superseded by">
            <Mono>{payload.superseded_by}</Mono>
          </Field>
        </Fields>
      );

    case "EntryTemporalResolved":
      return (
        <Fields>
          <Field label="memory">{ref(payload.id)}</Field>
          <Field label="occurred">{temporalRefLabel(payload.occurred_at)}</Field>
          {payload.produced_by && <Field label="by">{producedByLabel(payload.produced_by)}</Field>}
        </Fields>
      );

    case "EntryDescriptionMirrored":
      return (
        <Fields>
          <Field label="memory">{ref(payload.id)}</Field>
          <Field label="entry">
            <Mono>{payload.entry_id}</Mono>
          </Field>
        </Fields>
      );

    case "ScheduledJobFired":
      return (
        <Fields>
          <Field label="memory">{ref(payload.memory)}</Field>
          <Field label="fired">{formatDateTime(payload.fired_at)}</Field>
        </Fields>
      );

    case "ScheduledItemSurfaced":
      return (
        <Fields>
          <Field label="memory">{ref(payload.memory)}</Field>
          <Field label="surfaced">{formatDateTime(payload.surfaced_at)}</Field>
        </Fields>
      );

    case "MemoryDescriptionRegenerated":
      return (
        <Fields>
          <Field label="memory">{ref(payload.id)}</Field>
          <Field label="description">
            <span className="font-serif text-ink-soft">{payload.new_text}</span>
          </Field>
          {payload.produced_by && <Field label="by">{producedByLabel(payload.produced_by)}</Field>}
        </Fields>
      );

    case "BeliefArbitrated":
      return (
        <Fields>
          <Field label="memory">{ref(payload.memory)}</Field>
          <Field label="statement">
            <span className="text-ink">{payload.resolution.statement}</span>
          </Field>
          <Field label="competing">{payload.competing_entries.length} entries</Field>
          {payload.produced_by && <Field label="by">{producedByLabel(payload.produced_by)}</Field>}
        </Fields>
      );

    case "LinksInferred":
      return (
        <Fields>
          <Field label="memory">{ref(payload.memory)}</Field>
          {payload.result.new_relations.length > 0 && (
            <Field label="coined relations">
              {payload.result.new_relations.map((r) => (
                <div key={r.name}>
                  {r.name} / {r.inverse} ({r.from_card} → {r.to_card}
                  {r.symmetric && ", symmetric"}
                  {r.reflexive && ", reflexive"})
                </div>
              ))}
            </Field>
          )}
          {payload.result.links.length > 0 && (
            <Field label="inferred links">
              {payload.result.links.map((l, i) => (
                <div key={i}>
                  {l.direction === "to" ? "→" : "←"} {l.relation} {l.target}
                  <span className="text-ink-faint"> (entry {l.entry})</span>
                </div>
              ))}
            </Field>
          )}
          {payload.result.new_relations.length === 0 && payload.result.links.length === 0 && (
            <Field label="result">no relationships found</Field>
          )}
          {payload.produced_by && <Field label="by">{producedByLabel(payload.produced_by)}</Field>}
        </Fields>
      );

    case "MemoryVolatilitySet":
      return (
        <Fields>
          <Field label="memory">{ref(payload.id)}</Field>
          <Field label="volatility">{payload.volatility}</Field>
        </Fields>
      );

    default:
      return undefined;
  }
}

/// Dispatch a payload to its bespoke render, trying the interaction cases first (the larger set),
/// then the memory cases, then the readable tree fallback.
export function renderPayload(ctx: RenderContext): ReactNode {
  return renderInteractionPayload(ctx) ?? renderMemoryPayload(ctx) ?? <Tree value={ctx.payload} />;
}
