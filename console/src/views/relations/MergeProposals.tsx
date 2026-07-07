import { useState } from "react";

import type { MemoryId } from "../../types/MemoryId.ts";
import type { MergeProposalView, MergeStatus } from "../../lib/model/graph.ts";
import { statePath } from "../../lib/nav/routes.ts";
import { Button, Eyebrow, Hint } from "../../components/primitives.tsx";

/// The operator's merge-decision surface: every cross-platform merge proposal the folded log holds,
/// each with the proposer's stated grounds, its two stubs (linked into State), and where it now stands
/// — pending, merged, or rejected. When `onResolve` is supplied (the live agent frame at the head),
/// a still-pending proposal carries approve/decline buttons that author the operator's call; in the
/// read-only eval viewer, or scrubbed back in time, the proposals render as a record without actions.
///
/// Derived from the log rather than fetched, so the eval viewer and the live console show the same
/// picture, and a resolution folds back through the same materializer that produced the list.
export function MergeProposals({
  proposals,
  base,
  cursor,
  navigate,
  onResolve,
}: {
  proposals: MergeProposalView[];
  base: string;
  cursor: number;
  navigate: (path: string) => void;
  onResolve?: (from: MemoryId, to: MemoryId, accept: boolean) => Promise<void>;
}) {
  // The pair currently being resolved (keyed by its two ids), and the last failure, so the buttons
  // disable in flight and a rejected request surfaces its reason.
  const [busy, setBusy] = useState<string | null>(null);
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

  return (
    <section className="flex flex-col gap-2">
      <Eyebrow>{`identity merges · ${proposals.length}`}</Eyebrow>
      <ul className="flex flex-col divide-y divide-line border-y border-line">
        {proposals.map((proposal) => {
          const key = `${proposal.from_id}:${proposal.to_id}`;
          const pending = proposal.status === "pending";
          return (
            <li key={key} className="flex flex-col gap-1.5 py-3">
              <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
                <StubLink name={proposal.from} base={base} cursor={cursor} navigate={navigate} />
                <span className="font-mono text-2xs text-ink-faint">same&nbsp;as</span>
                <StubLink name={proposal.to} base={base} cursor={cursor} navigate={navigate} />
                <StatusBadge status={proposal.status} />
                <span className="font-mono text-2xs text-ink-faint">
                  {proposal.source === "Orchestration" ? "handle match" : "proposed by the agent"}
                </span>
              </div>

              {proposal.rationale && (
                <p className="border-l border-line pl-3 text-xs leading-relaxed text-ink-soft">
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
            </li>
          );
        })}
      </ul>
      {error && <Hint tone="error">{error}</Hint>}
    </section>
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
      className={`rounded-xs border px-1.5 py-0.5 font-mono text-2xs uppercase tracking-wider ${tone}`}
    >
      {status}
    </span>
  );
}

/// A stub's handle, linked into the State view at the cursor so the operator can inspect what each
/// side actually holds before deciding.
function StubLink({
  name,
  base,
  cursor,
  navigate,
}: {
  name: string;
  base: string;
  cursor: number;
  navigate: (path: string) => void;
}) {
  return (
    <button
      onClick={() => navigate(statePath(base, cursor, name))}
      title={`Open ${name} in State`}
      className="font-mono text-xs text-clay underline-offset-2 transition-colors hover:text-ink hover:underline"
    >
      {name}
    </button>
  );
}
