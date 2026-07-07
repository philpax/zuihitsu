import { Link } from "react-router-dom";

import { useConsoleNav } from "../lib/nav/consoleNav.ts";

/// The cross-section nav in a frame's header. The section you are in is marked; a loaded sibling is
/// a link; a sibling not yet loaded is a file picker, so you can bring it in without a trip back to
/// the landing. The eval frame shows only its own section — trends is reached from the landing, so
/// the scenarios view is not cluttered with a picker for it — while the trends screen keeps the
/// scenarios pivot for the way back.
export function FrameNav({ current }: { current: "eval" | "trends" }) {
  const nav = useConsoleNav();
  return (
    <nav className="flex shrink-0 items-baseline gap-4 font-mono text-2xs">
      <Section
        label="scenarios"
        active={current === "eval"}
        loaded={nav.hasPackage}
        to="/eval"
        accept="application/json,.json"
        onLoad={nav.openPackage}
      />
      {current === "trends" && (
        <Section
          label="trends"
          active
          loaded={nav.hasHistory}
          to="/trends"
          accept=".jsonl,application/json"
          onLoad={nav.openHistory}
        />
      )}
    </nav>
  );
}

function Section({
  label,
  active,
  loaded,
  to,
  accept,
  onLoad,
}: {
  label: string;
  active: boolean;
  loaded: boolean;
  to: string;
  accept: string;
  onLoad: (file: File) => void;
}) {
  if (active) {
    // A link to the section's root even while marked current, so it returns from a sub-page (a run)
    // to the section's index (the overview) — a no-op when already there.
    return (
      <Link to={to} className="border-b-2 border-clay pb-1 text-ink">
        {label}
      </Link>
    );
  }
  if (loaded) {
    return (
      <Link
        to={to}
        className="border-b-2 border-transparent pb-1 text-ink-soft transition-colors hover:text-clay"
      >
        {label}
      </Link>
    );
  }
  return (
    <label
      className="cursor-pointer border-b-2 border-transparent pb-1 text-ink-faint transition-colors hover:text-clay"
      title={`Open a ${label === "trends" ? "history" : "package"} file`}
    >
      {label} +
      <input
        type="file"
        accept={accept}
        className="hidden"
        onChange={(event) => {
          const file = event.target.files?.[0];
          if (file) onLoad(file);
        }}
      />
    </label>
  );
}
