//! The live eval log: the event stream a run emits, with two faces over one [`LiveEvent`] type. The
//! durable face is a `.jsonl` sidecar of whole runs — `Manifest`, then per run a `RunStarted` and a
//! `RunCompleted` (which carries the run's full record), ended by a `Finished`; it resumes an
//! interrupted run and folds into the final package. The live face (with `--serve`) is the same stream
//! plus `RunEvent`s broadcast as a run deliberates — these animate the console's deep-dive but are
//! *not* persisted, because `RunCompleted` carries the authoritative copy, so a viewer who joins
//! mid-run still folds the exact same package the sidecar would.

mod helpers;
mod resume;
#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, HashSet},
    fs::{File, OpenOptions},
    io::BufWriter,
    path::Path,
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use ts_rs::TS;
use zuihitsu::{Event, progress::TurnProgress};

use crate::{
    error::EvalError,
    harness,
    package::{Aggregate, EvalPackage, RunMeta, RunRecord, ScenarioMeta, ScenarioReport},
};

pub use helpers::now_ms;
use helpers::{flush, write};
pub use resume::{ResumeState, read_sidecar, resume_state_from_package, take_infra_failed};

const BROADCAST_CAPACITY: usize = 8192;

/// One event in an eval run's live log. Emitted in order; the console folds the sequence into an
/// [`EvalPackage`], and the harness persists it as a `.jsonl` sidecar.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LiveEvent {
    /// The plan: the run's metadata and every scenario that will run, in order. First event; seeds the
    /// scoreboard with all scenarios before any has a result. `scenario` indices elsewhere point into
    /// this list.
    Manifest {
        meta: RunMeta,
        scenarios: Vec<ScenarioMeta>,
    },
    /// A run began. `at_ms` is the harness's wall-clock (epoch milliseconds) at the start — the real
    /// clock, for the viewer's live elapsed and projection. `#[serde(default)]` fills `0` for a
    /// pre-timing sidecar line.
    RunStarted {
        scenario: u32,
        run: u32,
        #[serde(default)]
        #[ts(type = "number")]
        at_ms: i64,
    },
    /// One domain event from a run's deliberation, streamed live as it happens. Broadcast-only — it
    /// drives the deep-dive's unfolding view, but is *not* written to the sidecar: the authoritative
    /// record arrives in `RunCompleted`, so a client that joined mid-run (and missed some of these) is
    /// still made whole.
    RunEvent {
        scenario: u32,
        run: u32,
        event: Event,
    },
    /// One in-flight generation fragment from a run's deliberation — the same [`TurnProgress`]
    /// frame the live agent console streams, multiplexed with which run produced it. Broadcast-only
    /// and deliberately not buffered for catch-up: a viewer joining mid-generation misses earlier
    /// tokens and simply picks up from the next frame, because the durable record (`ModelCalled`,
    /// or `ModelCallAborted` for a discarded attempt) arrives as a `RunEvent` regardless.
    RunProgress {
        scenario: u32,
        run: u32,
        frame: TurnProgress,
    },
    /// A run finished: its whole record (events, verdicts, metrics) and the scenario's aggregate
    /// recomputed over its runs so far. Authoritative — folding it reproduces the canonical package
    /// regardless of which live `RunEvent`s a client happened to see.
    RunCompleted {
        scenario: u32,
        run: u32,
        record: RunRecord,
        aggregate: Aggregate,
        /// The harness's wall-clock (epoch milliseconds) at completion — mirrors `record.finished_at_ms`
        /// so a viewer folding only the live stream has the finish clock without unpacking the record.
        /// `#[serde(default)]` fills `0` for a pre-timing sidecar line.
        #[serde(default)]
        #[ts(type = "number")]
        at_ms: i64,
    },
    /// The whole run completed.
    Finished {
        #[ts(type = "number")]
        finished_at_ms: i64,
    },
}

/// Accumulates a run's [`LiveEvent`]s into a growing [`EvalPackage`] while appending them to a `.jsonl`
/// sidecar. Shared across the concurrent run tasks; every method serializes behind the lock, so the
/// sidecar's lines and the in-memory package stay consistent. [`EvalSink::package`] yields the folded
/// package to write once the whole run completes.
pub struct EvalSink {
    inner: Mutex<Inner>,
    /// Each emitted event, tagged with its monotonic id, broadcast to live `serve` subscribers. The
    /// sender lives outside the lock; the id and the write live inside it, so a subscriber that joins
    /// while holding the lock (see [`EvalSink::subscribe`]) sees a consistent snapshot-then-deltas cut.
    events: broadcast::Sender<(u64, LiveEvent)>,
}

