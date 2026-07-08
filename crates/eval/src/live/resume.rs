//! Resuming an interrupted run from its `.jsonl` sidecar.

use std::{
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
            // The sidecar holds only whole runs: `RunCompleted` carries the full record, so a resume
            // reads it straight back. A run with a `RunStarted` but no completion died mid-flight and
            // re-drives clean. `RunEvent`s are broadcast-only and never reach the sidecar.
            LiveEvent::RunCompleted {
                scenario, record, ..
            } => completed.push((scenario, record)),
            LiveEvent::RunStarted { .. }
            | LiveEvent::RunEvent { .. }
            | LiveEvent::Finished { .. } => {}
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
