/// The read-only banner: a hairline strip under the header, shown when the server is booted
/// `--read-only` against an at-rest instance's data. Calm by design — the situation is intentional
/// (the operator chose inspection mode), so the banner informs rather than alarms: clay ink on the
/// paper ground, no fill, no toast. Mutating actions are refused with `409`; data is a snapshot at
/// boot, refreshed only by restarting.
export function ReadOnlyBanner() {
  return (
    <div
      role="status"
      className="flex items-baseline gap-3 border-b border-line py-2 font-mono text-2xs"
    >
      <span className="shrink-0 text-clay">● the agent is booted read-only — inspection mode</span>
      <span className="ml-auto hidden shrink-0 text-ink-faint sm:inline">
        mutating actions are refused — data is a snapshot at boot
      </span>
    </div>
  );
}
