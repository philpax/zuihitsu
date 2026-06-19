import { useEffect, useRef, useState } from "react";

import type { LiveConnection, LiveStatus } from "../lib/live.ts";
import { useLiveLog } from "../lib/live.ts";
import { useStreamLocation } from "../lib/useStreamLocation.ts";
import { type GenesisStatus, genesisStatus } from "../lib/operator.ts";
import { Dot, Eyebrow } from "./primitives.tsx";
import { StreamWorkspace } from "./StreamWorkspace.tsx";
import { GenesisGate } from "./GenesisGate.tsx";
import { LuaConsole } from "./LuaConsole.tsx";
import { SettingsView } from "./SettingsView.tsx";
import { PromptsView } from "./PromptsView.tsx";

/// The agent frame: tail a running agent's event log and drive the shared stream views off it,
/// exactly as the eval frame drives them off a run's embedded log. The differences are all the agent
/// frame's: the source is a live `/control/events` tail (so the timeline head grows as events
/// arrive), the Conversation view is interactive (you converse and act with operator authority), and
/// an agentless instance is gated behind genesis until it is born.
export function LiveShell({
  connection,
  base = "/live",
  onClose,
}: {
  connection: LiveConnection;
  /// The route the views live under — `/live` in the full console, or the root (`""`) in the embedded
  /// build, where the agent view is the whole app.
  base?: string;
  /// Disconnect and return to the landing. Absent in the embedded build, where there is nowhere to
  /// return to, so the close affordance is hidden.
  onClose?: () => void;
}) {
  // Mirrors the cursor's follow state for the poll loop, which folds a new batch to the head only
  // while followed — a ref so the long-lived interval reads the current value without re-subscribing.
  const following = useRef(true);
  const log = useLiveLog(connection, following);
  // The active view and timeline cursor live in the URL, exactly as in the eval frame.
  const { view, seq, selectView, setSeq } = useStreamLocation(base);
  // The handle you converse under as a participant, lifted here so it survives view switches.
  const [sender, setSender] = useState("");
  const [genesis, setGenesis] = useState<GenesisStatus | "loading" | "unreachable">("loading");

  useEffect(() => {
    let cancelled = false;
    genesisStatus(connection).then(
      (value) => !cancelled && setGenesis(value),
      () => !cancelled && setGenesis("unreachable"),
    );
    return () => {
      cancelled = true;
    };
  }, [connection]);

  return (
    <div className="mx-auto flex min-h-screen max-w-[76rem] flex-col px-4 sm:px-8">
      <header className="border-b border-line py-4 sm:py-6">
        <div className="flex items-baseline justify-between gap-3">
          <div className="flex items-baseline gap-3">
            <span className="font-serif text-xl text-ink">zuihitsu</span>
            <Eyebrow>console · agent</Eyebrow>
          </div>
          <div className="flex items-baseline gap-3 font-mono text-xs text-ink-soft">
            <span className="hidden items-baseline gap-3 sm:flex">
              <ConnectionBadge status={log.status} />
              <Dot />
              <span>{log.head} events</span>
            </span>
            {onClose && (
              <button
                onClick={onClose}
                className="ml-1 shrink-0 text-ink-faint transition-colors hover:text-clay"
                title="Disconnect"
              >
                ✕
              </button>
            )}
          </div>
        </div>

        {/* On mobile the connection status drops to a quieter second row. */}
        <div className="mt-2 flex items-baseline gap-3 font-mono text-xs text-ink-soft sm:hidden">
          <ConnectionBadge status={log.status} />
          <Dot />
          <span>{log.head} events</span>
        </div>
      </header>

      {!log.replica ? (
        <Pending status={log.status} />
      ) : genesis === "loading" ? (
        <Pending status={log.status} />
      ) : genesis === "Empty" || genesis === "Incomplete" ? (
        <GenesisGate
          connection={connection}
          resuming={genesis === "Incomplete"}
          onCreated={() => setGenesis("Complete")}
        />
      ) : (
        <StreamWorkspace
          replica={log.replica}
          events={log.events}
          head={log.head}
          view={view ?? "conversation"}
          onSelectView={selectView}
          seq={seq}
          onSeq={setSeq}
          onFollowingChange={(value) => {
            following.current = value;
          }}
          participant={{ connection, sender, setSender }}
          extraViews={[
            { id: "console", label: "Console", node: <LuaConsole connection={connection} /> },
            {
              id: "prompts",
              label: "Prompts",
              node: <PromptsView connection={connection} events={log.events} />,
            },
            { id: "settings", label: "Settings", node: <SettingsView connection={connection} /> },
          ]}
        />
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
      <div className="py-16 text-center text-sm text-clay">
        Could not reach the agent — {status.message}
      </div>
    );
  }
  return <div className="py-16 text-center text-sm text-ink-faint">Connecting to the agent…</div>;
}
