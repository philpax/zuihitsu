//! The `events` command: inspect one recorded run's event log, grouped under the journal steps that
//! produced the events. Reading a run this way is how an operator picks the `--step K` to resume from —
//! each step line shows its event-seq span, and the step index is the resume coordinate.

use std::path::Path;

use zuihitsu::{Event, Seq};

use crate::{
    error::EvalError,
    executor::StepRecord,
    package::RunRecord,
    replay::{
        render::{event_summary, humane_offset, name_map, summarize_step},
        resolve_run, resolve_scenario,
    },
};

/// Print a run's events grouped by the journal step that produced them (or, for a run recorded before
/// step journaling, a flat listing with a note). `scenario` is required when the package holds more than
/// one scenario; `run` defaults to `0`; `truncate` clips each event's summary (`0` = full).
pub(crate) fn events(
    package: &Path,
    scenario: Option<&str>,
    run: usize,
    truncate: usize,
) -> Result<(), EvalError> {
    let pkg = crate::replay::load(package)?;
    let report = resolve_scenario(&pkg, scenario).map_err(EvalError::Events)?;
    let record = resolve_run(report, run).map_err(EvalError::Events)?;

    println!("package  {}", package.display());
    println!(
        "scenario {}  (run {} of {})",
        report.meta.name,
        run,
        report.runs.len(),
    );
    println!("verdict  {}", verdict_summary(record));

    if record.journal.is_empty() {
        print_flat(record, truncate);
        return Ok(());
    }
    print_grouped(record, truncate);
    println!(
        "\nresume from a step with: replay --mode resume --step <index> -s {} --run {} {}",
        report.meta.name,
        run,
        package.display(),
    );
    Ok(())
}

/// A one-line verdict summary: how many verdicts held, and whether the gating oracles all held.
fn verdict_summary(record: &RunRecord) -> String {
    let held = record.verdicts.iter().filter(|v| v.passed).count();
    let total = record.verdicts.len();
    let gating = if record.metrics.gating_passed {
        "gate ok"
    } else {
        "gate FAIL"
    };
    format!("{held}/{total} verdicts held, {gating}")
}

/// The flat listing for a journal-less run: every event in seq order, with a note that the run predates
/// step journaling, so `--mode resume` is unavailable while `--mode rejudge` still works.
fn print_flat(record: &RunRecord, truncate: usize) {
    println!(
        "\n(this run predates step journaling: `replay --mode resume` is unavailable; \
         `replay --mode rejudge` still works)\n"
    );
    let names = name_map(&record.events);
    let base = base_millis(&record.events);
    for event in &record.events {
        print_event(event, base, &names, truncate);
    }
}

/// The grouped listing: genesis events below the first step, then each step with its event span.
fn print_grouped(record: &RunRecord, truncate: usize) {
    let grouping = group_events(&record.events, &record.journal);
    let names = name_map(&record.events);
    let base = base_millis(&record.events);

    if !grouping.genesis.is_empty() {
        println!("\ngenesis   {}", span_label_from(&grouping.genesis));
        for event in &grouping.genesis {
            print_event(event, base, &names, truncate);
        }
    }
    for group in &grouping.steps {
        let skipped = if group.record.skipped {
            "  [skipped]"
        } else {
            ""
        };
        println!(
            "\nstep {}  {}   {}{skipped}",
            group.record.index,
            summarize_step(&group.record.step),
            span_label(group.record),
        );
        for event in &group.events {
            print_event(event, base, &names, truncate);
        }
    }
}

/// Print one event line: seq, offset from the run's first event, payload type, and compact summary.
fn print_event(event: &Event, base: i64, names: &crate::replay::render::NameMap, truncate: usize) {
    let offset = humane_offset(event.recorded_at.as_millisecond() - base);
    println!(
        "  {:>5}  {:<8}  {:<24}  {}",
        event.seq.0,
        offset,
        crate::event_render::payload_type(&event.payload),
        event_summary(&event.payload, names, truncate),
    );
}

/// The seq span label for a journal step (`[seq 14–29]`, or `[no events]` for an empty span).
fn span_label(record: &StepRecord) -> String {
    match (record.first_seq, record.last_seq) {
        (Some(first), Some(last)) => format!("[seq {}–{}]", first.0, last.0),
        _ => "[no events]".to_owned(),
    }
}

/// The seq span label for the genesis group, from its collected events.
fn span_label_from(events: &[&Event]) -> String {
    match (events.first(), events.last()) {
        (Some(first), Some(last)) => format!("[seq {}–{}]", first.seq.0, last.seq.0),
        _ => "[no events]".to_owned(),
    }
}

/// The run's first event's `recorded_at` (millis), the anchor every event's offset is measured from.
fn base_millis(events: &[Event]) -> i64 {
    events
        .first()
        .map(|event| event.recorded_at.as_millisecond())
        .unwrap_or(0)
}

/// A run's events bucketed into the genesis events (below the first journaled step's span) and the
/// per-step events. Genesis events predate the journal — the birth events the executor left below step
/// zero — so they group separately; every other event falls in exactly one step's contiguous span.
pub(crate) struct Grouping<'a> {
    pub(crate) genesis: Vec<&'a Event>,
    pub(crate) steps: Vec<StepGroup<'a>>,
}

/// One journal step and the events its span covers, in seq order.
pub(crate) struct StepGroup<'a> {
    pub(crate) record: &'a StepRecord,
    pub(crate) events: Vec<&'a Event>,
}

/// Bucket a run's events under their journal steps. An event whose seq falls below the first step's span
/// (or all events, when no step appended any) is genesis; otherwise it belongs to the step whose
/// contiguous span contains its seq. A pure function over the recorded data, so it is tested directly.
pub(crate) fn group_events<'a>(events: &'a [Event], journal: &'a [StepRecord]) -> Grouping<'a> {
    let first_covered = journal
        .iter()
        .filter_map(|record| record.first_seq)
        .map(|seq| seq.0)
        .min();

    let mut genesis = Vec::new();
    let mut step_events: Vec<Vec<&Event>> = vec![Vec::new(); journal.len()];
    for event in events {
        let seq = event.seq.0;
        if first_covered.is_none_or(|first| seq < first) {
            genesis.push(event);
            continue;
        }
        match journal.iter().position(|record| within(record, event.seq)) {
            Some(index) => step_events[index].push(event),
            // A seq at or past the first span but in no step's span is a gap that the contiguous journal
            // does not produce; group it with genesis rather than dropping it.
            None => genesis.push(event),
        }
    }

    let steps = journal
        .iter()
        .zip(step_events)
        .map(|(record, events)| StepGroup { record, events })
        .collect();
    Grouping { genesis, steps }
}

/// Whether a step's span covers `seq`.
fn within(record: &StepRecord, seq: Seq) -> bool {
    match (record.first_seq, record.last_seq) {
        (Some(first), Some(last)) => first <= seq && seq <= last,
        _ => false,
    }
}
