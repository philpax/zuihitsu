import { Link } from "react-router-dom";

import { useConsoleNav } from "../lib/consoleNav.ts";

/// The cross-section nav in a frame's header: pivot between the package's scenarios and the metrics
/// trends. The section you are in is marked; a loaded sibling is a link; a sibling not yet loaded is a
/// file picker, so you can bring it in without a trip back to the landing.
export function FrameNav({ current }: { current: "eval" | "trends" }) {
  const nav = useConsoleNav();
  return (
    <nav className="flex items-baseline gap-4 font-mono text-2xs">
      <Section
        label="scenarios"
        active={current === "eval"}
        loaded={nav.hasPackage}
        to="/eval"
        accept="application/json,.json"
        onLoad={nav.openPackage}
      />
      <Section
        label="trends"
        active={current === "trends"}
        loaded={nav.hasHistory}
        to="/trends"
        accept=".jsonl,application/json"
        onLoad={nav.openHistory}
      />
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
    return <span className="border-b-2 border-clay pb-1 text-ink">{label}</span>;
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
