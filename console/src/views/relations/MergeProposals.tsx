import { useState } from "react";

import type { MemoryId } from "@zuihitsu/wire/types/MemoryId.ts";
import type { MergeProposalView, MergeStatus } from "../../lib/model/graph.ts";
import { Button, Eyebrow, Hint } from "../../components/primitives.tsx";
import { MemoryNameLink } from "../../components/eventDetailParts.tsx";

/// The operator's merge-decision surface: every cross-platform merge proposal the folded log holds,
/// each with the proposer's stated grounds, its two stubs (linked into State), and where it now stands
/// — pending, merged, or rejected. When `onResolve` is supplied (the live agent frame at the head),
/// a still-pending proposal carries approve/decline buttons that author the operator's call; a merged
/// one carries an unmerge affordance (`onUnmerge`) that retracts the `same_as` — the undo of a wrong
/// merge, splitting the two identities back apart. A merged pair also marks which stub is the class's
/// primary — the id class-level reads resolve through — and, with `onDesignatePrimary`, lets the
/// operator pin the other stub or release a pin they set, overriding the earliest-ULID default. In the
/// read-only eval viewer, or scrubbed back in time, the proposals render as a record without actions.
///
/// Derived from the log rather than fetched, so the eval viewer and the live console show the same
/// picture, and a resolution folds back through the same materializer that produced the list.
export function MergeProposals({
  proposals,
  cursor,
  onResolve,
  onUnmerge,
  onDesignatePrimary,
}: {
  proposals: MergeProposalView[];
  cursor: number;
  onResolve?: (from: MemoryId, to: MemoryId, accept: boolean) => Promise<void>;
  onUnmerge?: (from: MemoryId, to: MemoryId) => Promise<void>;
  onDesignatePrimary?: (memory: MemoryId, designated: boolean) => Promise<void>;
}) {
  // The pair currently being resolved (keyed by its two ids), and the last failure, so the buttons
  // disable in flight and a rejected request surfaces its reason. `confirming` holds the pair whose
  // unmerge is awaiting the second, deliberate click — retracting a merge is destructive.
  const [busy, setBusy] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  if (proposals.length === 0) return null;

  async function resolve(proposal: MergeProposalView, accept: boolean) {
    if (!onResolve) return;
    const key = `${proposal.from_id}:${proposal.to_id}`;
    setBusy(key);
    setError(null);
    try {
      await onResolve(proposal.from_id, proposal.to_id, accept);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }

  async function unmerge(proposal: MergeProposalView) {
    if (!onUnmerge) return;
    const key = `${proposal.from_id}:${proposal.to_id}`;
    setBusy(key);
    setConfirming(null);
    setError(null);
    try {
      await onUnmerge(proposal.from_id, proposal.to_id);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }

  async function designate(proposal: MergeProposalView, memory: MemoryId, designated: boolean) {
    if (!onDesignatePrimary) return;
    const key = `${proposal.from_id}:${proposal.to_id}`;
    setBusy(key);
    setError(null);
    try {
      await onDesignatePrimary(memory, designated);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }

  return (
    <section className="flex flex-col gap-2">
      <Eyebrow>{`identity merges · ${proposals.length}`}</Eyebrow>
      <ul className="flex flex-col divide-y divide-line border-y border-line">
        {proposals.map((proposal) => {
          const key = `${proposal.from_id}:${proposal.to_id}`;
          const pending = proposal.status === "pending";
          const merged = proposal.status === "merged";
          return (
            <li key={key} className="flex flex-col gap-1.5 py-3">
              <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
                <MemoryNameLink name={proposal.from} seq={cursor} />
                {proposal.from_primary && <PrimaryBadge pinned={proposal.from_designated} />}
                <span className="font-mono text-2xs text-ink-faint">same&nbsp;as</span>
                <MemoryNameLink name={proposal.to} seq={cursor} />
                {proposal.to_primary && <PrimaryBadge pinned={proposal.to_designated} />}
                <StatusBadge status={proposal.status} />
                <span className="font-mono text-2xs text-ink-faint">proposed by the agent</span>
              </div>

              {proposal.rationale && (
                <p className="border-l border-line pl-3 text-xs/relaxed text-ink-soft">
                  {proposal.rationale}
                </p>
              )}

              {pending && onResolve && (
                <div className="mt-0.5 flex items-center gap-2">
                  <Button primary disabled={busy !== null} onClick={() => resolve(proposal, true)}>
                    approve merge
                  </Button>
                  <Button disabled={busy !== null} onClick={() => resolve(proposal, false)}>
                    decline
                  </Button>
                  {busy === key && <Hint>working…</Hint>}
                </div>
              )}

              {merged &&
                onUnmerge &&
                (confirming === key ? (
                  <div className="mt-0.5 flex items-center gap-2">
                    <Hint tone="error">retract this merge? the two identities split apart.</Hint>
                    <Button disabled={busy !== null} onClick={() => unmerge(proposal)}>
                      unmerge
                    </Button>
                    <Button disabled={busy !== null} onClick={() => setConfirming(null)}>
                      cancel
                    </Button>
                    {busy === key && <Hint>working…</Hint>}
                  </div>
                ) : (
                  <div className="mt-0.5 flex items-center gap-2">
                    <Button disabled={busy !== null} onClick={() => setConfirming(key)}>
                      unmerge
                    </Button>
                    {busy === key && <Hint>working…</Hint>}
                  </div>
                ))}

              {merged && onDesignatePrimary && (
                <div className="mt-0.5 flex flex-wrap items-center gap-2">
                  {!proposal.from_primary && (
                    <Button
                      disabled={busy !== null}
                      onClick={() => designate(proposal, proposal.from_id, true)}
                    >
                      make {proposal.from} primary
                    </Button>
                  )}
                  {!proposal.to_primary && (
                    <Button
                      disabled={busy !== null}
                      onClick={() => designate(proposal, proposal.to_id, true)}
                    >
                      make {proposal.to} primary
                    </Button>
                  )}
                  {/* A pinned primary can be released back to the earliest-ULID default. */}
                  {proposal.from_primary && proposal.from_designated && (
                    <Button
                      disabled={busy !== null}
                      onClick={() => designate(proposal, proposal.from_id, false)}
                    >
                      release {proposal.from}
                    </Button>
                  )}
                  {proposal.to_primary && proposal.to_designated && (
                    <Button
                      disabled={busy !== null}
                      onClick={() => designate(proposal, proposal.to_id, false)}
                    >
                      release {proposal.to}
                    </Button>
                  )}
                </div>
              )}
            </li>
          );
        })}
      </ul>
      {error && <Hint tone="error">{error}</Hint>}
    </section>
  );
}

/// A marker on the stub a class resolves through — the primary. A `pinned` primary is the operator's
/// explicit choice (`ClassPrimaryDesignated`); an unpinned one won by the earliest-ULID default, shown
/// in fainter ink so the deliberate pin reads as the stronger mark.
function PrimaryBadge({ pinned }: { pinned: boolean }) {
  const tone = pinned ? "border-clay/50 text-clay" : "border-line text-ink-faint";
  return (
    <span
      className={`rounded-xs border px-1.5 py-0.5 font-mono text-2xs tracking-wider uppercase ${tone}`}
    >
      {pinned ? "primary · pinned" : "primary"}
    </span>
  );
}

/// A merge proposal's resolution state as a hairline chip — clay while it awaits a decision, sage once
/// merged, muted once refused.
function StatusBadge({ status }: { status: MergeStatus }) {
  const tone =
    status === "pending"
      ? "border-clay/50 text-clay"
      : status === "merged"
        ? "border-sage/50 text-sage"
        : "border-line text-ink-faint";
  return (
    <span
      className={`rounded-xs border px-1.5 py-0.5 font-mono text-2xs tracking-wider uppercase ${tone}`}
    >
      {status}
    </span>
  );
}
