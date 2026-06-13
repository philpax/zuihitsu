import { VIEWS, type ViewId } from "../lib/views.ts";

/// The view tabs. Package-scoped views are always reachable; run-scoped views light up only once a
/// run is selected. Unbuilt views stand faint, naming what is still to come.
export function Nav({
  view,
  onSelect,
  runActive,
}: {
  view: ViewId;
  onSelect: (view: ViewId) => void;
  runActive: boolean;
}) {
  return (
    <nav className="flex gap-7 border-b border-line text-sm">
      {VIEWS.map((entry) => {
        const enabled = entry.ready && (entry.scope === "package" || runActive);
        const active = entry.id === view;
        const title = !entry.ready
          ? "Coming soon"
          : entry.scope === "run" && !runActive
            ? "Select a run first"
            : undefined;
        return (
          <button
            key={entry.id}
            disabled={!enabled}
            onClick={() => enabled && onSelect(entry.id)}
            title={title}
            className={
              "-mb-px border-b-2 py-3 transition-colors " +
              (active
                ? "border-clay text-ink"
                : enabled
                  ? "border-transparent text-ink-soft hover:text-ink"
                  : "cursor-default border-transparent text-ink-faint/55")
            }
          >
            {entry.label}
          </button>
        );
      })}
    </nav>
  );
}