struct Inner {
    package: EvalPackage,
    writer: BufWriter<File>,
    next_id: u64,
    /// The runs currently driving, by `(scenario, run)` — retained only while a run is in flight
    /// (dropped when it finishes into the package), so a client that connects mid-run can be caught up
    /// on the deliberation so far rather than seeing it start partway through. Each carries its start
    /// wall-clock so the replayed `RunStarted` reproduces the real one.
    in_flight: BTreeMap<(u32, u32), InFlightRun>,
}

/// A run's live catch-up state: its start wall-clock (epoch milliseconds) and the deliberation events
/// seen so far, replayed to a client that joins mid-run.
struct InFlightRun {
    started_at_ms: i64,
    events: Vec<Event>,
}

impl EvalSink {
    /// Seed the package with every scenario (no runs yet) and write the `Manifest` line to `sidecar`.
    pub fn new(
        meta: RunMeta,
        scenarios: Vec<ScenarioMeta>,
        sidecar: &Path,
    ) -> Result<EvalSink, EvalError> {
        let file = File::create(sidecar).map_err(|source| EvalError::WriteOutput {
            path: sidecar.to_path_buf(),
            source,
        })?;
        let package = EvalPackage {
            meta: meta.clone(),
            scenarios: scenarios
                .iter()
                .cloned()
                .map(|meta| ScenarioReport {
                    meta,
                    runs: Vec::new(),
                    aggregate: harness::aggregate(&[]),
                })
                .collect(),
        };
        let (events, _) = broadcast::channel(BROADCAST_CAPACITY);
        let sink = EvalSink {
            inner: Mutex::new(Inner {
                package,
                writer: BufWriter::new(file),
                next_id: 0,
                in_flight: BTreeMap::new(),
            }),
            events,
        };
        sink.emit(LiveEvent::Manifest { meta, scenarios })?;
        Ok(sink)
    }

    /// Open a run: record that it is driving (an empty live buffer, stamped with its start wall-clock)
    /// and emit `RunStarted`.
    pub fn run_started(&self, scenario: u32, run: u32, at_ms: i64) -> Result<(), EvalError> {
        let mut inner = self.inner.lock();
        inner.in_flight.insert(
            (scenario, run),
            InFlightRun {
                started_at_ms: at_ms,
                events: Vec::new(),
            },
        );
        self.emit_locked(
            &mut inner,
            LiveEvent::RunStarted {
                scenario,
                run,
                at_ms,
            },
        )
    }

    /// Broadcast one `RunEvent` live — a single event from a run's deliberation as it is recorded.
    /// Retained in the run's live buffer (to catch up a late-joining client) but not written to the
    /// sidecar: the authoritative copy rides the run's `RunCompleted`, so a viewer loses nothing.
    pub fn run_event(&self, scenario: u32, run: u32, event: Event) -> Result<(), EvalError> {
        let mut inner = self.inner.lock();
        inner
            .in_flight
            .entry((scenario, run))
            .or_insert_with(|| InFlightRun {
                started_at_ms: 0,
                events: Vec::new(),
            })
            .events
            .push(event.clone());
        self.broadcast_locked(
            &mut inner,
            LiveEvent::RunEvent {
                scenario,
                run,
                event,
            },
        );
        Ok(())
    }

    /// Broadcast one in-flight generation fragment. Nothing is persisted and nothing is buffered:
    /// progress exists only for whoever is watching right now.
    pub fn run_progress(&self, scenario: u32, run: u32, frame: TurnProgress) {
        let mut inner = self.inner.lock();
        self.broadcast_locked(
            &mut inner,
            LiveEvent::RunProgress {
                scenario,
                run,
                frame,
            },
        );
    }

    /// Fold a finished run in: append its record to the scenario, recompute the scenario aggregate, and
    /// emit `RunCompleted` carrying the whole record. This is the authoritative copy a client folds —
    /// the live `RunEvent`s only animated the deliberation up to here.
    pub fn run_finished(&self, scenario: u32, record: RunRecord) -> Result<(), EvalError> {
        let mut inner = self.inner.lock();
        let run = record.index;
        // The run is whole now and lives in the package; retire its live catch-up buffer.
        inner.in_flight.remove(&(scenario, run));
        let at_ms = record.finished_at_ms;
        let report = &mut inner.package.scenarios[scenario as usize];
        report.runs.push(record.clone());
        report.runs.sort_by_key(|run| run.index);
        report.aggregate = harness::aggregate(&report.runs);
        let aggregate = report.aggregate;
        self.emit_locked(
            &mut inner,
            LiveEvent::RunCompleted {
                scenario,
                run,
                record,
                aggregate,
                at_ms,
            },
        )?;
        // Flush at the run boundary so a kill never loses a completed run: the sidecar always holds
        // whole runs, and resume re-drives only what genuinely did not finish.
        flush(&mut inner.writer)
    }

