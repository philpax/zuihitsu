//! Resuming an interrupted run from its `.jsonl` sidecar, and healing the runs an infrastructure
//! outage poisoned.
//!
//! A `--resume` continues an interrupted suite from its sidecar, driving only the runs it does not
//! already hold. `--retry-infra-failed` extends that: a completed run bearing the
//! infrastructure-failure signature ([`infra_failed`]) is treated as not-done — re-driven live, its
//! poisoned record superseded rather than kept. The signature is structural: the model backend was
//! unreachable for the run's whole life, so every turn it drove deferred and no reply was produced.
//! An oracle-failed run — one whose turns all ran but whose verdicts missed — is legitimate data and
//! is **never** retried; redoing it would silently bias the suite's rates toward passing.
//!
//! Supersession needs no new sidecar record kind. The sidecar is an append-only log of whole-run
//! records keyed by `(scenario, run)`; a re-driven run appends a fresh `RunCompleted` under the same
//! key, and [`read_sidecar`] resolves duplicates last-write-wins, so the newest record is
//! authoritative. An un-healed sidecar holds exactly one record per key, so this reads identically to
//! before — old sidecars load unchanged.

use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use crate::{
    error::EvalError,
    live::LiveEvent,
    package::{RunMeta, RunRecord, ScenarioMeta},
};

/// An interrupted run folded from its sidecar: the manifest it began with, and the runs that finished.
pub struct ResumeState {
    pub meta: RunMeta,
    pub scenarios: Vec<ScenarioMeta>,
    /// `(scenario index, the completed run)`, one entry per `(scenario, run)` after last-write-wins
    /// deduplication, ordered by that key.
    pub completed: Vec<(u32, RunRecord)>,
}

/// Fold a `.jsonl` sidecar from an interrupted run into its [`ResumeState`]. Only runs with a
/// `RunCompleted` count as done; a run with a `RunStarted` (and perhaps some `RunEvent`s) but no
/// completion died mid-flight, so its partial events are dropped and it re-drives clean.
///
/// Duplicate `RunCompleted`s for the same `(scenario, run)` resolve last-write-wins: a healed run
/// appends a fresh record after the poisoned one it supersedes, and the later record is the one kept.
/// An un-healed sidecar carries one record per key, so this is a no-op there.
pub fn read_sidecar(path: &Path) -> Result<ResumeState, EvalError> {
    let file = File::open(path).map_err(|source| EvalError::WriteOutput {
        path: path.to_path_buf(),
        source,
    })?;
    let mut meta: Option<RunMeta> = None;
    let mut scenarios = Vec::new();
    // Keyed by `(scenario, run index)` so a re-driven run's fresh record overwrites the one it
    // supersedes; the `BTreeMap` also yields a deterministic `(scenario, run)` order.
    let mut completed: BTreeMap<(u32, u32), RunRecord> = BTreeMap::new();
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
            // The sidecar holds only whole runs: `RunCompleted` carries the full record, so a resume
            // reads it straight back. A run with a `RunStarted` but no completion died mid-flight and
            // re-drives clean. `RunEvent`s are broadcast-only and never reach the sidecar.
            LiveEvent::RunCompleted {
                scenario, record, ..
            } => {
                completed.insert((scenario, record.index), record);
            }
            LiveEvent::RunStarted { .. }
            | LiveEvent::RunEvent { .. }
            | LiveEvent::RunProgress { .. }
            | LiveEvent::Finished { .. } => {}
        }
    }
    let meta = meta.ok_or_else(|| EvalError::ResumeSidecar {
        path: path.to_path_buf(),
        reason: "no manifest line".to_owned(),
    })?;
    let completed = completed
        .into_iter()
        .map(|((scenario, _), record)| (scenario, record))
        .collect();
    Ok(ResumeState {
        meta,
        scenarios,
        completed,
    })
}

