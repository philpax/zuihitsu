import { useRef } from "react";

import type { LiveConnection, LiveStatus } from "../lib/live.ts";
import { useLiveLog } from "../lib/live.ts";
import { Dot, Eyebrow } from "./primitives.tsx";
import { StreamWorkspace } from "./StreamWorkspace.tsx";

/// The agent frame: tail a running agent's event log and drive the shared stream views off it,
/// exactly as the eval frame drives them off a run's embedded log. The only differences are the
/// source (a live `/control/events` tail rather than a loaded file) and that the timeline's head
/// grows as events arrive — following it keeps the views on the latest state, while scrubbing back
/// pins them as new events keep extending the timeline.
export function LiveShell({
  connection,
  onClose,
}: {
  connection: LiveConnection;
  onClose: () => void;
}) {
  // Mirrors the cursor's follow state for the poll loop, which folds a new batch to the head only
  // while followed — a ref so the long-lived interval reads the current value without re-subscribing.
  const following = useRef(true);
  const log = useLiveLog(connection, following);

  return (
    <div className="mx-auto flex min-h-screen max-w-[76rem] flex-col px-8">
      <header className="flex items-baseline justify-between border-b border-line py-6">
        <div className="flex items-baseline gap-3">
          <span className="font-serif text-xl text-ink">zuihitsu</span>
          <Eyebrow>console · agent</Eyebrow>
        </div>
        <div className="flex items-baseline gap-3 font-mono text-xs text-ink-soft">
          <ConnectionBadge status={log.status} />
          <Dot />
          <span>{log.head} events</span>
          <button
            onClick={onClose}
            className="ml-1 text-ink-faint transition-colors hover:text-clay"
            title="Disconnect"
          >
            ✕
          </button>
        </div>
      </header>

      {log.replica ? (
        <StreamWorkspace
          replica={log.replica}
          events={log.events}
          head={log.head}
          onFollowingChange={(value) => {
            following.current = value;
          }}
        />
      ) : (
        <Pending status={log.status} />
      )}
    </div>
  );
}

function ConnectionBadge({ status }: { status: LiveStatus }) {
  if (status.status === "error") {
    return (
      <span className="text-clay" title={status.message}>
        ● disconnected
      </span>
    );
  }
  if (status.status === "connecting") {
    return <span className="text-ink-faint">○ connecting</span>;
  }
  return <span className="text-sage">● live</span>;
}

function Pending({ status }: { status: LiveStatus }) {
  if (status.status === "error") {
    return (
      <div className="py-24 text-center text-sm text-clay">
        Could not reach the agent — {status.message}
      </div>
    );
  }
  return <div className="py-24 text-center text-sm text-ink-faint">Connecting to the agent…</div>;
}
