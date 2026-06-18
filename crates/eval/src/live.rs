//! The live eval log: the event stream a run emits, persisted as a `.jsonl` sidecar and (with
//! `--serve`) broadcast to the console, which folds it into a growing [`EvalPackage`]. One log with two
//! faces — the durable sidecar resumes an interrupted run and folds into the final package; the stream
//! drives the live view. The log of a whole run is a `Manifest`, then per run a `RunStarted`, its
//! `RunEvent`s, and a `RunCompleted`, ended by a `Finished`.

use std::{
    collections::{BTreeMap, HashSet},
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
    sync::Mutex,
};

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use zuihitsu::Event;

use crate::{
    error::EvalError,
    harness,
    package::{
        Aggregate, EvalPackage, RunMeta, RunMetrics, RunRecord, ScenarioMeta, ScenarioReport,
        Verdict,
    },
};

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
    /// A run began.
    RunStarted { scenario: u32, run: u32 },
    /// One domain event from a run's deliberation, streamed as it happens.
    RunEvent {
        scenario: u32,
        run: u32,
        event: Event,
    },
    /// A run finished: its verdicts and metrics, and the scenario's aggregate recomputed over its runs
    /// so far. The run's events arrived as `RunEvent`s.
    RunCompleted {
        scenario: u32,
        run: u32,
        verdicts: Vec<Verdict>,
        metrics: RunMetrics,
        aggregate: Aggregate,
    },
    /// The whole run completed.
    Finished {
        #[ts(type = "number")]
        finished_at_ms: i64,
    },
}

/// Accumulates a run's [`LiveEvent`]s into a growing [`EvalPackage`] while appending them to a `.jsonl`
/// sidecar. Shared across the concurrent run tasks; every method serializes behind the lock, so the
/// sidecar's lines and the in-memory package stay consistent. [`EvalSink::into_package`] yields the
/// final package to fold and write once the whole run completes.
pub struct EvalSink {
    inner: Mutex<Inner>,
}

struct Inner {
    package: EvalPackage,
    writer: BufWriter<File>,
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
        let sink = EvalSink {
            inner: Mutex::new(Inner {
                package,
                writer: BufWriter::new(file),
            }),
        };
        sink.emit(LiveEvent::Manifest { meta, scenarios })?;
        Ok(sink)
    }

    /// Emit `RunStarted` — a run has begun driving.
    pub fn run_started(&self, scenario: u32, run: u32) -> Result<(), EvalError> {
        self.emit(LiveEvent::RunStarted { scenario, run })
    }

    /// Fold a finished run in: stream its events as `RunEvent`s, append the record to its scenario,
    /// recompute the scenario aggregate, and emit the light `RunCompleted`.
    pub fn run_completed(&self, scenario: u32, record: RunRecord) -> Result<(), EvalError> {
        let mut inner = self.inner.lock().expect("eval sink poisoned");
        let run = record.index;
        for event in &record.events {
            write(
                &mut inner.writer,
                &LiveEvent::RunEvent {
                    scenario,
                    run,
                    event: event.clone(),
                },
            )?;
        }
        let verdicts = record.verdicts.clone();
        let metrics = record.metrics;
        let report = &mut inner.package.scenarios[scenario as usize];
        report.runs.push(record);
        report.runs.sort_by_key(|run| run.index);
        report.aggregate = harness::aggregate(&report.runs);
        let aggregate = report.aggregate;
        write(
            &mut inner.writer,
            &LiveEvent::RunCompleted {
                scenario,
                run,
                verdicts,
                metrics,
                aggregate,
            },
        )?;
        // Flush at the run boundary so a kill never loses a completed run: the sidecar always holds
        // whole runs, and resume re-drives only what genuinely did not finish.
        inner
            .writer
            .flush()
            .map_err(|source| EvalError::WriteOutput {
                path: Path::new("<eval sidecar>").to_path_buf(),
                source,
            })
    }

    /// Emit `Finished`, stamp the package, and flush the sidecar.
    pub fn finish(&self, finished_at_ms: i64) -> Result<(), EvalError> {
        let mut inner = self.inner.lock().expect("eval sink poisoned");
        inner.package.meta.finished_at_ms = finished_at_ms;
        write(&mut inner.writer, &LiveEvent::Finished { finished_at_ms })?;
        inner
            .writer
            .flush()
            .map_err(|source| EvalError::WriteOutput {
                path: Path::new("<eval sidecar>").to_path_buf(),
                source,
            })
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
        Ok(EvalSink {
            inner: Mutex::new(Inner {
                package,
                writer: BufWriter::new(file),
            }),
        })
    }

    /// The `(scenario, run)` pairs already complete — what a resumed run skips so only the missing runs
    /// drive.
    pub fn done_runs(&self) -> HashSet<(u32, u32)> {
        let inner = self.inner.lock().expect("eval sink poisoned");
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

    /// The final folded package — every scenario with its runs and aggregate.
    pub fn into_package(self) -> EvalPackage {
        self.inner.into_inner().expect("eval sink poisoned").package
    }

    fn emit(&self, event: LiveEvent) -> Result<(), EvalError> {
        let mut inner = self.inner.lock().expect("eval sink poisoned");
        write(&mut inner.writer, &event)
    }
}

