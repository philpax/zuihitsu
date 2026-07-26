import { useState } from "react";
import type { ReactNode } from "react";

import type { MemoryId } from "@zuihitsu/wire/types/MemoryId.ts";
import type { MergeProposalView } from "@zuihitsu/wire/wasm/console_wasm.js";
import type { MergeStatus } from "@zuihitsu/wire/wasm/console_wasm.js";
import { Eyebrow, Hint, WorkingPulse } from "../../components/primitives.tsx";
import { MemoryNameLink } from "../../components/eventDetailParts.tsx";
import { useReadOnly } from "../../lib/view/readOnly.ts";

/// The operator's merge-decision surface: every cross-platform merge proposal the folded log holds,
/// each with the proposer's stated grounds, its two stubs (linked into State), and where it now stands
/// — pending or merged. When `onResolve` is supplied (the live agent frame at the head), a
/// still-pending proposal carries a confirm affordance that authors the operator's merge; leaving it
/// unconfirmed keeps it pending. A merged one carries an unmerge affordance (`onUnmerge`) that
/// retracts the `same_as` — the undo of a wrong merge, splitting the two identities back apart. A
/// merged pair also marks which stub is the class's primary — the id class-level reads resolve
/// through — and, with `onDesignatePrimary`, lets the operator pin the other stub or release a pin
/// they set, overriding the earliest-ULID default. In the read-only eval viewer, or scrubbed back in
/// time, the proposals render as a record without actions.
///
/// Each pair is one calm card: the canonical primary anchors it (marked with a small ◆, clay when
/// pinned), the other stub folds in beneath, and the proposer's grounds sit as a quiet annotation
/// anchored to the pair. Actions are one register of subordinate text buttons — the destructive
/// unmerge alone carries the clay accent — so the information leads and the controls recede.
///
/// Derived from the log rather than fetched, so the eval viewer and the live console show the same
/// picture, and a resolution folds back through the same materializer that produced the list.
export function MergeProposals({
  proposals,
  onResolve,
  onUnmerge,
  onDesignatePrimary,
}: {
  proposals: MergeProposalView[];
  onResolve?: (from: MemoryId, to: MemoryId) => Promise<void>;
  onUnmerge?: (from: MemoryId, to: MemoryId) => Promise<void>;
  onDesignatePrimary?: (memory: MemoryId, designated: boolean) => Promise<void>;
}) {
  // Booted for inspection only: every decision below would be refused with a `409`, so the cards keep
  // their verbs but hold them closed, with the reason stated once at the foot of the surface.
  const readOnly = useReadOnly();
  // The pair currently being acted on (keyed by its two ids), and the last failure, so the buttons
  // disable in flight and a rejected request surfaces its reason. `confirming` holds the pair whose
  // unmerge is awaiting the second, deliberate click — retracting a merge is destructive.
  const [busy, setBusy] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  if (proposals.length === 0) return null;

  // One runner behind every action: key the pair busy, clear the last error, run, and surface any
  // rejection. Every handler funnels through here so the in-flight and failure behaviour is uniform.
  async function run(key: string, action: () => Promise<void>) {
    if (readOnly) return;
    setBusy(key);
    setError(null);
    try {
      await action();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }

  return (
    <section className="flex flex-col gap-3">
      <Eyebrow>{`identity merges · ${proposals.length}`}</Eyebrow>
      <ul className="flex flex-col gap-3">
        {proposals.map((proposal) => {
          const key = `${proposal.from_id}:${proposal.to_id}`;
          return (
            <MergeCard
              key={key}
              proposal={proposal}
              working={busy === key}
              busy={busy !== null}
              isConfirming={confirming === key}
              onResolve={
                onResolve && (() => run(key, () => onResolve(proposal.from_id, proposal.to_id)))
              }
              onUnmerge={
                onUnmerge &&
                (() => {
                  setConfirming(null);
                  return run(key, () => onUnmerge(proposal.from_id, proposal.to_id));
                })
              }
              onDesignate={
                onDesignatePrimary &&
                ((memory, designated) => run(key, () => onDesignatePrimary(memory, designated)))
              }
              onStartConfirm={() => setConfirming(key)}
              onCancelConfirm={() => setConfirming(null)}
            />
          );
        })}
      </ul>
      {error && <Hint tone="error">{error}</Hint>}
      {readOnly && (
        <Hint className="text-2xs">
          read-only — merge decisions cannot be recorded in inspection mode
        </Hint>
      )}
    </section>
  );
}

/// One proposal as a card. The identity stack sits at the top with its status mark; the proposer's
/// grounds follow as an anchored note; the pair's verbs — confirm, unmerge — close it. Per-stub
/// primary controls ride on the member rows they concern, so the primary choice reads where the
/// primary is shown rather than in a separate bank of buttons.
function MergeCard({
  proposal,
  working,
  busy,
  isConfirming,
  onResolve,
  onUnmerge,
  onDesignate,
  onStartConfirm,
  onCancelConfirm,
}: {
  proposal: MergeProposalView;
  working: boolean;
  busy: boolean;
  isConfirming: boolean;
  onResolve?: () => void;
  onUnmerge?: () => void;
  onDesignate?: (memory: MemoryId, designated: boolean) => void;
  onStartConfirm: () => void;
  onCancelConfirm: () => void;
}) {
  const readOnly = useReadOnly();
  const pending = proposal.status === "pending";
  const merged = proposal.status === "merged";
  const [lead, folded] = orderedMembers(proposal);
  // Every verb on the card is closed while another pair is in flight, and while the instance is
  // booted read-only.
  const locked = busy || readOnly;

  return (
    <li className="flex flex-col gap-3 rounded-sm border border-line bg-paper-raised p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="flex min-w-0 flex-col gap-1">
          <MemberRow
            member={lead}
            merged={merged}
            lead
            action={merged ? designateAction(lead, onDesignate, working || readOnly) : null}
          />
          <MemberRow
            member={folded}
            merged={merged}
            lead={false}
            action={merged ? designateAction(folded, onDesignate, working || readOnly) : null}
          />
        </div>
        <StatusMark status={proposal.status} />
      </div>

      <ProvenanceNote rationale={proposal.rationale} source={proposal.source} />

      {pending && onResolve && (
        <div className="flex items-center gap-3">
          <RowAction tone="affirm" disabled={locked} onClick={onResolve}>
            confirm merge
          </RowAction>
          {working && <WorkingPulse className="self-center" />}
        </div>
      )}

      {merged &&
        onUnmerge &&
        (isConfirming ? (
          <div className="flex flex-wrap items-center gap-3">
            <Hint tone="error">retract this merge? the two identities split apart.</Hint>
            <RowAction tone="destructive" disabled={locked} onClick={onUnmerge}>
              unmerge
            </RowAction>
            <RowAction disabled={busy} onClick={onCancelConfirm}>
              cancel
            </RowAction>
            {working && <WorkingPulse className="self-center" />}
          </div>
        ) : (
          <div className="flex items-center gap-3">
            <RowAction tone="destructive" disabled={locked} onClick={onStartConfirm}>
              unmerge
            </RowAction>
            {working && <WorkingPulse className="self-center" />}
          </div>
        ))}
    </li>
  );
}

/// One stub on a proposal's identity stack: its marker gutter, its name (a link into State), and any
/// primary control it carries. The leading row is the anchor — the readable identity a class shows its
/// far endpoints under; the folded row is the stub merged into it, set a notch quieter.
function MemberRow({
  member,
  merged,
  lead,
  action,
}: {
  member: Member;
  merged: boolean;
  lead: boolean;
  action: ReactNode;
}) {
  const marker = member.primary ? (
    <PrimaryMark pinned={member.designated} />
  ) : (
    <span aria-hidden className="inline-block w-3 shrink-0 text-center text-ink-faint/60">
      {merged ? "↳" : "·"}
    </span>
  );
  return (
    <div className="flex items-baseline gap-2">
      {marker}
      <span className={"min-w-0 truncate " + (lead ? "text-base" : "text-sm")} title={member.name}>
        <MemoryNameLink name={member.name} />
      </span>
      {action && <span className="ml-auto shrink-0 pl-2">{action}</span>}
    </div>
  );
}

/// The primary control a merged stub carries: a non-primary stub can be made primary; a stub the
/// operator has pinned can be released back to the earliest-ULID default. A stub that is primary by
/// default carries nothing — the way to move it is to make its partner primary instead. `locked`
/// closes the control while this pair is in flight, or while the instance is booted read-only.
function designateAction(
  member: Member,
  onDesignate: ((memory: MemoryId, designated: boolean) => void) | undefined,
  locked: boolean,
): ReactNode {
  if (!onDesignate) return null;
  if (!member.primary) {
    return (
      <RowAction disabled={locked} onClick={() => onDesignate(member.id, true)}>
        make primary
      </RowAction>
    );
  }
  if (member.designated) {
    return (
      <RowAction disabled={locked} onClick={() => onDesignate(member.id, false)}>
        release
      </RowAction>
    );
  }
  return null;
}

/// The proposer's grounds, anchored to the pair by a hairline: the rationale sentence in soft ink
/// above the provenance line in faint ink. A rationale-less proposal (an orchestration-handle match, a
/// `same_as`-via-link) still shows the provenance, so the annotation is never a floating fragment.
function ProvenanceNote({
  rationale,
  source,
}: {
  rationale: string | null;
  source: MergeProposalView["source"];
}) {
  return (
    <div className="flex flex-col gap-1 border-l border-line pl-3">
      {rationale && <p className="text-xs/relaxed text-ink-soft">{rationale}</p>}
      <p className="font-mono text-2xs text-ink-faint">{provenanceLabel(source)}</p>
    </div>
  );
}

/// Where the proposal came from, in the operator's register. Every proposal is the agent's own
/// judgment today; the mapping keeps the phrasing in one place for the day a second source appears.
function provenanceLabel(source: MergeProposalView["source"]): string {
  switch (source) {
    case "Agent":
      return "proposed by the agent";
  }
}

/// The proposal's standing as a quiet mark, not a shouting chip: a filled dot and a lowercase word —
/// sage once merged, clay while it awaits the operator's decision.
function StatusMark({ status }: { status: MergeStatus }) {
  const merged = status === "merged";
  return (
    <span className="flex shrink-0 items-center gap-1.5 font-mono text-2xs tracking-wide text-ink-faint">
      <span className={"size-1.5 rounded-full " + (merged ? "bg-sage" : "bg-clay")} aria-hidden />
      {status}
    </span>
  );
}

/// The mark on the stub a class resolves through — the primary. A pinned primary
/// (`ClassPrimaryDesignated`) is the operator's explicit choice, drawn in clay; one that won by the
/// earliest-ULID default is drawn in faint ink, so the deliberate pin reads as the stronger mark. The
/// title carries the distinction for a reader who hovers.
function PrimaryMark({ pinned }: { pinned: boolean }) {
  return (
    <span
      aria-hidden
      title={pinned ? "Primary, pinned by you" : "Primary, by default (earliest id)"}
      className={
        "inline-block w-3 shrink-0 text-center " + (pinned ? "text-clay" : "text-ink-faint")
      }
    >
      ◆
    </span>
  );
}

/// One text button, the surface's single action register: quiet by default, ink for the affirmative
/// verb, clay for the destructive one. Underlines on hover so it reads as a control without a box.
function RowAction({
  tone = "default",
  disabled,
  onClick,
  children,
}: {
  tone?: "default" | "affirm" | "destructive";
  disabled?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  const tones = {
    default: "text-ink-soft enabled:hover:text-ink",
    affirm: "text-ink enabled:hover:text-clay",
    destructive: "text-clay",
  } as const;
  return (
    <button
      disabled={disabled}
      onClick={onClick}
      className={
        "font-mono text-xs underline-offset-2 transition-colors enabled:hover:underline disabled:opacity-40 " +
        tones[tone]
      }
    >
      {children}
    </button>
  );
}

/// A stub on a proposal, flattened from the pair's two-sided shape so the card can order and render the
/// two symmetrically.
interface Member {
  name: string;
  id: MemoryId;
  primary: boolean;
  designated: boolean;
}

/// Order a pair's two stubs for display: the canonical primary leads as the anchor; failing a primary
/// (a merged class whose primary is a third, older member), the readable name leads so a raw
/// platform-qualified stub id never takes the anchor line.
function orderedMembers(proposal: MergeProposalView): [Member, Member] {
  const a: Member = {
    name: proposal.from,
    id: proposal.from_id,
    primary: proposal.from_primary,
    designated: proposal.from_designated,
  };
  const b: Member = {
    name: proposal.to,
    id: proposal.to_id,
    primary: proposal.to_primary,
    designated: proposal.to_designated,
  };
  const rank = (m: Member) => (m.primary ? 0 : m.name.includes("@") ? 2 : 1);
  return rank(a) <= rank(b) ? [a, b] : [b, a];
}