/// Whether a completed run's record bears the infrastructure-failure signature: the model backend was
/// unreachable for the run's whole life, so every turn it drove deferred — no deliberation step ran
/// and no reply was produced. The eval turn path returns such a turn as a completed run (the deferral
/// is `Ok`, the inbound is durably recorded), so the poison hides among genuine results and must be
/// told apart structurally.
///
/// The rule reads the recorded journal and metrics, never prose or an error string:
///
/// - the journal drove at least one model-invoking step ([`EvalStep::drives_model`](crate::step::EvalStep::drives_model)
///   — a `Turn` or an `Imprint`), **and**
/// - the run recorded zero `ModelCalled` events (`metrics.model_calls == 0`).
///
/// A single successful `ModelCalled` proves the endpoint was reachable for part of the run, so the
/// run's outcome reflects the model rather than the infrastructure — it is legitimate data and is
/// never flagged. The detector is therefore conservative: it flags only a run it can prove reached no
/// model at all, and so never retries a `MaxStepsExceeded`, a `Silent`, or an oracle miss (each of
/// those records model calls). A scenario that legitimately never calls the model (it only seeds
/// events and runs catch-up passes) drives no model-invoking step, so the journal clause spares it,
/// and a pre-journal record (an older package with an empty journal) carries no evidence a turn was
/// driven and is likewise never flagged.
pub fn infra_failed(record: &RunRecord) -> bool {
    record.metrics.model_calls == 0 && record.journal.iter().any(|step| step.step.drives_model())
}

