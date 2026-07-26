import { useState } from "react";
import type { EntryView } from "@zuihitsu/wire/types/EntryView.ts";
import type { MemoryId } from "@zuihitsu/wire/types/MemoryId.ts";
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
import { Link } from "../../lib/nav/history.tsx";
import { useOptionalStream } from "../../lib/nav/useStreamLocation.ts";
import { temporalRefLabel } from "../../components/eventDetailUtilities.ts";
import { Eyebrow } from "../../components/primitives.tsx";
import { clusterByClass, groupByNamespace, leafName } from "./memoryUtilities.ts";

// Module-level plugin array so the React Compiler sees a stable object. Memory entries are
// agent-authored Markdown — GFM tables, lists, emphasis — but carry no turn references, so the
// turn-ref plugin is absent (unlike `TurnMarkdown`).
const entryMarkdownPlugins = [remarkGfm];

/// The desktop sidebar: memories grouped by namespace, and within each namespace clustered by `same_as`
/// identity class. A class renders as a collapsible node headed by its canonical primary; expanding it
/// reveals the other members (the platform stubs), each still selectable exactly as a leaf is. A memory
/// in no class renders as a plain leaf. Expansion is sticky and purely user-controlled: a cluster stays
/// open until its disclosure triangle is clicked shut, and a selection change never collapses it. A
/// deep-link into a hidden class member is the one exception — it expands that member's cluster so the
/// selection never lands behind a collapsed head, only ever adding to the open set.
export function MemoryList({
  memories,
  selected,
  recurring,
  primaryOf,
  designated,
  onSelect,
}: {
  memories: MemoryView[];
  selected: string | null;
  recurring: Map<string, RecurringItem[]>;
  /// Each memory's canonical `same_as` primary id, from `replica.memoryClasses()` — the key the sidebar
  /// clusters on.
  primaryOf: Map<MemoryId, MemoryId>;
  /// The ids the operator has pinned as their class's primary, marked on the cluster head.
  designated: Set<MemoryId>;
  onSelect: (name: string) => void;
}) {
  const groups = groupByNamespace(memories);
  const [expanded, setExpanded] = useState<Set<MemoryId>>(new Set());
  const toggle = (id: MemoryId) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  // Cluster each namespace's memories once, so the render and the deep-link auto-expand read the
  // same grouping.
  const clustersByNamespace = groups.map(
    ([namespace, items]) => [namespace, clusterByClass(items, primaryOf)] as const,
  );

  // Auto-expand on deep link: when the opened memory is a hidden class member (not its cluster's
  // head), reveal its cluster so the selection never lands behind a collapsed head. Expansion is
  // otherwise purely user-controlled — this only ever ADDS to the expanded set, so a plain selection
  // change never collapses anything and a cluster the operator closed by hand stays closed unless a
  // deep link into one of its hidden members reopens it. The adjustment runs during render (the
  // documented alternative to an effect for reconciling state with a changing input), guarded by the
  // last member cluster it opened so it fires once per distinct deep link rather than fighting a
  // manual collapse on every render.
  const selectedMemberPrimary =
    clustersByNamespace
      .flatMap(([, clusters]) => clusters)
      .find((cluster) => cluster.members.some((member) => member.name === selected))?.primary.id ??
    null;
  const [autoExpanded, setAutoExpanded] = useState<MemoryId | null>(null);
  if (selectedMemberPrimary !== null && selectedMemberPrimary !== autoExpanded) {
    setAutoExpanded(selectedMemberPrimary);
    setExpanded((prev) => {
      if (prev.has(selectedMemberPrimary)) return prev;
      const next = new Set(prev);
      next.add(selectedMemberPrimary);
      return next;
    });
  }

  return (
    <nav className="flex flex-col gap-4 sm:gap-6">
      {clustersByNamespace.map(([namespace, clusters]) => (
        <div key={namespace}>
          <Eyebrow>{namespace}</Eyebrow>
          <ul className="mt-2 flex flex-col">
            {clusters.map(({ primary, members }) => {
              const open = members.length > 0 && expanded.has(primary.id);
              return (
                <li key={primary.id}>
                  <MemoryRow
                    memory={primary}
                    namespace={namespace}
                    active={primary.name === selected}
                    recurring={recurring.has(primary.id)}
                    designated={members.length > 0 && designated.has(primary.id)}
                    onSelect={onSelect}
                    disclosure={
                      members.length > 0
                        ? { open, count: members.length, onToggle: () => toggle(primary.id) }
                        : undefined
                    }
                  />
                  {open && (
                    <ul className="flex flex-col">
                      {members.map((member) => (
                        <li key={member.id}>
                          <MemoryRow
                            memory={member}
                            namespace={namespace}
                            active={member.name === selected}
                            recurring={recurring.has(member.id)}
                            onSelect={onSelect}
                            nested
                          />
                        </li>
                      ))}
                    </ul>
                  )}
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </nav>
  );
}

/// One selectable row in the sidebar tree — a leaf, a cluster head (with a trailing disclosure
/// control), or a nested class member. Leaves and heads sit flush at the sidebar's left edge with
/// identical styling; a revealed member carries a light indent under its head, and the disclosure
/// rides at the row's right so the name column stays consistent. The name selects the memory; the
/// disclosure toggle, when present, expands or collapses the class without changing the selection.
function MemoryRow({
  memory,
  namespace,
  active,
  recurring,
  designated,
  nested,
  disclosure,
  onSelect,
}: {
  memory: MemoryView;
  namespace: string;
  active: boolean;
  recurring: boolean;
  designated?: boolean;
  nested?: boolean;
  disclosure?: { open: boolean; count: number; onToggle: () => void };
  onSelect: (name: string) => void;
}) {
  return (
    <div className={"flex items-start" + (nested ? " pl-3" : "")}>
      <button
        onClick={() => onSelect(memory.name)}
        title={memory.description ? `${memory.name} — ${memory.description}` : memory.name}
        className={
          "flex w-full min-w-0 flex-col border-l-2 py-1 pl-2.5 text-left transition-colors " +
          (active ? "border-clay text-ink" : "border-transparent text-ink-soft hover:text-ink")
        }
      >
        <span className="flex w-full min-w-0 items-baseline font-mono text-xs">
          <span className="truncate">{leafName(memory.name, namespace)}</span>
          {designated && (
            <span className="ml-1.5 shrink-0 text-clay" title="operator-designated primary">
              ◆
            </span>
          )}
          {recurring && (
            <span className="ml-1.5 shrink-0 text-sage" title="recurring">
              ↻
            </span>
          )}
        </span>
        {/* The synthesized description, clamped, so the list reads as a glanceable index of what each
            memory is about rather than a bare list of names. */}
        {memory.description && (
          <span className="mt-0.5 line-clamp-2 text-2xs/snug text-ink-faint">
            {memory.description}
          </span>
        )}
      </button>
      {disclosure && (
        <button
          onClick={disclosure.onToggle}
          aria-expanded={disclosure.open}
          title={
            disclosure.open
              ? "Collapse identity class"
              : `Expand ${disclosure.count} class member${disclosure.count === 1 ? "" : "s"}`
          }
          className="mt-1 shrink-0 px-1 font-mono text-2xs text-ink-faint transition-colors hover:text-ink"
        >
          <span aria-hidden>{disclosure.open ? "▾" : "▸"}</span>
        </button>
      )}
    </div>
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
  readOnly = false,
  highlighted,
}: {
  entry: EntryView;
  nameById: Map<string, string>;
  faded?: boolean;
  disputed?: boolean;
  /// Render the operator archaeology beside the entry: each corroborating attester with its posture,
  /// distinct phrasing, and — on a history read — its retracted attestations struck through. The
  /// compact metadata line keeps only the attester chips and the count badge without this.
  expanded?: boolean;
  /// The memory's name, so the retract button can address the entry by memory + entry id, and the
  /// entry can carry an `#entry-<id>` anchor a `?entry=` deep link scrolls to.
  memoryName?: string;
  /// Retract this entry under operator authority. Present only in the live agent frame at the head.
  onRetract?: (memory: string, entry: EntryId, reason: string) => Promise<void>;
  /// Whether the instance is booted for inspection only, so the retraction would be refused with a
  /// `409`. The control still renders, held closed; the detail pane states why once for the section
  /// rather than repeating it beside every entry.
  readOnly?: boolean;
  /// The entry-deep-link target (`?entry=<id>`): marked with a clay rule and tint so the eye lands on
  /// exactly the entry the reference named.
  highlighted?: boolean;
}) {
  const stream = useOptionalStream();
  const priv = isPrivate(entry.visibility);
  // The founding attestation is `attestations[0]` (the reads order founding first), and it is the
  // same teller the row already renders as `told by`. The tail is the corroboration — a further
  // teller standing behind the same fact — so the chips never double-render the founding teller.
  const corroborations = entry.attestations.slice(1);
  const liveCorroborations = corroborations.filter((att) => att.retracted_reason === null);
  const liveCount = entry.attestations.filter((att) => att.retracted_reason === null).length;
  return (
    <li
      id={`entry-${entry.entry_id}`}
      className={
        (faded ? "opacity-55" : "") +
        (highlighted ? " border-l-2 border-clay bg-clay-soft/15 py-1 pl-3" : "")
      }
    >
      <div className={"text-base/relaxed " + (faded ? "text-ink-soft line-through" : "text-ink")}>
        <ReactMarkdown remarkPlugins={entryMarkdownPlugins} components={turnComponents}>
          {entry.text}
        </ReactMarkdown>
      </div>
      <p className="mt-1 flex flex-wrap items-baseline gap-x-2.5 font-mono text-2xs text-ink-faint">
        {/* The entry id leads the line (faint, truncated), the same handle the agent supersedes or
            retracts by; the title carries the full id, and the id links to its own entry — the
            shareable `?entry=` deep link that highlights this entry. */}
        {stream && memoryName ? (
          <Link
            to={stream.link.state(memoryName, { entry: entry.entry_id, seq: stream.seq })}
            title={entry.entry_id}
            className="text-ink-faint/60 underline-offset-2 transition-colors hover:text-ink hover:underline"
          >
            {entry.entry_id.slice(0, 10)}
          </Link>
        ) : (
          <span className="text-ink-faint/60" title={entry.entry_id}>
            {entry.entry_id.slice(0, 10)}
          </span>
        )}
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
          <RetractButton
            memoryName={memoryName}
            entryId={entry.entry_id}
            onRetract={onRetract}
            readOnly={readOnly}
          />
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
  readOnly = false,
}: {
  memoryName: string;
  entryId: EntryId;
  onRetract: (memory: string, entry: EntryId, reason: string) => Promise<void>;
  /// Booted for inspection only: the trigger renders disabled rather than opening a reason field for
  /// a retraction the server would refuse.
  readOnly?: boolean;
}) {
  const [confirming, setConfirming] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function commit() {
    const trimmed = reason.trim();
    if (!trimmed || readOnly) return;
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
          disabled={readOnly}
          className="text-clay transition-colors hover:text-ink disabled:text-ink-faint/40 disabled:hover:text-ink-faint/40"
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
