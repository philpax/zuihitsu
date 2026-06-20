import { useState } from "react";

import type { RunRecord } from "../types/RunRecord.ts";
import type { Verdict } from "../types/Verdict.ts";
import { formatMs, formatTokenSplit } from "../lib/format.ts";
import { Eyebrow } from "./primitives.tsx";

/// The run's verdicts and cost, sitting above the deep views so opening a run answers "did it pass,
/// and why" before anything else. The summary line carries the gate's outcome and the run's metrics;
/// expanding it lists every oracle and metric the judge scored — its rationale, and the verbatim
/// judge response behind a judged (rather than deterministically checked) criterion. Open by default
/// when something failed, since that is the moment this panel exists for.
export function VerdictPanel({ run, gating }: { run: RunRecord; gating: boolean }) {
  const failed = run.verdicts.filter((verdict) => !verdict.passed);
  const [open, setOpen] = useState(failed.length > 0);
  const { metrics } = run;
  const total = run.verdicts.length;
  const passed = total - failed.length;
  // A gating eval's badge is the gate's outcome; a metric eval gates nothing, so its badge is the
  // run's criteria tally instead (the gate would always read "held" and mislead).
  const clean = gating ? metrics.gating_passed : failed.length === 0;
  const badge = gating
    ? metrics.gating_passed
      ? "gating · held"
      : "gating · regressed"
    : `metric · ${passed}/${total} passed`;

  return (
    <section className="border-b border-line py-3">
      <button
        onClick={() => setOpen(!open)}
        className="flex w-full flex-wrap items-baseline gap-x-4 gap-y-1 py-1 text-left"
      >
        <span className="flex items-baseline gap-2">
          <span className="font-mono text-2xs text-ink-faint">{open ? "▾" : "▸"}</span>
          <span
            className={
              "font-mono text-2xs uppercase tracking-widest " + (clean ? "text-sage" : "text-clay")
            }
          >
            {badge}
          </span>
        </span>
        {gating && total > 0 && (
          <span className="font-mono text-2xs text-ink-soft">
            {passed}/{total} criteria passed
          </span>
        )}
        <span className="ml-auto flex items-baseline gap-3 font-mono text-2xs text-ink-faint">
          <span>{metrics.model_calls} calls</span>
          <span>·</span>
          <span>{formatMs(metrics.wall_clock_ms)}</span>
          <span>·</span>
          <span>{formatTokenSplit(metrics.prompt_tokens, metrics.completion_tokens)}</span>
        </span>
      </button>

      {open &&
        (run.verdicts.length === 0 ? (
          <p className="mt-2 font-mono text-2xs text-ink-faint">
            No criteria recorded for this run.
          </p>
        ) : (
          <ul className="mt-2 flex flex-col gap-2.5">
            {run.verdicts.map((verdict, index) => (
              <VerdictRow key={index} verdict={verdict} />
            ))}
          </ul>
        ))}
    </section>
  );
}

function VerdictRow({ verdict }: { verdict: Verdict }) {
  const [showRaw, setShowRaw] = useState(false);
  return (
    <li className="border-l-2 border-line pl-3">
      <div className="flex flex-wrap items-baseline gap-x-2.5 gap-y-0.5">
        <span className={"font-mono text-2xs " + (verdict.passed ? "text-sage" : "text-clay")}>
          {verdict.passed ? "✓" : "✗"}
        </span>
        <span className="text-sm text-ink">{verdict.criterion}</span>
        <Eyebrow>{verdict.kind}</Eyebrow>
      </div>
      {verdict.rationale && (
        <p className="mt-1 max-w-prose text-sm leading-relaxed text-ink-soft">
          {verdict.rationale}
        </p>
      )}
      {verdict.judge_raw && (
        <div className="mt-1">
          <button
            onClick={() => setShowRaw(!showRaw)}
            className="font-mono text-2xs text-ink-faint transition-colors hover:text-ink-soft"
          >
            {showRaw ? "▾" : "▸"} judge response
          </button>
          {showRaw && (
            <pre className="mt-1 max-h-72 overflow-auto whitespace-pre-wrap border-l border-line bg-oat/40 px-3 py-2 font-mono text-2xs leading-relaxed text-ink-soft">
              {verdict.judge_raw}
            </pre>
          )}
        </div>
      )}
    </li>
  );
}