/// Remove the [`infra_failed`] runs from a resumed sidecar's completed set, returning their
/// `(scenario index, run index)` coordinates so the caller can re-drive and report them. The kept
/// runs stay in `completed` to seed the package verbatim; the removed ones fall out of the run-skip
/// set the harness derives, so exactly those re-drive against the live model. Their fresh
/// `RunCompleted` records then supersede the poisoned ones in the sidecar (see [`read_sidecar`]).
pub fn take_infra_failed(completed: &mut Vec<(u32, RunRecord)>) -> Vec<(u32, u32)> {
    let mut retried = Vec::new();
    completed.retain(|(scenario, record)| {
        let poisoned = infra_failed(record);
        if poisoned {
            retried.push((*scenario, record.index));
        }
        !poisoned
    });
    retried
}

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use zuihitsu::{Completion, FlakyModel, InstanceFeatures, ModelClient, ScriptedModel, Seq};

    use super::{infra_failed, read_sidecar, take_infra_failed};
    use crate::{
        context::{RunContext, RunDeps},
        executor::{StepRecord, execute},
        harness,
        live::EvalSink,
        package::{Bar, Category, RunMeta, RunMetrics, RunRecord, ScenarioMeta, Verdict},
        step::{EvalStep, Turn},
    };

    /// A unique temp directory for a test that touches the filesystem (a sidecar round-trip).
    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zuihitsu-eval-heal-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn meta() -> RunMeta {
        RunMeta {
            harness_version: "test".to_owned(),
            git_sha: None,
            git_dirty: false,
            model_id: "test-model".to_owned(),
            embedding_model: None,
            scenario_filter: None,
            started_at_ms: 100,
            finished_at_ms: 100,
            runs_per_scenario: 2,
            concurrency: 1,
            rejudged_from: None,
            resumed_from: None,
        }
    }

    fn scenario_meta(name: &str) -> ScenarioMeta {
        ScenarioMeta {
            name: name.to_owned(),
            category: Category::Recall,
            description: "heal test".to_owned(),
            bar: Bar::gating(),
        }
    }

    /// A synthetic record with the given journal steps, model-call count, and verdicts — enough to
    /// exercise the structural detector without driving a model.
    fn record(
        index: u32,
        journal: Vec<EvalStep>,
        model_calls: u32,
        verdicts: Vec<Verdict>,
    ) -> RunRecord {
        RunRecord {
            index,
            started_at_ms: 0,
            finished_at_ms: 0,
            events: Vec::new(),
            journal: journal
                .into_iter()
                .enumerate()
                .map(|(offset, step)| StepRecord {
                    index: offset as u32,
                    step,
                    first_seq: None,
                    last_seq: None,
                    seq_after: Seq::ZERO,
                    skipped: false,
                })
                .collect(),
            verdicts,
            metrics: RunMetrics {
                model_calls,
                ..RunMetrics::default()
            },
        }
    }

    fn a_turn() -> EvalStep {
        Turn::new("discord", "team", "dave", "A fact to keep.").into()
    }

    /// Drive one participant turn against `model` and fold it into a record exactly as the harness
    /// does — so the detector is tested against the real event/journal shape a run produces, not a
    /// hand-built approximation.
    async fn drive_one_turn(model: Arc<dyn ModelClient>) -> RunRecord {
        let deps = RunDeps {
            model,
            embedder: None,
            dimensions: 0,
            web: crate::fetch_fixture::web_fetcher(),
        };
        let ctx = RunContext::new(&deps, InstanceFeatures::default())
            .await
            .expect("a fresh agent boots");
        let steps = vec![a_turn()];
        let journal = execute(&steps, &ctx).await.expect("the turn drives");
        let events = ctx.events().expect("the run's log");
        let metrics = harness::run_metrics(&events, true, 0);
        RunRecord {
            index: 0,
            started_at_ms: 0,
            finished_at_ms: 0,
            events,
            journal,
            verdicts: Vec::new(),
            metrics,
        }
    }

    /// A turn deferred by an unreachable backend produces the infra-failure signature: a journaled
    /// `Turn` step, but zero `ModelCalled` events (the model was never reached). This ties the
    /// detector to the exact record a real outage writes.
    #[tokio::test]
    async fn a_deferred_turn_bears_the_infra_signature() {
        let record = drive_one_turn(Arc::new(FlakyModel::always_transient())).await;
        assert_eq!(record.metrics.model_calls, 0, "the model was never reached");
        assert!(
            infra_failed(&record),
            "a run whose only turn deferred is infra-failed"
        );
    }

    /// A turn that reached the model records a `ModelCalled` and so is never flagged — its outcome is
    /// the model's, not the infrastructure's.
    #[tokio::test]
    async fn a_run_that_reached_the_model_is_not_infra_failed() {
        let record = drive_one_turn(Arc::new(ScriptedModel::new([Completion::Reply(
            "Noted.".to_owned(),
        )])))
        .await;
        assert!(record.metrics.model_calls >= 1, "the model was reached");
        assert!(!infra_failed(&record));
    }

    /// An oracle-failed run — the model ran (calls recorded) but a verdict missed — is legitimate data
    /// and must never be flagged for retry, whatever its verdicts say.
    #[test]
    fn an_oracle_failed_run_is_never_retried() {
        let failed = record(
            0,
            vec![a_turn()],
            5,
            vec![Verdict::oracle("safety", false, "slipped", None)],
        );
        assert!(
            !infra_failed(&failed),
            "a run that reached the model is legitimate data even when a verdict missed"
        );
    }

    /// A scenario that legitimately never calls the model (only seeds events and runs catch-up passes)
    /// drives no model-invoking step, so zero `ModelCalled` events do not mark it infra-failed.
    #[test]
    fn a_model_free_scenario_is_not_infra_failed() {
        let seeded = record(0, vec![EvalStep::SeedEvents(Vec::new())], 0, Vec::new());
        assert!(!infra_failed(&seeded));
    }

    /// A pre-journal record (an older package with an empty journal) carries no evidence a turn was
    /// driven, so it is conservatively never flagged.
    #[test]
    fn a_pre_journal_record_is_not_infra_failed() {
        let old = record(0, Vec::new(), 0, Vec::new());
        assert!(!infra_failed(&old));
    }

    /// The partition removes only the poisoned runs and returns their coordinates; every legitimate
    /// run — oracle-failed, model-free, or pre-journal — is kept for the package verbatim.
    #[test]
    fn take_infra_failed_removes_only_the_poisoned() {
        let mut completed = vec![
            (0u32, record(0, vec![a_turn()], 7, Vec::new())), // healthy
            (
                0u32,
                record(
                    1,
                    vec![a_turn()],
                    5,
                    vec![Verdict::oracle("safety", false, "slipped", None)],
                ),
            ), // oracle-failed, kept
            (0u32, record(2, vec![a_turn()], 0, Vec::new())), // poisoned
            (
                1u32,
                record(0, vec![EvalStep::SeedEvents(Vec::new())], 0, Vec::new()),
            ), // model-free
        ];
        let retried = take_infra_failed(&mut completed);
        assert_eq!(retried, vec![(0, 2)], "only the poisoned run is taken");
        let kept: Vec<(u32, u32)> = completed
            .iter()
            .map(|(scenario, run)| (*scenario, run.index))
            .collect();
        assert_eq!(
            kept,
            vec![(0, 0), (0, 1), (1, 0)],
            "every legitimate run stays"
        );
    }

    /// The end-to-end heal: an interrupted sidecar carrying a poisoned run and a kept oracle-failed
    /// sibling is re-read, the poisoned run re-driven, and its fresh record supersedes the poisoned one
    /// — the package ends with one record per index, correct aggregate, and no duplicate.
    #[test]
    fn a_poisoned_run_is_healed_and_its_record_superseded() {
        let dir = temp_dir();
        let sidecar = dir.join("run.jsonl");

        // Phase one: a run that recorded a poisoned (0,0) and a legitimate oracle-failed (0,1).
        let sink =
            EvalSink::new(meta(), vec![scenario_meta("heal")], &sidecar).expect("sink opens");
        sink.run_finished(0, record(0, vec![a_turn()], 0, Vec::new()))
            .expect("poisoned run lands");
        sink.run_finished(
            0,
            record(
                1,
                vec![a_turn()],
                5,
                vec![Verdict::oracle("safety", false, "slipped", None)],
            ),
        )
        .expect("oracle-failed run lands");
        drop(sink);

        // The heal reads the sidecar, takes the poisoned run, and seeds only the kept ones.
        let mut state = read_sidecar(&sidecar).expect("sidecar reads");
        assert_eq!(state.completed.len(), 2, "both runs read, no duplicate");
        let retried = take_infra_failed(&mut state.completed);
        assert_eq!(retried, vec![(0, 0)], "the poisoned run is taken for retry");

        let sink = EvalSink::resume(state, &sidecar).expect("resume reopens the sidecar");
        // Only the kept oracle-failed run is done; the poisoned index re-drives.
        assert_eq!(
            sink.done_runs(),
            std::iter::once((0u32, 1u32)).collect(),
            "the poisoned run is not counted done"
        );

        // The redo lands under the same index with the model reached this time.
        sink.run_finished(0, record(0, vec![a_turn()], 6, Vec::new()))
            .expect("healed run lands");

        let package = sink.package();
        let runs = &package.scenarios[0].runs;
        assert_eq!(runs.len(), 2, "one record per index, no duplicate");
        assert_eq!(runs[0].index, 0);
        assert_eq!(runs[1].index, 1);
        assert_eq!(
            runs[0].metrics.model_calls, 6,
            "index 0 is the healed record"
        );
        assert_eq!(package.scenarios[0].aggregate.runs, 2);

        // Re-reading the sidecar resolves (0,0) to the healed record: last-write-wins supersession.
        let reread = read_sidecar(&sidecar).expect("sidecar re-reads");
        assert_eq!(reread.completed.len(), 2, "supersession, not duplication");
        let healed = reread
            .completed
            .iter()
            .find(|(scenario, run)| *scenario == 0 && run.index == 0)
            .expect("the (0,0) record");
        assert_eq!(healed.1.metrics.model_calls, 6, "the healed record won");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Healing a sidecar with no poisoned runs is a clean no-op: the partition takes nothing and the
    /// completed set is untouched, so a finished-but-healthy package resumes unchanged.
    #[test]
    fn healing_a_healthy_sidecar_is_a_no_op() {
        let mut completed = vec![
            (0u32, record(0, vec![a_turn()], 4, Vec::new())),
            (0u32, record(1, vec![a_turn()], 5, Vec::new())),
        ];
        let before = completed.len();
        let retried = take_infra_failed(&mut completed);
        assert!(retried.is_empty(), "nothing to heal");
        assert_eq!(completed.len(), before, "the completed set is untouched");
    }

    /// An old-format sidecar — a manifest and a pre-journal `RunCompleted` line without `journal` or
    /// `at_ms` — still loads, and its journal-less record is conservatively not infra-failed.
    #[test]
    fn an_old_format_sidecar_loads() {
        let dir = temp_dir();
        let sidecar = dir.join("old.jsonl");
        let manifest = serde_json::to_string(&crate::live::LiveEvent::Manifest {
            meta: meta(),
            scenarios: vec![scenario_meta("legacy")],
        })
        .unwrap();
        // A RunCompleted as an older harness wrote it: no `journal`, no `at_ms`, minimal metrics.
        let old_completed = r#"{"kind":"run_completed","scenario":0,"run":0,"record":{"index":0,"events":[],"verdicts":[],"metrics":{"model_calls":0,"steps":0,"wall_clock_ms":0,"total_latency_ms":0,"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"gating_passed":true}},"aggregate":{"runs":1,"rate":0.0,"gating_passed":true,"wall_clock_ms":{"p50":0.0,"p95":0.0,"mean":0.0},"latency_ms":{"p50":0.0,"p95":0.0,"mean":0.0},"tokens":{"prompt_mean":0.0,"completion_mean":0.0,"total_mean":0.0},"steps_mean":0.0}}"#;
        std::fs::write(&sidecar, format!("{manifest}\n{old_completed}\n")).unwrap();

        let state = read_sidecar(&sidecar).expect("an old-format sidecar loads");
        assert_eq!(state.completed.len(), 1);
        let (_, old_record) = &state.completed[0];
        assert!(old_record.journal.is_empty(), "the journal defaults empty");
        assert!(
            !infra_failed(old_record),
            "a journal-less record is never flagged"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
