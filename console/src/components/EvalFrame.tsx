import { Link, Outlet, useMatch } from "react-router-dom";

import type { EvalPackage } from "../types/EvalPackage.ts";
import { formatDate } from "../lib/format.ts";
import { Dot } from "./primitives.tsx";
import { FrameNav } from "./FrameNav.tsx";

/// The eval frame: a loaded package of many runs. The header is shared by every nested route — the
/// Scenarios overview at the index, and a single run's deep views below it — carrying the package's
/// file name, model, commit, and date, with a breadcrumb back to the overview that shows while a run
/// is open.
/// The package itself flows to the nested routes as the outlet context, so a run route can resolve
/// its scenario and run from the URL against it.
export function EvalFrame({
  pkg,
  fileName,
  onClose,
}: {
  pkg: EvalPackage;
  fileName: string | null;
  onClose: () => void;
}) {
  // The active run, if any, drives the breadcrumb. `:run` is the run index; `:scenario` its name.
  const runMatch = useMatch("/eval/:scenario/:run/:view");
  const crumb = runMatch
    ? { scenario: runMatch.params.scenario ?? "", run: runMatch.params.run ?? "" }
    : null;

  return (
    <div className="mx-auto flex min-h-screen max-w-[76rem] flex-col px-4 sm:px-8">
      <header className="border-b border-line py-4 sm:py-6">
        <div className="flex items-baseline justify-between gap-3">
          <div className="flex min-w-0 items-baseline gap-3">
            <span className="font-serif text-xl text-ink">zuihitsu</span>
            <FrameNav current="eval" />
            {crumb && (
              <Link
                to="/eval"
                className="ml-1 hidden min-w-0 items-baseline gap-2 font-mono text-xs text-ink-soft transition-colors hover:text-clay sm:flex"
                title="Back to the package"
              >
                <span className="text-ink-faint">/</span>
                <span className="truncate">{crumb.scenario}</span>
                <span className="shrink-0 text-ink-faint">· run {crumb.run}</span>
              </Link>
            )}
          </div>
          <div className="flex items-baseline gap-3 font-mono text-xs text-ink-soft">
            <span className="hidden items-baseline gap-3 sm:flex">
              {fileName && (
                <>
                  <span className="max-w-[14rem] truncate text-ink" title={fileName}>
                    {fileName}
                  </span>
                  <Dot />
                </>
              )}
              <span className="max-w-[16rem] truncate">{pkg.meta.model_id}</span>
              <Dot />
              {pkg.meta.git_sha && (
                <>
                  <span>{pkg.meta.git_sha.slice(0, 7)}</span>
                  <Dot />
                </>
              )}
              <span>{formatDate(pkg.meta.finished_at_ms)}</span>
            </span>
            <button
              onClick={onClose}
              className="ml-1 shrink-0 text-ink-faint transition-colors hover:text-clay"
              title="Close this package"
            >
              ✕
            </button>
          </div>
        </div>

        {/* On mobile the run breadcrumb and model drop to a quieter second row. */}
        <div className="mt-2 flex items-baseline justify-between gap-3 font-mono text-2xs text-ink-soft sm:hidden">
          {crumb ? (
            <Link
              to="/eval"
              className="flex min-w-0 items-baseline gap-2 transition-colors hover:text-clay"
            >
              <span className="text-ink-faint">/</span>
              <span className="truncate">{crumb.scenario}</span>
              <span className="shrink-0 text-ink-faint">· run {crumb.run}</span>
            </Link>
          ) : (
            <span />
          )}
          <span className="shrink-0 truncate text-ink-faint">{fileName ?? pkg.meta.model_id}</span>
        </div>
      </header>

      <Outlet context={pkg} />
    </div>
  );
}
