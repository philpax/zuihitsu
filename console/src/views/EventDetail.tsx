import { Fragment, type ReactNode } from "react";
import { Link } from "react-router-dom";

import type { EventPayload } from "../types/EventPayload.ts";
import type { ProducedBy } from "../types/ProducedBy.ts";
import type { TemporalRef } from "../types/TemporalRef.ts";
import { refName } from "../lib/events.ts";
import {
  completionSummary,
  isPrivate,
  tellerLabel,
  terminalCauseLabel,
  visibilityLabel,
} from "../lib/labels.ts";
import { formatDateTime, formatMs } from "../lib/format.ts";
import { rruleLabel } from "../lib/audit.ts";
import { statePath } from "../lib/routes.ts";
import { Lua } from "../components/Lua.tsx";
import { ThinkingMarkdown } from "../components/ThinkingMarkdown.tsx";

/// The expanded view of a single event, rendered for its kind. Every payload gets a bespoke,
/// label-and-value layout — a Lua block highlighted, a model call's reasoning and token usage, an
/// entry's teller and visibility — and the handful with no dedicated case fall to a readable field
/// tree rather than a raw JSON dump. This is where the log stops being a stream of one-liners and
/// becomes inspectable.
///
/// When `base` (the stream's path) and `seq` (this event's seq) are given, every memory the event
/// references becomes a link into the State view folded to that seq with the memory open — so an
/// event's mention of a memory carries you to it at the point in the timeline it happened. Without
/// them the references render as plain names (the viewer is then usable outside a routed stream).
/// `recordedAt`, when given, prints the wall-clock time the event was committed beneath the body.
export function EventDetail({
  payload,
  nameById,
  base,
  seq,
  recordedAt,
}: {
  payload: EventPayload;
  nameById: Map<string, string>;
  base?: string;
  seq?: number;
  recordedAt?: number;
}) {
  const ref = (id: string) => <Ref id={id} nameById={nameById} base={base} seq={seq} />;
  const refs = (ids: string[], empty?: string) => (
    <RefList ids={ids} nameById={nameById} base={base} seq={seq} empty={empty} />
  );

  return (
    <div className="flex flex-col gap-2">
      {renderPayload()}
      {recordedAt != null && (
        <p className="font-mono text-2xs text-ink-faint">at {formatDateTime(recordedAt)}</p>
      )}
    </div>
  );

  function renderPayload() {
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
            {payload.told_in && <Field label="told in">{ref(payload.told_in)}</Field>}
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
            {payload.produced_by && (
              <Field label="by">{producedByLabel(payload.produced_by)}</Field>
            )}
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
            {payload.produced_by && (
              <Field label="by">{producedByLabel(payload.produced_by)}</Field>
            )}
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
            {payload.produced_by && (
              <Field label="by">{producedByLabel(payload.produced_by)}</Field>
            )}
          </Fields>
        );

      case "MemoryVolatilitySet":
        return (
          <Fields>
            <Field label="memory">{ref(payload.id)}</Field>
            <Field label="volatility">{payload.volatility}</Field>
          </Fields>
        );

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
              {payload.seeded_from_turn && <Field label="seeded from">a prior session</Field>}
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
        return <Tree value={payload} />;
    }
  }
}

/// A memory reference: the memory's name, a link into the State view at this event's seq when the
/// stream's `base` and the `seq` are known and the id names a memory, plain text otherwise.
function Ref({
  id,
  nameById,
  base,
  seq,
}: {
  id: string;
  nameById: Map<string, string>;
  base?: string;
  seq?: number;
}) {
  const name = refName(id, nameById);
  const to = base != null && seq != null && nameById.has(id) ? statePath(base, seq, name) : null;
  if (!to) return <>{name}</>;
  return (
    <Link
      to={to}
      title="Open this memory in State, at this point in the timeline"
      className="text-clay underline-offset-2 transition-colors hover:text-ink hover:underline"
    >
      {name}
    </Link>
  );
}

/// A comma-separated list of memory references, each a link under the same rules as [`Ref`].
function RefList({
  ids,
  nameById,
  base,
  seq,
  empty = "—",
}: {
  ids: string[];
  nameById: Map<string, string>;
  base?: string;
  seq?: number;
  empty?: string;
}) {
  if (ids.length === 0) return <>{empty}</>;
  return (
    <>
      {ids.map((id, index) => (
        <Fragment key={index}>
          {index > 0 && ", "}
          <Ref id={id} nameById={nameById} base={base} seq={seq} />
        </Fragment>
      ))}
    </>
  );
}

/// A human label for an entry's resolved time, across the temporal-reference variants.
function temporalRefLabel(ref: TemporalRef): string {
  if ("instant" in ref) return formatDateTime(ref.instant);
  if ("day" in ref) return ref.day;
  if ("range" in ref)
    return `${formatDateTime(ref.range.start)} – ${formatDateTime(ref.range.end)}`;
  if ("approx" in ref) return `~${formatDateTime(ref.approx.center)} (±${ref.approx.fuzz_days}d)`;
  if ("recurring" in ref) return rruleLabel(ref.recurring);
  return `${ref.before_after.dir} ${ref.before_after.anchor}`;
}

/// Who produced a derived event — the model and prompt template behind it.
function producedByLabel(by: ProducedBy): string {
  return `${by.model_id} · ${by.template_name} v${by.template_version}`;
}

function Mono({ children }: { children: ReactNode }) {
  return <span className="break-all text-ink-soft">{children}</span>;
}

/// A long text body (a brief, a prompt template) — the content itself, not a JSON dump.
function Prose({ children }: { children: string }) {
  return (
    <pre className="max-h-72 overflow-auto whitespace-pre-wrap border-l border-line bg-oat/40 px-3 py-2 font-mono text-2xs leading-relaxed text-ink-soft">
      {children}
    </pre>
  );
}

/// The readable fallback for a payload with no bespoke case: nested label/value rows rather than a
/// raw code block, so even an unforeseen event type stays legible.
function Tree({ value }: { value: unknown }) {
  if (value === null || value === undefined) return <span className="text-ink-faint">—</span>;
  if (Array.isArray(value)) {
    if (value.length === 0) return <span className="text-ink-faint">(none)</span>;
    return (
      <div className="flex flex-col gap-1">
        {value.map((item, index) => (
          <Tree key={index} value={item} />
        ))}
      </div>
    );
  }
  if (typeof value === "object") {
    return (
      <Fields>
        {Object.entries(value as Record<string, unknown>).map(([key, child]) => (
          <Field key={key} label={key}>
            <Tree value={child} />
          </Field>
        ))}
      </Fields>
    );
  }
  return <span className="text-ink">{String(value)}</span>;
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
