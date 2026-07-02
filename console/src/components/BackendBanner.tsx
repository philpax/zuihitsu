import type { BackendHealth } from "../types/BackendHealth.ts";

/// The degraded-backend banner: a hairline strip under the header, shown while the agent's model
/// transport is down or struggling (`/control/health` reports an open or half-open circuit, or
/// live retries) and gone on recovery. Calm by design — the situation is already handled (inbounds
/// are recorded and deferred; the agent catches up when the model returns), so the banner informs
/// rather than alarms: clay ink on the paper ground, no fill, no toast.
export function BackendBanner({ health }: { health: BackendHealth }) {
  return (
    <div
      role="status"
      className="flex items-baseline gap-3 border-b border-line py-2 font-mono text-2xs"
    >
      <span className="shrink-0 text-clay">● {stateLine(health)}</span>
      {health.last_failure && (
        <span className="min-w-0 truncate text-ink-faint" title={health.last_failure}>
          {health.last_failure}
        </span>
      )}
      <span className="ml-auto hidden shrink-0 text-ink-faint sm:inline">
        messages are kept — the agent catches up when it returns
      </span>
    </div>
  );
}

/// The one-line reading of the circuit: open (failing fast, with the time to the next probe),
/// half-open (the probe is in flight), or closed-but-failing (the retry loop is riding a blip).
function stateLine(health: BackendHealth): string {
  switch (health.circuit) {
    case "open": {
      if (health.open_remaining_ms === null) return "the agent's model is unreachable";
      const seconds = Math.max(1, Math.round(Number(health.open_remaining_ms) / 1000));
      return `the agent's model is unreachable — probing again in ~${seconds}s`;
    }
    case "half_open":
      return "checking whether the agent's model has returned…";
    case "closed":
      return "the agent's model is struggling — retrying";
  }
}
