import { useState } from "react";

import type { Replica } from "../../lib/replica/replica.ts";
import type { SessionModel } from "../../lib/model/conversation.ts";
import type { BriefTrace } from "../../lib/model/brief.ts";
import { Disclosure, Excerpt } from "../../components/primitives.tsx";
import { BriefSections } from "./BriefTrace.tsx";

/// The brief the agent saw, frozen at the session's open: the literal text (`session.brief`, captured
/// on `SessionStarted`) directly, and — one level deeper, behind its own toggle — the composer's trace
/// (which memories it weighed, and why each entry was surfaced, trimmed, or filtered). The trace is
/// gated because evaluating it re-folds the replica to the session's seq, so it reflects the frozen
/// point rather than the cursor; that re-fold is paid only when asked for, and cached once.
export function BriefBlock({
  replica,
  session,
  contextMemory,
}: {
  replica: Replica;
  session: SessionModel;
  contextMemory: string | null;
}) {
  const [open, setOpen] = useState(false);
  const [traceOpen, setTraceOpen] = useState(false);
  const [trace, setTrace] = useState<BriefTrace | null>(null);

  function toggleTrace() {
    // Compose the trace at the session's open seq — re-fold there, read, restore the fold, all
    // synchronously in this handler so the rest of the view never observes the moved fold.
    if (trace === null) {
      const restore = replica.foldedSeq;
      replica.foldTo(session.seq);
      setTrace(
        replica.brief(
          session.participantIds,
          contextMemory,
          session.startedAt,
          session.workingSet ?? [],
        ),
      );
      replica.foldTo(restore);
    }
    setTraceOpen(!traceOpen);
  }

  return (
    <div className="mb-2 border-b border-line pb-6">
      <Disclosure
        open={open}
        onToggle={() => setOpen(!open)}
        label="brief"
        summary={session.participants.join(", ") || "no one present"}
      />
      {open && (
        <>
          <Excerpt className="mt-3 max-h-96">{session.brief}</Excerpt>
          <Disclosure
            open={traceOpen}
            onToggle={toggleTrace}
            label="composition trace"
            summary="· re-folds the replica to evaluate"
            className="mt-3"
          />
          {traceOpen && session.workingSet === null && (
            <p className="mt-2 text-2xs text-ink-faint">
              The working set is unavailable for this session (recorded before capture); the trace
              may omit active-thread memories.
            </p>
          )}
          {traceOpen && trace && <BriefSections sections={trace.sections} />}
        </>
      )}
    </div>
  );
}
