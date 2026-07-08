//! Low-level sidecar helpers: stamping, serializing, and flushing.

use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{error::EvalError, live::LiveEvent};

/// The harness's wall-clock as epoch milliseconds — the real clock that stamps run start and finish
/// (never the scenario's simulated clock). Falls back to `0` if the system clock predates the epoch.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or(0)
}

/// Serialize one event as a single JSON line. The sidecar shares the `.jsonl` convention of the
/// tracked history; each line is one self-contained [`LiveEvent`].
pub(super) fn write(writer: &mut BufWriter<File>, event: &LiveEvent) -> Result<(), EvalError> {
    let line = serde_json::to_string(event)?;
    writeln!(writer, "{line}").map_err(|source| EvalError::WriteOutput {
        path: Path::new("<eval sidecar>").to_path_buf(),
        source,
    })
}

/// Flush the buffered sidecar to disk — at a run boundary, so durability is per completed run.
pub(super) fn flush(writer: &mut BufWriter<File>) -> Result<(), EvalError> {
    writer.flush().map_err(|source| EvalError::WriteOutput {
        path: Path::new("<eval sidecar>").to_path_buf(),
        source,
    })
}
