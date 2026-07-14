//! The step journal record: one step's event-log coverage, journaled by the executor.

use serde::{Deserialize, Serialize};
use zuihitsu_core::ids::Seq;

use crate::step::EvalStep;

/// One step's event-log coverage. The span (`first_seq`..=`last_seq`) is the events the step appended,
/// and `seq_after` is the log head after it — the watermark phase two restores the store up to when it
/// resumes from a step. The spans are contiguous and non-overlapping across the journal, so every
/// event belongs to exactly one step; `seq_after` is monotone non-decreasing, unchanged by a step that
/// appended nothing, and equal to the log head after the final step.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StepRecord {
    pub index: u32,
    pub step: EvalStep,
    /// The seq of the first event the step appended, or `None` if it appended none (`Advance` only
    /// moves the clock; a skipped `ConfirmProposedMerge` performs nothing).
    pub first_seq: Option<Seq>,
    /// The seq of the last event the step appended, or `None` if it appended none.
    pub last_seq: Option<Seq>,
    /// The log head after this step — the restore watermark for resuming at step K.
    pub seq_after: Seq,
    /// Whether the step performed no operation because a run-time precondition was absent (a
    /// `ConfirmProposedMerge` with `on_missing: Skip` and no proposal in the log).
    pub skipped: bool,
}
