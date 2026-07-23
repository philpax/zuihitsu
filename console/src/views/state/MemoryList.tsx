import { useState } from "react";
import type { EntryView } from "@zuihitsu/wire/types/EntryView.ts";
import type { MemoryView } from "@zuihitsu/wire/types/MemoryView.ts";
import type { EntryId } from "@zuihitsu/wire/types/EntryId.ts";
import type { RecurringItem } from "../../lib/model/audit.ts";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { turnComponents } from "../../components/markdownComponents.tsx";
import {
  attestationHidden,
  connectorPlatform,
  isPrivate,
  tellerLabel,
  visibilityLabel,
} from "../../lib/model/labels.ts";
import { formatDateTime } from "../../lib/format/format.ts";
import { temporalRefLabel } from "../../components/eventDetailUtilities.ts";
import { Eyebrow } from "../../components/primitives.tsx";
import { groupByNamespace, leafName } from "./memoryUtilities.ts";

// Module-level plugin array so the React Compiler sees a stable object. Memory entries are
// agent-authored Markdown — GFM tables, lists, emphasis — but carry no turn references, so the
// turn-ref plugin is absent (unlike `TurnMarkdown`).
const entryMarkdownPlugins = [remarkGfm];

function MemoryRef({ name, onSelect }: { name: string; onSelect: (name: string) => void }) {
  return (
    <button
      onClick={() => onSelect(name)}
      title={`Open ${name}`}
      className="text-clay underline-offset-2 transition-colors hover:text-ink hover:underline"
    >
      {name}
    </button>
  );
}

