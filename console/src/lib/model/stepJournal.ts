/// Derived state and formatting for the eval step journal — the recorded run's scenario↔log
/// correspondence (`crates/eval` → `StepRecord`). A one-line summary of each [`EvalStep`] (mirroring
/// the Rust `replay::render::summarize_step`) and the boundary markers that let the Events view group a
/// run's flat log by the step that produced each span. Only populated for eval runs; a live tail has no
/// journal, so the marker map is empty and the log renders unbroken.

import type { EvalStep } from "@zuihitsu/wire/types/EvalStep.ts";
import type { StepRecord } from "@zuihitsu/wire/types/StepRecord.ts";
import type { StepText } from "@zuihitsu/wire/types/StepText.ts";

/// How many characters of a step's text a summary keeps before clipping, matching the Rust summarizer.
const SUMMARY_CLIP = 60;

/// A boundary rendered above the first event of a step's span in the flat event stream. `genesis`
/// precedes the birth events that predate step zero; `resume` marks where a resumed run's live
/// continuation takes over from the restored recording; `step` names one scenario beat. Several may
/// share an anchor seq — buffered no-op steps (an `Advance` appends nothing) and the resume note both
/// flush before the next step that actually produced an event.
export type StepMarker =
  | { kind: "genesis" }
  | { kind: "resume" }
  | { kind: "step"; index: number; label: string; skipped: boolean };

/// The markers to render before each event, keyed by the seq of the event they sit above. A step that
/// appended events anchors at its `first_seq`; a no-op step (which appended none) buffers forward and
/// flushes before the next step that did, so its beat still reads in order. Trailing no-op steps with
/// no following event have nothing to anchor to and are dropped. An empty journal yields an empty map —
/// old packages and live tails render unbroken.
export function buildStepMarkers(
  journal: readonly StepRecord[],
  firstEventSeq: number | null,
  resumedFromStep: number | null,
): Map<number, StepMarker[]> {
  const markers = new Map<number, StepMarker[]>();
  if (journal.length === 0) return markers;

  const push = (seq: number, marker: StepMarker) => {
    const existing = markers.get(seq);
    if (existing) existing.push(marker);
    else markers.set(seq, [marker]);
  };

  // The birth events sit outside the journal, before the first step's span — mark them once, at the
  // top, only when there truly are events preceding that first span.
  const firstSpanStart = journal.reduce<number | null>((min, record) => {
    if (record.first_seq === null) return min;
    return min === null ? record.first_seq : Math.min(min, record.first_seq);
  }, null);
  if (firstEventSeq !== null && (firstSpanStart === null || firstEventSeq < firstSpanStart)) {
    push(firstEventSeq, { kind: "genesis" });
  }

  let pending: StepMarker[] = [];
  for (const record of journal) {
    if (resumedFromStep !== null && record.index === resumedFromStep + 1) {
      pending.push({ kind: "resume" });
    }
    pending.push({
      kind: "step",
      index: record.index,
      label: summarizeStep(record.step),
      skipped: record.skipped,
    });
    if (record.first_seq !== null) {
      for (const marker of pending) push(record.first_seq, marker);
      pending = [];
    }
  }
  return markers;
}

/// A one-line summary of a step, mirroring `replay::render::summarize_step` — the variant with its
/// load-bearing arguments, clipping long text.
export function summarizeStep(step: EvalStep): string {
  if (step === "settle") return "Settle";
  if (step === "describe_catch_up") return "DescribeCatchUp";
  if (step === "link_inference_catch_up") return "LinkInferenceCatchUp";
  if (step === "checkpoint_sweep") return "CheckpointSweep";

  if ("turn" in step) {
    const turn = step.turn;
    return `Turn ${turn.platform}/${turn.scope} ${turn.sender}: ${summarizeText(turn.text)}`;
  }
  if ("interrupted_turn" in step) {
    const burst = step.interrupted_turn;
    return `InterruptedTurn ${burst.platform}/${burst.scope} ${burst.first.sender}: ${summarizeText(burst.first.text)} | interrupt ${burst.interrupt.sender}: ${summarizeText(burst.interrupt.text)}`;
  }
  if ("imprint" in step) return `Imprint: "${clip(step.imprint.text)}"`;
  if ("advance" in step) return `Advance ${humaneDuration(step.advance.millis)}`;
  if ("seed_events" in step) return `SeedEvents (×${step.seed_events.length})`;
  if ("tune_supersession" in step) {
    return `TuneSupersession window_seconds=${step.tune_supersession.window_seconds}`;
  }
  if ("tighten_compaction" in step) {
    const { token_budget, flush_min_turns } = step.tighten_compaction;
    return `TightenCompaction budget=${token_budget} flush_min_turns=${flush_min_turns}`;
  }
  if ("force_compaction" in step) {
    const { platform, scope } = step.force_compaction;
    return `ForceCompaction ${platform}/${scope}`;
  }
  if ("tune_checkpoint" in step) {
    const { min_delta_chars, cooldown_seconds } = step.tune_checkpoint;
    return `TuneCheckpoint min_delta_chars=${min_delta_chars} cooldown_seconds=${cooldown_seconds}`;
  }
  // The remaining variant: confirm_proposed_merge.
  return `ConfirmProposedMerge (on_missing: ${step.confirm_proposed_merge.on_missing})`;
}

/// A step's text as a compact quoted fragment. A `with_turn_ref` shows its template and the anchor turn
/// it references, since the resolved `[turn:<id>]` token is only known at execution.
function summarizeText(text: StepText): string {
  if ("literal" in text) return `"${clip(text.literal)}"`;
  return `"${clip(text.with_turn_ref.template)}" (ref: "${clip(text.with_turn_ref.of_turn)}")`;
}

/// A duration in milliseconds as a compact humane string — the two most significant units, e.g. `5d`,
/// `3d 4h`, `4h 30m`, `2m10s`, `10s`. Mirrors the Rust `humane_duration` so a scripted `Advance` reads
/// the same in the console as in the replay CLI.
function humaneDuration(millis: number): string {
  const negative = millis < 0;
  const totalSecs = Math.floor(Math.abs(millis) / 1000);
  const days = Math.floor(totalSecs / 86_400);
  const hours = Math.floor((totalSecs % 86_400) / 3600);
  const minutes = Math.floor((totalSecs % 3600) / 60);
  const seconds = totalSecs % 60;

  let magnitude: string;
  if (days > 0) magnitude = twoUnits(days, "d", hours, "h", " ");
  else if (hours > 0) magnitude = twoUnits(hours, "h", minutes, "m", " ");
  else if (minutes > 0) magnitude = twoUnits(minutes, "m", seconds, "s", "");
  else magnitude = `${seconds}s`;
  return negative ? `-${magnitude}` : magnitude;
}

/// The larger unit always, the smaller only when non-zero — `3d 4h` but `5d`, joined by `sep`.
function twoUnits(
  major: number,
  majorUnit: string,
  minor: number,
  minorUnit: string,
  sep: string,
): string {
  return minor === 0 ? `${major}${majorUnit}` : `${major}${majorUnit}${sep}${minor}${minorUnit}`;
}

/// Clip text to the summary width, appending an ellipsis when it overruns.
function clip(text: string): string {
  return text.length > SUMMARY_CLIP ? `${text.slice(0, SUMMARY_CLIP)}…` : text;
}