    /// Emit `Finished`, stamp the package, and flush the sidecar.
    pub fn finish(&self, finished_at_ms: i64) -> Result<(), EvalError> {
        let mut inner = self.inner.lock();
        inner.package.meta.finished_at_ms = finished_at_ms;
        self.emit_locked(&mut inner, LiveEvent::Finished { finished_at_ms })?;
        flush(&mut inner.writer)
    }

    /// Snapshot the current package, the in-flight runs' events so far, and subscribe to the deltas to
    /// come — all under the lock, so the cut is consistent: the snapshot and catch-up reflect exactly
    /// the events before the subscription, and the receiver gets exactly those after, with no gap or
    /// overlap. The catch-up is replayed as `RunStarted` + `RunEvent`s so a client joining mid-run sees
    /// the deliberation from its start. The basis for `serve`'s stream.
    pub fn subscribe(
        &self,
    ) -> (
        EvalPackage,
        Vec<LiveEvent>,
        broadcast::Receiver<(u64, LiveEvent)>,
    ) {
        let inner = self.inner.lock();
        let mut catch_up = Vec::new();
        for (&(scenario, run), in_flight) in &inner.in_flight {
            catch_up.push(LiveEvent::RunStarted {
                scenario,
                run,
                at_ms: in_flight.started_at_ms,
            });
            for event in &in_flight.events {
                catch_up.push(LiveEvent::RunEvent {
                    scenario,
                    run,
                    event: event.clone(),
                });
            }
        }
        let receiver = self.events.subscribe();
        (inner.package.clone(), catch_up, receiver)
    }

    /// Re-open an interrupted run's sidecar to continue it: seed the package with the runs that
    /// already completed (from [`read_sidecar`]) and append onward — the manifest and those runs are
    /// already on disk, so nothing is re-written, only continued.
    pub fn resume(state: ResumeState, sidecar: &Path) -> Result<EvalSink, EvalError> {
        let file = OpenOptions::new()
            .append(true)
            .open(sidecar)
            .map_err(|source| EvalError::WriteOutput {
                path: sidecar.to_path_buf(),
                source,
            })?;
        let mut package = EvalPackage {
            meta: state.meta,
            scenarios: state
                .scenarios
                .into_iter()
                .map(|meta| ScenarioReport {
                    meta,
                    runs: Vec::new(),
                    aggregate: harness::aggregate(&[]),
                })
                .collect(),
        };
        for (scenario, record) in state.completed {
            package.scenarios[scenario as usize].runs.push(record);
        }
        for report in &mut package.scenarios {
            report.runs.sort_by_key(|run| run.index);
            report.aggregate = harness::aggregate(&report.runs);
        }
        let (events, _) = broadcast::channel(BROADCAST_CAPACITY);
        Ok(EvalSink {
            inner: Mutex::new(Inner {
                package,
                writer: BufWriter::new(file),
                // Resumed deltas continue the id sequence past what the sidecar already holds.
                next_id: 0,
                in_flight: BTreeMap::new(),
            }),
            events,
        })
    }

    /// The `(scenario, run)` pairs already complete — what a resumed run skips so only the missing runs
    /// drive.
    pub fn done_runs(&self) -> HashSet<(u32, u32)> {
        let inner = self.inner.lock();
        inner
            .package
            .scenarios
            .iter()
            .enumerate()
            .flat_map(|(scenario, report)| {
                report
                    .runs
                    .iter()
                    .map(move |run| (scenario as u32, run.index))
            })
            .collect()
    }

    /// The folded package as it stands — every scenario with its runs and aggregate. Cloned (rather
    /// than consuming) so the sink lives on to keep serving the final state to viewers.
    pub fn package(&self) -> EvalPackage {
        self.inner.lock().package.clone()
    }

    fn emit(&self, event: LiveEvent) -> Result<(), EvalError> {
        let mut inner = self.inner.lock();
        self.emit_locked(&mut inner, event)
    }

    /// Write one event to the sidecar, then broadcast it — under the held lock, so the sidecar's line
    /// order and the broadcast order agree. For the durable events (manifest, run boundaries).
    fn emit_locked(&self, inner: &mut Inner, event: LiveEvent) -> Result<(), EvalError> {
        write(&mut inner.writer, &event)?;
        self.broadcast_locked(inner, event);
        Ok(())
    }

    /// Tag an event with the next id and broadcast it to live subscribers, without writing it to the
    /// sidecar — for the live-only `RunEvent`s. The id sequence is shared with [`EvalSink::emit_locked`]
    /// so every broadcast event is ordered. A send with no live subscribers is a no-op.
    fn broadcast_locked(&self, inner: &mut Inner, event: LiveEvent) {
        let id = inner.next_id;
        inner.next_id += 1;
        let _ = self.events.send((id, event));
    }
}