export function MemoryList({
  memories,
  selected,
  recurring,
  onSelect,
}: {
  memories: MemoryView[];
  selected: string | null;
  recurring: Map<string, RecurringItem[]>;
  onSelect: (name: string) => void;
}) {
  const groups = groupByNamespace(memories);

  return (
    <nav className="flex flex-col gap-4 sm:gap-6">
      {groups.map(([namespace, items]) => (
        <div key={namespace}>
          <Eyebrow>{namespace}</Eyebrow>
          <ul className="mt-2 flex flex-col">
            {items.map((memory) => {
              const active = memory.name === selected;
              return (
                <li key={memory.id}>
                  <button
                    onClick={() => onSelect(memory.name)}
                    title={
                      memory.description ? `${memory.name} — ${memory.description}` : memory.name
                    }
                    className={
                      "-ml-3 flex w-full min-w-0 flex-col border-l-2 py-1 pl-2.5 text-left transition-colors " +
                      (active
                        ? "border-clay text-ink"
                        : "border-transparent text-ink-soft hover:text-ink")
                    }
                  >
                    <span className="flex w-full min-w-0 items-baseline font-mono text-xs">
                      <span className="truncate">{leafName(memory.name, namespace)}</span>
                      {recurring.has(memory.id) && (
                        <span className="ml-1.5 shrink-0 text-sage" title="recurring">
                          ↻
                        </span>
                      )}
                    </span>
                    {/* The synthesized description, clamped, so the list reads as a glanceable index of
                        what each memory is about rather than a bare list of names. */}
                    {memory.description && (
                      <span className="mt-0.5 line-clamp-2 text-2xs/snug text-ink-faint">
                        {memory.description}
                      </span>
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </nav>
  );
}

export function EntryItem({
  entry,
  nameById,
  faded,
  disputed,
  expanded,
  memoryName,
  onRetract,
}: {
  entry: EntryView;
  nameById: Map<string, string>;
  faded?: boolean;
  disputed?: boolean;
  /// Render the operator archaeology beside the entry: each corroborating attester with its posture,
  /// distinct phrasing, and — on a history read — its retracted attestations struck through. The
  /// compact metadata line keeps only the attester chips and the count badge without this.
  expanded?: boolean;
  /// The memory's name, passed so the retract button can address the entry by memory + entry id.
  memoryName?: string;
  /// Retract this entry under operator authority. Present only in the live agent frame at the head.
  onRetract?: (memory: string, entry: EntryId, reason: string) => Promise<void>;
}) {
  const priv = isPrivate(entry.visibility);
  // The founding attestation is `attestations[0]` (the reads order founding first), and it is the
  // same teller the row already renders as `told by`. The tail is the corroboration — a further
  // teller standing behind the same fact — so the chips never double-render the founding teller.
  const corroborations = entry.attestations.slice(1);
  const liveCorroborations = corroborations.filter((att) => att.retracted_reason === null);
  const liveCount = entry.attestations.filter((att) => att.retracted_reason === null).length;
  return (
    <li className={faded ? "opacity-55" : undefined}>
      <div className={"text-base/relaxed " + (faded ? "text-ink-soft line-through" : "text-ink")}>
        <ReactMarkdown remarkPlugins={entryMarkdownPlugins} components={turnComponents}>
          {entry.text}
        </ReactMarkdown>
      </div>
      <p className="mt-1 flex flex-wrap items-baseline gap-x-2.5 font-mono text-2xs text-ink-faint">
        {/* The entry id leads the line (faint, truncated), the same handle the agent supersedes or
            retracts by; the title carries the full id. */}
        <span className="text-ink-faint/60" title={entry.entry_id}>
          {entry.entry_id.slice(0, 10)}
        </span>
        <span className="text-ink-faint/45">·</span>
        {entry.retracted_reason !== null && (
          <>
            <span className="text-clay">retracted: {entry.retracted_reason}</span>
            <span className="text-ink-faint/45">·</span>
          </>
        )}
        {disputed && (
          <>
            <span className="text-clay">disputed</span>
            <span className="text-ink-faint/45">·</span>
          </>
        )}
        <span>told by {tellerLabel(entry.told_by, nameById)}</span>
        {/* A count badge when more than one live teller stands behind the fact — the founding teller
            plus its corroboration. */}
        {liveCount > 1 && (
          <span
            className="border border-line px-1 text-ink-faint"
            title={`${liveCount} tellers stand behind this fact`}
          >
            ×{liveCount}
          </span>
        )}
        {/* The compact corroboration: the further tellers as inline chips. A hidden corroboration
            (posture narrower than the entry's audience) wears the clay confidence idiom so the
            operator tells it apart from open corroboration at a glance — the agent-facing read drops
            it, the operator console keeps it. The expanded view lists these in full below instead. */}
        {!expanded &&
          liveCorroborations.map((att, index) => {
            const hidden = attestationHidden(att.posture, entry.visibility);
            return (
              <span key={index} className="contents">
                <span className="text-ink-faint/45">·</span>
                <span
                  className={hidden ? "text-clay" : undefined}
                  title={
                    hidden
                      ? `hidden corroboration (${visibilityLabel(att.posture, nameById)})`
                      : visibilityLabel(att.posture, nameById)
                  }
                >
                  also {tellerLabel(att.teller, nameById)}
                </span>
              </span>
            );
          })}
        <span className="text-ink-faint/45">·</span>
        <span className={priv ? "text-clay" : undefined}>
          {visibilityLabel(entry.visibility, nameById)}
        </span>
        {/* A connector-maintained attribute (a username, display name, or nickname the platform
            connector owns) is marked so it reads apart from an agent-recorded fact — the cleanup
            passes leave it untouched, since the connector supersedes it as the account changes. */}
        {connectorPlatform(entry.origin) && (
          <>
            <span className="text-ink-faint/45">·</span>
            <span
              className="text-sage"
              title="maintained by a platform connector; the cleanup passes leave it untouched"
            >
              via {connectorPlatform(entry.origin)}
            </span>
          </>
        )}
        <span className="text-ink-faint/45">·</span>
        <time dateTime={new Date(entry.asserted_at).toISOString()}>
          {formatDateTime(entry.asserted_at)}
        </time>
        {/* The bi-temporal pair: occurred beside asserted, with the extraction-resolved marker so a
            guessed date never masquerades as a stated one. */}
        {entry.occurred_at && (
          <>
            <span className="text-ink-faint/45">·</span>
            <span
              title={
                entry.occurred_authored
                  ? "the occurrence was authored at append"
                  : "the occurrence was resolved by the turn-end temporal extraction"
              }
            >
              occurred {temporalRefLabel(entry.occurred_at)}
              {!entry.occurred_authored && " (extracted)"}
            </span>
          </>
        )}
        {!faded && memoryName && onRetract && (
          <RetractButton memoryName={memoryName} entryId={entry.entry_id} onRetract={onRetract} />
        )}
      </p>
      {/* Operator archaeology: the corroborating attestations in full, each attester with its own
          posture and distinct phrasing. A hidden corroboration keeps the clay confidence idiom; a
          retracted one (present only on the history reads) reads struck through with its stated
          reason, the way a retracted entry does. */}
      {expanded && corroborations.length > 0 && (
        <ul className="mt-1.5 flex flex-col gap-1 border-l border-line pl-3 font-mono text-2xs text-ink-faint">
          {corroborations.map((att, index) => {
            const retracted = att.retracted_reason !== null;
            const hidden = attestationHidden(att.posture, entry.visibility);
            return (
              <li key={index}>
                <span className="flex flex-wrap items-baseline gap-x-2">
                  <span
                    className={
                      (retracted ? "text-ink-soft line-through " : hidden ? "text-clay " : "") +
                      "font-medium"
                    }
                  >
                    also {tellerLabel(att.teller, nameById)}
                  </span>
                  <span className="text-ink-faint/45">·</span>
                  <span className={hidden ? "text-clay" : undefined}>
                    {visibilityLabel(att.posture, nameById)}
                  </span>
                  {att.source_entry && (
                    <>
                      <span className="text-ink-faint/45">·</span>
                      <span title={att.source_entry}>
                        carried from {att.source_entry.slice(0, 10)}
                      </span>
                    </>
                  )}
                  {retracted && (
                    <>
                      <span className="text-ink-faint/45">·</span>
                      <span className="text-clay">withdrawn: {att.retracted_reason}</span>
                    </>
                  )}
                </span>
                {att.phrasing && (
                  <p className="mt-0.5 font-serif text-2xs/relaxed text-ink-soft italic">
                    “{att.phrasing}”
                  </p>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </li>
  );
}

/// A small inline retract control: click to reveal a reason input, then confirm to retract the
/// entry. The entry drops from live surfaces while remaining in history with the reason.
function RetractButton({
  memoryName,
  entryId,
  onRetract,
}: {
  memoryName: string;
  entryId: EntryId;
  onRetract: (memory: string, entry: EntryId, reason: string) => Promise<void>;
}) {
  const [confirming, setConfirming] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function commit() {
    const trimmed = reason.trim();
    if (!trimmed) return;
    setBusy(true);
    setError(null);
    try {
      await onRetract(memoryName, entryId, trimmed);
      setConfirming(false);
      setReason("");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  }

  if (!confirming) {
    return (
      <>
        <span className="text-ink-faint/45">·</span>
        <button
          onClick={() => setConfirming(true)}
          className="text-clay transition-colors hover:text-ink"
          title="Retract this entry under operator authority"
        >
          retract
        </button>
      </>
    );
  }

  return (
    <span className="mt-1 flex w-full flex-wrap items-center gap-2">
      <input
        value={reason}
        onChange={(e) => {
          setReason(e.target.value);
          setError(null);
        }}
        placeholder="reason for retraction…"
        autoFocus
        className="flex-1 border border-line bg-transparent px-2 py-1 font-mono text-2xs text-ink placeholder:text-ink-faint/60 focus:border-ink-faint focus:outline-none"
      />
      <button
        onClick={commit}
        disabled={busy || !reason.trim()}
        className="text-clay transition-colors hover:text-ink disabled:text-ink-faint/40"
      >
        confirm
      </button>
      <button
        onClick={() => {
          setConfirming(false);
          setReason("");
          setError(null);
        }}
        disabled={busy}
        className="text-ink-faint transition-colors hover:text-ink disabled:text-ink-faint/40"
      >
        cancel
      </button>
      {busy && <span className="text-ink-faint">working…</span>}
      {error && <span className="text-clay">{error}</span>}
    </span>
  );
}

export function Section({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <section className="mt-6">
      <Eyebrow>{label}</Eyebrow>
      <div className="mt-3">{children}</div>
    </section>
  );
}

export { MemoryRef };