/// An interrupted run folded from its sidecar: the manifest it began with, and the runs that finished.
pub struct ResumeState {
    pub meta: RunMeta,
    pub scenarios: Vec<ScenarioMeta>,
    /// `(scenario index, the completed run)`.
    pub completed: Vec<(u32, RunRecord)>,
}

/// Fold a `.jsonl` sidecar from an interrupted run into its [`ResumeState`]. Only runs with a
/// `RunCompleted` count as done; a run with a `RunStarted` (and perhaps some `RunEvent`s) but no
/// completion died mid-flight, so its partial events are dropped and it re-drives clean.
pub fn read_sidecar(path: &Path) -> Result<ResumeState, EvalError> {
    let file = File::open(path).map_err(|source| EvalError::WriteOutput {
        path: path.to_path_buf(),
        source,
    })?;
    let mut meta: Option<RunMeta> = None;
    let mut scenarios = Vec::new();
    let mut pending: BTreeMap<(u32, u32), Vec<Event>> = BTreeMap::new();
    let mut completed = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|source| EvalError::WriteOutput {
            path: path.to_path_buf(),
            source,
        })?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<LiveEvent>(&line)? {
            LiveEvent::Manifest {
                meta: run_meta,
                scenarios: metas,
            } => {
                meta = Some(run_meta);
                scenarios = metas;
            }
            LiveEvent::RunEvent {
                scenario,
                run,
                event,
            } => pending.entry((scenario, run)).or_default().push(event),
            LiveEvent::RunCompleted {
                scenario,
                run,
                verdicts,
                metrics,
                ..
            } => {
                let events = pending.remove(&(scenario, run)).unwrap_or_default();
                completed.push((
                    scenario,
                    RunRecord {
                        index: run,
                        events,
                        verdicts,
                        metrics,
                    },
                ));
            }
            LiveEvent::RunStarted { .. } | LiveEvent::Finished { .. } => {}
        }
    }
    let meta = meta.ok_or_else(|| EvalError::ResumeSidecar {
        path: path.to_path_buf(),
        reason: "no manifest line".to_owned(),
    })?;
    Ok(ResumeState {
        meta,
        scenarios,
        completed,
    })
}

/// Serialize one event as a single JSON line. The sidecar shares the `.jsonl` convention of the
/// tracked history; each line is one self-contained [`LiveEvent`].
fn write(writer: &mut BufWriter<File>, event: &LiveEvent) -> Result<(), EvalError> {
    let line = serde_json::to_string(event)?;
    writeln!(writer, "{line}").map_err(|source| EvalError::WriteOutput {
        path: Path::new("<eval sidecar>").to_path_buf(),
        source,
    })
}
